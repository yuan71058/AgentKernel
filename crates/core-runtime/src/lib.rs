//! # Core Runtime
//!
//! AgentKernel 的顶层运行时入口，串联所有子 crate。
//!
//! 用法：
//! ```rust,no_run
//! use core_runtime::AgentKernel;
//! use core_protocol::*;
//!
//! # async fn example() -> Result<(), core_runtime::RuntimeErrorReport> {
//! let kernel = AgentKernel::new(ProviderConfig {
//!     protocol: Protocol::OpenAI,
//!     base_url: "https://api.deepseek.com".into(),
//!     api_key: "sk-...".into(),
//!     model: "deepseek-chat".into(),
//!     ..Default::default()
//! });
//!
//! // 注册工具
//! kernel.register_tool(
//!     "",
//!     Tool { name: "calc".into(), description: "计算器".into(), input_schema: serde_json::json!({}), compiled_schemas: Default::default() },
//!     ToolRegistration {
//!         tool_name: "calc".into(),
//!         description: "计算器".into(),
//!         client_id: "local".into(),
//!         permissions: vec![],
//!         timeout_ms: 0,
//!         tags: vec![],
//!     },
//! );
//!
//! // 对话
//! let resp = kernel.chat("sess_1", "你好").await?;
//! println!("{}", resp.content);
//! # Ok(())
//! # }
//! ```

use core_context::ContextManager;
use core_events::EventBus;
use core_events::event_types::*;
use core_model::{Router, CallTrace, analyze_tool_chain, should_retry};
use core_protocol::*;
use core_session::SessionManager;
use core_storage::{Storage, MemoryStorage};
use core_tool::ToolManager;
use core_trace::TraceCollector;
use std::collections::HashMap;
use std::any::Any;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::{broadcast, Notify};
use futures::FutureExt;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SessionToolSnapshotItem {
    tool: Tool,
    #[serde(default)]
    registration: Option<ToolRegistration>,
}

const DEFAULT_REPEATED_TOOL_CALL_LIMIT: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolCallFingerprint {
    tool_name: String,
    input_json: String,
}

#[derive(Debug, Clone, Default)]
struct RepeatedToolCallGuard {
    last_fingerprint: Option<ToolCallFingerprint>,
    consecutive_count: u32,
}

impl RepeatedToolCallGuard {
    fn observe(&mut self, tool_name: &str, input: &serde_json::Value) -> u32 {
        let fingerprint = ToolCallFingerprint {
            tool_name: tool_name.to_string(),
            input_json: canonicalize_json(input),
        };
        if self.last_fingerprint.as_ref() == Some(&fingerprint) {
            self.consecutive_count += 1;
        } else {
            self.last_fingerprint = Some(fingerprint);
            self.consecutive_count = 1;
        }
        self.consecutive_count
    }

    fn last(&self) -> Option<&ToolCallFingerprint> {
        self.last_fingerprint.as_ref()
    }
}

fn canonicalize_json(value: &serde_json::Value) -> String {
    fn normalize(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(normalize).collect())
            }
            serde_json::Value::Object(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by(|(left, _), (right, _)| left.cmp(right));
                let mut normalized = serde_json::Map::new();
                for (key, value) in entries {
                    normalized.insert(key.clone(), normalize(value));
                }
                serde_json::Value::Object(normalized)
            }
            _ => value.clone(),
        }
    }

    serde_json::to_string(&normalize(value)).unwrap_or_else(|_| "null".to_string())
}

/// 对话选项
#[derive(Debug, Clone)]
pub struct ChatOptions {
    pub session_id: String,
    pub run_id: String,
    pub message: String,
    pub images: Vec<String>,
    pub audio: Vec<AudioInput>,
    pub max_repeated_tool_calls: u32,
    pub append_user_message: bool,
}

/// 音频输入
#[derive(Debug, Clone)]
pub struct AudioInput {
    pub data: String,
    pub format: String,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            run_id: String::new(),
            message: String::new(),
            images: Vec::new(),
            audio: Vec::new(),
            max_repeated_tool_calls: DEFAULT_REPEATED_TOOL_CALL_LIMIT,
            append_user_message: true,
        }
    }
}

/// 对话响应
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub session_id: String,
    pub run_id: String,
    pub content: String,
    pub status: String,
    pub partial_preserved: bool,
    pub usage: Usage,
    pub traces: Vec<CallTrace>,
    pub tool_calls_made: u32,
}

/// 运行时错误报告
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuntimeErrorReport {
    pub source: String,
    pub stage: String,
    pub retryable: bool,
    pub message: String,
}

impl RuntimeErrorReport {
    pub fn new(
        source: impl Into<String>,
        stage: impl Into<String>,
        retryable: bool,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            stage: stage.into(),
            retryable,
            message: message.into(),
        }
    }

    pub fn provider(stage: impl Into<String>, raw_error: impl Into<String>) -> Self {
        let raw_error = raw_error.into();
        let retryable = should_retry(&raw_error);
        Self::new("provider", stage, retryable, raw_error)
    }

    pub fn provider_retryable(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new("provider", stage, true, message.into())
    }

    pub fn core(stage: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new("core", stage, false, message)
    }

    pub fn validation(stage: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new("core_validation", stage, false, message)
    }

    pub fn to_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "error": self.message,
            "source": self.source,
            "stage": self.stage,
            "retryable": self.retryable,
        })
    }
}

impl fmt::Display for RuntimeErrorReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}:{} retryable={}] {}",
            self.source, self.stage, self.retryable, self.message
        )
    }
}

impl From<String> for RuntimeErrorReport {
    fn from(value: String) -> Self {
        RuntimeErrorReport::core("runtime", value)
    }
}
pub type Scaffold = AgentKernel;

/// 工具路由器 trait
///
/// 实现此 trait 可将工具调用路由到外部客户端（如 WS 前端）执行。
/// Core 不做业务编排，只负责状态与调度，工具执行在业务端。
#[async_trait::async_trait]
pub trait ToolRouter: Send + Sync + 'static {
    /// 路由工具调用到外部客户端并等待结果
    ///
    /// 返回 Ok((result_content, is_error)) 或 Err(error_message)
    async fn execute(
        &self,
        session_id: &str,
        run_id: &str,
        tool_name: &str,
        call_id: &str,
        input: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<(String, bool), String>;

    /// 取消某个 run 下等待中的工具调用。
    async fn cancel_run(&self, _run_id: &str) -> Result<(), String> {
        Ok(())
    }

    /// 支持 downcast（用于 WS 层解析工具结果）
    fn as_any(&self) -> &dyn Any;
}

struct RunControl {
    cancelled: AtomicBool,
    notify: Notify,
}

impl RunControl {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

struct ActiveRunEntry {
    session_id: String,
    started_at: Instant,
    control: Arc<RunControl>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ActiveRunSnapshot {
    pub session_id: String,
    pub run_id: String,
    pub duration_ms: u64,
    pub status: String,
}

struct RunCleanup {
    active_runs: Arc<RwLock<HashMap<String, ActiveRunEntry>>>,
    run_id: String,
}

impl Drop for RunCleanup {
    fn drop(&mut self) {
        self.active_runs.write().unwrap().remove(&self.run_id);
    }
}

/// AgentKernel 运行时核心
pub struct AgentKernel {
    pub config: ProviderConfig,
    pub system_prompt: RwLock<String>,
    pub provider_router: Router,
    pub tool_manager: Arc<ToolManager>,
    pub event_bus: Arc<EventBus>,
    pub context_mgr: Arc<ContextManager>,
    pub session_mgr: Arc<SessionManager>,
    pub trace_collector: Arc<TraceCollector>,
    pub storage: Arc<dyn Storage>,
    pub tool_router: Option<Arc<dyn ToolRouter>>,
    active_runs: Arc<RwLock<HashMap<String, ActiveRunEntry>>>,
}

impl AgentKernel {
    fn restore_tool_snapshot_value(&self, session_id: &str, snapshot: &serde_json::Value) -> Result<usize, String> {
        let items: Vec<SessionToolSnapshotItem> = serde_json::from_value(snapshot.clone())
            .map_err(|e| format!("invalid session tool snapshot: {}", e))?;
        let mut restored = 0usize;
        for item in items {
            if let Some(registration) = item.registration {
                self.tool_manager.restore(session_id, item.tool, registration)?;
                restored += 1;
            }
        }
        Ok(restored)
    }

    pub fn restore_session_tools(&self, session_id: &str) -> Result<usize, String> {
        let Some(snapshot) = self.session_mgr.get_session_tools(session_id) else {
            return Ok(0);
        };
        self.restore_tool_snapshot_value(session_id, &snapshot)
    }

    pub fn new(config: ProviderConfig) -> Self {
        Self::with_storage(config, Arc::new(MemoryStorage::new()))
    }

    /// 使用指定存储创建 AgentKernel
    pub fn with_storage(config: ProviderConfig, storage: Arc<dyn Storage>) -> Self {
        let event_bus = Arc::new(EventBus::new(4096));
        let tool_manager = Arc::new(ToolManager::new(event_bus.clone()));
        let context_mgr = Arc::new(ContextManager::new(
            config.context_window_tokens,
            80, // 默认 80% 阈值
            event_bus.clone(),
        ));
        let session_mgr = Arc::new(SessionManager::new(storage.clone(), event_bus.clone()));

        Self {
            config,
            system_prompt: RwLock::new(String::new()),
            provider_router: Router::default(),
            tool_manager,
            event_bus,
            context_mgr,
            session_mgr,
            trace_collector: Arc::new(TraceCollector::new()),
            storage,
            tool_router: None,
            active_runs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 设置 system prompt（builder）
    pub fn with_system_prompt(self, prompt: &str) -> Self {
        *self.system_prompt.write().unwrap() = prompt.to_string();
        self
    }

    /// 读取 system prompt
    pub fn get_system_prompt(&self) -> String {
        self.system_prompt.read().unwrap().clone()
    }

    /// 更新 system prompt（运行时可调用）
    pub fn set_system_prompt(&self, prompt: &str) {
        *self.system_prompt.write().unwrap() = prompt.to_string();
    }

    /// 设置工具路由器（用于将工具调用路由到外部客户端执行）
    pub fn with_tool_router(mut self, router: Arc<dyn ToolRouter>) -> Self {
        self.tool_router = Some(router);
        self
    }

    fn register_run_control(&self, session_id: &str, run_id: &str, started_at: Instant) -> Arc<RunControl> {
        let control = Arc::new(RunControl::new());
        self.active_runs
            .write()
            .unwrap()
            .insert(run_id.to_string(), ActiveRunEntry {
                session_id: session_id.to_string(),
                started_at,
                control: control.clone(),
            });
        control
    }

    pub fn cancel_run(&self, run_id: &str) -> bool {
        let control = self.active_runs
            .read()
            .unwrap()
            .get(run_id)
            .map(|entry| entry.control.clone());
        if let Some(control) = control {
            control.cancel();
            true
        } else {
            false
        }
    }

    pub fn list_active_runs(&self) -> Vec<ActiveRunSnapshot> {
        let mut runs: Vec<ActiveRunSnapshot> = self.active_runs
            .read()
            .unwrap()
            .iter()
            .map(|(run_id, entry)| ActiveRunSnapshot {
                session_id: entry.session_id.clone(),
                run_id: run_id.clone(),
                duration_ms: entry.started_at.elapsed().as_millis() as u64,
                status: if entry.control.is_cancelled() {
                    "cancelling".into()
                } else {
                    "running".into()
                },
            })
            .collect();
        runs.sort_by(|a, b| a.session_id.cmp(&b.session_id).then_with(|| a.run_id.cmp(&b.run_id)));
        runs
    }

    /// 检查指定 session 是否有正在运行的 run
    pub fn has_active_run_for_session(&self, session_id: &str) -> Option<String> {
        self.active_runs
            .read()
            .unwrap()
            .iter()
            .find(|(_, entry)| entry.session_id == session_id && !entry.control.is_cancelled())
            .map(|(run_id, _)| run_id.clone())
    }

    async fn finalize_cancelled_run(
        &self,
        session_id: &str,
        run_id: &str,
        partial_text: &Arc<RwLock<String>>,
        total_usage: Usage,
        traces: Vec<CallTrace>,
        tool_calls_made: u32,
        started_at: Instant,
    ) -> Result<ChatResponse, RuntimeErrorReport> {
        let content = partial_text.read().unwrap().clone();
        let partial_preserved = !content.trim().is_empty();

        if partial_preserved {
            let mut assistant_msg = Message {
                run_id: run_id.to_string(),
                ..Message::new(session_id, Role::Assistant, vec![ContentBlock::text(&content)])
            };
            assistant_msg.metadata.insert("interrupted".into(), serde_json::Value::Bool(true));
            self.context_mgr.add_message(session_id, assistant_msg.clone());
            self.storage
                .save_message(&assistant_msg)
                .await
                .map_err(|e| RuntimeErrorReport::core("run.cancelled.persist", e))?;
        }

        self.event_bus.emit(
            EventEnvelope::new(RUN_CANCELLED, session_id)
                .with_run_id(run_id)
                .with_payload(serde_json::json!({
                    "reason": "user_cancelled",
                    "partial_content": content,
                    "preserved": partial_preserved,
                    "duration_ms": started_at.elapsed().as_millis() as u64,
                })),
        );

        Ok(ChatResponse {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            content,
            status: "cancelled".into(),
            partial_preserved,
            usage: total_usage,
            traces,
            tool_calls_made,
        })
    }

    /// 注册工具
    pub fn register_tool(&self, session_id: &str, tool: Tool, registration: ToolRegistration) -> Result<(), String> {
        self.tool_manager.register(session_id, tool, registration)
    }

    /// 注销工具
    pub fn unregister_tool(&self, session_id: &str, name: &str) {
        self.tool_manager.unregister(session_id, name);
    }

    /// 订阅事件流
    pub fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.event_bus.subscribe()
    }

    /// 核心对话循环
    pub async fn chat(&self, session_id: &str, message: &str) -> Result<ChatResponse, RuntimeErrorReport> {
        let opts = ChatOptions {
            session_id: session_id.to_string(),
            message: message.to_string(),
            ..Default::default()
        };
        self.chat_with_options(opts).await
    }

    pub async fn chat_with_options(&self, mut opts: ChatOptions) -> Result<ChatResponse, RuntimeErrorReport> {
        if opts.session_id.is_empty() {
            return Err(RuntimeErrorReport::validation("chat.options", "session_id is required"));
        }
        if !opts.append_user_message && (!opts.message.is_empty() || !opts.images.is_empty() || !opts.audio.is_empty()) {
            return Err(RuntimeErrorReport::validation(
                "chat.options",
                "retry mode must not append message, images, or audio",
            ));
        }
        if opts.run_id.is_empty() {
            opts.run_id = format!("run_{}", uuid::Uuid::new_v4());
        }

        // 检查该 session 是否已有活跃 run，拒绝并发
        if let Some(active_run_id) = self.has_active_run_for_session(&opts.session_id) {
            return Err(RuntimeErrorReport::validation(
                "chat.concurrent",
                format!(
                    "session '{}' already has an active run ({}), wait for it to complete or cancel it first",
                    opts.session_id, active_run_id
                ),
            ));
        }

        let started_at = Instant::now();
        let run_control = self.register_run_control(&opts.session_id, &opts.run_id, started_at);
        let _run_cleanup = RunCleanup {
            active_runs: self.active_runs.clone(),
            run_id: opts.run_id.clone(),
        };
        let repeated_tool_call_limit = if opts.max_repeated_tool_calls > 0 {
            opts.max_repeated_tool_calls
        } else {
            DEFAULT_REPEATED_TOOL_CALL_LIMIT
        };

        // 优先用 session 级供应商配置，否则用全局默认
        let session_override = self.session_mgr.get_provider_override(&opts.session_id);
        let active_config: &ProviderConfig = match session_override {
            Some(ref cfg) => cfg,
            None => &self.config,
        };

        let adapter = self.provider_router.get(&active_config.protocol)
            .ok_or_else(|| RuntimeErrorReport::validation(
                "provider.resolve",
                format!("unsupported protocol: {:?}", active_config.protocol),
            ))?;

        self.event_bus.emit(
            EventEnvelope::new(RUN_STARTED, &opts.session_id)
                .with_run_id(&opts.run_id)
                .with_payload(serde_json::json!({
                    "provider": format!("{:?}", active_config.protocol).to_lowercase(),
                    "model": active_config.model,
                })),
        );

        // 能力校验：检查供应商是否支持图片/音频
        if !opts.images.is_empty() && !active_config.supports_image {
            return Err(RuntimeErrorReport::validation(
                "provider.capability",
                format!("当前供应商配置不支持图片输入（supports_image=false）。请在供应商设置中开启图片支持，或清除图片后重试。"),
            ));
        }
        if !opts.audio.is_empty() && !active_config.supports_audio {
            return Err(RuntimeErrorReport::validation(
                "provider.capability",
                format!("当前供应商配置不支持音频输入（supports_audio=false）。请在供应商设置中开启音频支持，或清除音频后重试。"),
            ));
        }

        // 首次发送时追加用户消息；重试时基于现有历史续跑，不新增 user message。
        if opts.append_user_message {
            let mut user_content = Vec::new();
            for img_b64 in &opts.images {
                // 前端传 base64 字符串，自动检测 media_type
                let (media_type, data) = if img_b64.starts_with("data:") {
                    // data:image/png;base64,xxxxx 格式
                    let parts: Vec<&str> = img_b64.splitn(2, ',').collect();
                    let header = parts.first().unwrap_or(&"");
                    let raw = parts.get(1).unwrap_or(&"");
                    let mt = header.split(':').nth(1).unwrap_or("image/png").split(';').next().unwrap_or("image/png");
                    (mt.to_string(), raw.to_string())
                } else {
                    ("image/png".to_string(), img_b64.clone())
                };
                user_content.push(ContentBlock::image(&media_type, &data));
            }
            // 追加音频
            for audio_input in &opts.audio {
                user_content.push(ContentBlock::audio(&audio_input.format, &audio_input.data));
            }
            user_content.push(ContentBlock::text(&opts.message));
            let user_msg = Message::new(&opts.session_id, Role::User, user_content);
            self.context_mgr.add_message(&opts.session_id, user_msg.clone());
            self.storage.save_message(&user_msg).await?;
        }

        // 获取当前 session 可用工具：如果 session metadata 有工具快照，则按快照过滤；否则使用全局已注册工具
        self.restore_session_tools(&opts.session_id)?;
        let all_tools = self.tool_manager.get_tools(&opts.session_id);
        let active_tools = self.session_mgr
            .get_session_tools(&opts.session_id)
            .and_then(|snapshot| snapshot.as_array().cloned())
            .map(|items| {
                let names: std::collections::HashSet<String> = items.iter()
                    .filter_map(|item| item.get("tool"))
                    .filter_map(|tool| tool.get("name"))
                    .filter_map(|name| name.as_str().map(String::from))
                    .collect();
                all_tools.iter()
                    .filter(|tool| names.contains(&tool.name))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or(all_tools);

        let mut total_usage = Usage::default();
        let mut traces = Vec::new();
        let mut tool_calls_made = 0u32;
        let partial_text = Arc::new(RwLock::new(String::new()));
        let mut repeated_tool_call_guard = RepeatedToolCallGuard::default();
        let mut round = 0u32;

        loop {
            if run_control.is_cancelled() {
                return self.finalize_cancelled_run(
                    &opts.session_id,
                    &opts.run_id,
                    &partial_text,
                    total_usage,
                    traces,
                    tool_calls_made,
                    started_at,
                ).await;
            }
            // 用户消息边界才应用 checkpoint 裁剪，避免 AI tool loop 中途切断 tool_use/tool_result 链。
            if round == 0 && opts.append_user_message {
                if let Some(ctx) = self.context_mgr.apply_checkpoint_trim_if_needed(&opts.session_id) {
                    self.storage.save_context_state(&ctx).await?;
                    self.event_bus.emit(EventEnvelope::new("context.updated", &opts.session_id)
                        .with_run_id(&opts.run_id)
                        .with_payload(serde_json::json!({"action": "trim.checkpoint_applied", "context": ctx})));
                }
            }

            // 阈值检测
            if round == 0 && self.context_mgr.should_compress(&opts.session_id) {
                let stats = self.context_mgr.stats(&opts.session_id);
                self.event_bus.emit(EventEnvelope::new(CONTEXT_THRESHOLD_REACHED, &opts.session_id)
                    .with_run_id(&opts.run_id)
                    .with_payload(serde_json::json!({
                        "estimated_tokens": stats.estimated_tokens,
                        "window_tokens": stats.window_tokens,
                        "usage_percent": stats.usage_percent,
                    })));
            }

            // 调用供应商
            let session_system_prompt = self.session_mgr
                .get_system_prompt(&opts.session_id)
                .unwrap_or_else(|| self.system_prompt.read().unwrap().clone());
            let system_prompt = session_system_prompt;
            let bus = self.event_bus.clone();
            let sid = opts.session_id.clone();
            let rid = opts.run_id.clone();
            let partial_text_for_handler = partial_text.clone();
            let handler: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(move |event| {
                let mut event = event;
                event.session_id = sid.clone();
                event.run_id = rid.clone();
                if matches!(event.event, StreamEventType::Text) {
                    *partial_text_for_handler.write().unwrap() = event.full_text.clone();
                }
                bus.emit(EventEnvelope::new(MODEL_DELTA, &sid)
                    .with_run_id(&rid)
                    .with_payload(serde_json::json!({
                        "delta": event.delta,
                        "event_type": event.event,
                    })));
            });

            let active_config_owned = active_config.clone();
            let system_prompt_owned = system_prompt.clone();
            let model_input = self.context_mgr.build_model_input(&opts.session_id);
            let tool_chain_report = analyze_tool_chain(&model_input);
            self.event_bus.emit(
                EventEnvelope::new(TOOL_CHAIN_DIAGNOSED, &opts.session_id)
                    .with_run_id(&opts.run_id)
                    .with_payload(serde_json::json!({
                        "report": tool_chain_report,
                    })),
            );
            let tools_for_round = active_tools.clone();
            let adapter_for_round = adapter.clone();
            let stream_task = tokio::spawn(async move {
                adapter_for_round
                    .stream_message(
                        &active_config_owned,
                        &system_prompt_owned,
                        &model_input,
                        &tools_for_round,
                        handler,
                    )
                    .await
            });
            let abort_handle = stream_task.abort_handle();

            let (resp, trace) = tokio::select! {
                _ = run_control.notify.notified() => {
                    abort_handle.abort();
                    return self.finalize_cancelled_run(
                        &opts.session_id,
                        &opts.run_id,
                        &partial_text,
                        total_usage,
                        traces,
                        tool_calls_made,
                        started_at,
                    ).await;
                }
                joined = stream_task => {
                    match joined {
                        Ok(Ok((resp, trace))) => (resp, trace),
                        Ok(Err(e)) => {
                            let report = RuntimeErrorReport::provider("model.stream", e);
                            self.event_bus.emit(EventEnvelope::new(RUN_FAILED, &opts.session_id)
                                .with_run_id(&opts.run_id)
                                .with_payload(report.to_payload()));
                            return Err(report);
                        }
                        Err(e) if e.is_cancelled() && run_control.is_cancelled() => {
                            return self.finalize_cancelled_run(
                                &opts.session_id,
                                &opts.run_id,
                                &partial_text,
                                total_usage,
                                traces,
                                tool_calls_made,
                                started_at,
                            ).await;
                        }
                        Err(e) => {
                            return Err(RuntimeErrorReport::core("model.stream.join", e.to_string()));
                        }
                    }
                }
            };

            traces.push(trace);
            total_usage.input_tokens += resp.usage.input_tokens;
            total_usage.output_tokens += resp.usage.output_tokens;

            // 检查工具调用
            let tool_uses: Vec<&ContentBlock> = resp.content.iter()
                .filter(|c| matches!(c, ContentBlock::ToolUse { .. }))
                .collect();

            if tool_uses.is_empty() {
                // 无工具调用 → 结束；空响应不落盘，否则会把失败态伪装成最终 assistant 消息，阻断 retry。
                let text: String = resp.content.iter().filter_map(|c| {
                    if let ContentBlock::Text { text, .. } = c { Some(text.as_str()) } else { None }
                }).collect();

                if text.trim().is_empty() && resp.content.is_empty() {
                    let report = RuntimeErrorReport::provider_retryable(
                        "model.empty_response",
                        "model returned an empty final response without text or tool_use; retry the run or check upstream provider output",
                    );
                    self.event_bus.emit(EventEnvelope::new(RUN_FAILED, &opts.session_id)
                        .with_run_id(&opts.run_id)
                        .with_payload(report.to_payload()));
                    return Err(report);
                }

                let assistant_msg = Message {
                    run_id: opts.run_id.clone(),
                    ..Message::new(&opts.session_id, Role::Assistant, resp.content.clone())
                };
                self.context_mgr.add_message(&opts.session_id, assistant_msg.clone());
                self.storage.save_message(&assistant_msg).await?;

                self.event_bus.emit(EventEnvelope::new(MODEL_COMPLETED, &opts.session_id)
                    .with_run_id(&opts.run_id)
                    .with_payload(serde_json::json!({"content": text})));

                self.event_bus.emit(EventEnvelope::new(RUN_COMPLETED, &opts.session_id)
                    .with_run_id(&opts.run_id)
                    .with_payload(serde_json::json!({
                        "status": "completed",
                        "input_tokens": total_usage.input_tokens,
                        "output_tokens": total_usage.output_tokens,
                        "total_tokens": total_usage.input_tokens + total_usage.output_tokens,
                        "tool_calls_made": tool_calls_made,
                        "duration_ms": started_at.elapsed().as_millis() as u64,
                    })));

                return Ok(ChatResponse {
                    session_id: opts.session_id,
                    run_id: opts.run_id,
                    content: text,
                    status: "completed".into(),
                    partial_preserved: false,
                    usage: total_usage,
                    traces,
                    tool_calls_made,
                });
            }

            for tool_use in &tool_uses {
                if let ContentBlock::ToolUse { name, input, .. } = tool_use {
                    let consecutive_count = repeated_tool_call_guard.observe(name, input);
                    if consecutive_count >= repeated_tool_call_limit {
                        let fingerprint = repeated_tool_call_guard
                            .last()
                            .cloned()
                            .unwrap_or(ToolCallFingerprint {
                                tool_name: name.clone(),
                                input_json: canonicalize_json(input),
                            });
                        let report = RuntimeErrorReport::core(
                            "tool.repeated_call_loop",
                            format!(
                                "detected repeated tool call loop: tool '{}' with identical input repeated {} times consecutively; input={}",
                                fingerprint.tool_name,
                                consecutive_count,
                                fingerprint.input_json
                            ),
                        );
                        self.event_bus.emit(
                            EventEnvelope::new(RUN_FAILED, &opts.session_id)
                                .with_run_id(&opts.run_id)
                                .with_payload(report.to_payload()),
                        );
                        return Err(report);
                    }
                }
            }

            // 保存助手消息（含 tool_use）
            let assistant_tool_msg = Message {
                run_id: opts.run_id.clone(),
                ..Message::new(&opts.session_id, Role::Assistant, resp.content.clone())
            };
            self.context_mgr.add_message(&opts.session_id, assistant_tool_msg.clone());
            self.storage.save_message(&assistant_tool_msg).await?;

            // 执行工具（先校验，再并发路由；全部结果聚合后再回模型）
            let mut tool_tasks = Vec::new();
            for tu in &tool_uses {
                if let ContentBlock::ToolUse { id, name, input } = tu {
                    let id = id.clone();
                    let name = name.clone();
                    let input = input.clone();

                    // ── Core 校验层：存在性 + 必填参数 ──
                    let validation_err = if !self.tool_manager.has_tool(&opts.session_id, &name) {
                        Some(format!("工具 '{}' 不存在。已注册的工具: {:?}", name, self.tool_manager.tool_names(&opts.session_id)))
                    } else {
                        // 检查 input_schema 中的 required 字段
                        self.tool_manager.get_tool(&opts.session_id, &name).and_then(|tool| {
                            let required: Vec<String> = tool.input_schema
                                .get("required")
                                .and_then(|r| r.as_array())
                                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                                .unwrap_or_default();
                            if required.is_empty() {
                                None
                            } else {
                                let input_obj = input.as_object();
                                let missing: Vec<&str> = required.iter()
                                    .filter(|f| {
                                        input_obj.is_none() ||
                                        !input_obj.unwrap().contains_key(f.as_str()) ||
                                        input_obj.unwrap()[f.as_str()].is_null()
                                    })
                                    .map(|f| f.as_str())
                                    .collect();
                                if missing.is_empty() { None }
                                else { Some(format!("工具 '{}' 缺少必填参数: {:?}", name, missing)) }
                            }
                        })
                    };

                    if let Some(err) = validation_err {
                        // 校验失败 → Core 直接报错给模型，不走 WS
                        tracing::warn!(tool = %name, error = %err, "tool validation failed");
                        tool_tasks.push(futures::future::ready((id, name, err, true)).boxed());
                    } else if let Some(router) = self.tool_router.clone() {
                        let session_id = opts.session_id.clone();
                        let run_id = opts.run_id.clone();
                        let timeout_ms = self.tool_manager
                            .get_registration(&session_id, &name)
                            .map(|registration| registration.timeout_ms)
                            .unwrap_or(0);
                        tool_tasks.push(async move {
                            match router.execute(&session_id, &run_id, &name, &id, input, timeout_ms).await {
                                Ok((result, is_error)) => (id, name, result, is_error),
                                Err(e) => (id, name, format!("tool routing error: {}", e), true),
                            }
                        }.boxed());
                    } else {
                        tool_tasks.push(futures::future::ready((id.clone(), name.clone(), format!("tool '{}' executed (no router)", name), false)).boxed());
                    }
                }
            }

            let tool_outputs = tokio::select! {
                _ = run_control.notify.notified() => {
                    if let Some(router) = self.tool_router.clone() {
                        let _ = router.cancel_run(&opts.run_id).await;
                    }
                    // 取消时，为所有未完成的工具调用保存 error tool_result
                    // 避免消息历史中出现没有 result 的 tool_use
                    let mut cancelled_results = Vec::new();
                    for tu in &tool_uses {
                        if let ContentBlock::ToolUse { id, name, .. } = tu {
                            cancelled_results.push(ContentBlock::tool_result(
                                id,
                                &format!("工具 '{}' 执行被用户中断", name),
                                true,
                            ));
                        }
                    }
                    if !cancelled_results.is_empty() {
                        let cancel_result_msg = Message::new(&opts.session_id, Role::User, cancelled_results);
                        self.context_mgr.add_message(&opts.session_id, cancel_result_msg.clone());
                        let _ = self.storage.save_message(&cancel_result_msg).await;
                    }
                    return self.finalize_cancelled_run(
                        &opts.session_id,
                        &opts.run_id,
                        &partial_text,
                        total_usage,
                        traces,
                        tool_calls_made,
                        started_at,
                    ).await;
                }
                outputs = futures::future::join_all(tool_tasks) => outputs,
            };
            let mut tool_results = Vec::new();
            for (id, name, result, is_error) in tool_outputs {
                tool_results.push(ContentBlock::tool_result(&id, &result, is_error));
                tool_calls_made += 1;

                // 这是 Core 对工具执行结果的事件记录，用于事件流、trace、debug、replay；不是再次要求前端执行。
                self.event_bus.emit(EventEnvelope::new(TOOL_CALL_RESULT, &opts.session_id)
                    .with_run_id(&opts.run_id)
                    .with_payload(serde_json::json!({
                        "tool_name": name, "call_id": id,
                        "result": result, "is_error": is_error,
                    })));
            }

            // 追加工具结果
            let tool_result_msg = Message::new(&opts.session_id, Role::User, tool_results);
            self.context_mgr.add_message(&opts.session_id, tool_result_msg.clone());
            self.storage.save_message(&tool_result_msg).await?;
            round = round.saturating_add(1);
        }
    }

    pub async fn retry_session(&self, session_id: &str, run_id: &str, max_repeated_tool_calls: u32) -> Result<ChatResponse, RuntimeErrorReport> {
        if session_id.is_empty() {
            return Err(RuntimeErrorReport::validation("session.retry", "session_id is required"));
        }

        self.load_session_state(session_id)
            .await
            .map_err(|e| RuntimeErrorReport::core("session.retry.load", e))?;

        let messages = self.context_mgr.get_all_messages(session_id);
        if messages.is_empty() {
            return Err(RuntimeErrorReport::validation("session.retry", "session has no messages to retry"));
        }

        let mut last_non_empty: Option<&Message> = None;
        for msg in messages.iter().rev() {
            if msg.content.is_empty() {
                continue;
            }
            last_non_empty = Some(msg);
            break;
        }

        let Some(last_msg) = last_non_empty else {
            return Err(RuntimeErrorReport::validation("session.retry", "session has no retryable messages"));
        };

        if last_msg.role == Role::Assistant && !last_msg.content.iter().any(|c| matches!(c, ContentBlock::ToolUse { .. })) {
            return Err(RuntimeErrorReport::validation(
                "session.retry",
                "last message is already a final assistant response; retry is only allowed for unfinished/failed runs",
            ));
        }

        // 如果最后停在 assistant tool_use，说明还有 pending tool call 没有回填结果，不能凭空续跑。
        if last_msg.role == Role::Assistant && last_msg.content.iter().any(|c| matches!(c, ContentBlock::ToolUse { .. })) {
            return Err(RuntimeErrorReport::validation(
                "session.retry",
                "last message contains pending tool_use without tool_result; wait for tool result or cancel the run first",
            ));
        }

        self.chat_with_options(ChatOptions {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            message: String::new(),
            images: Vec::new(),
            audio: Vec::new(),
            max_repeated_tool_calls,
            append_user_message: false,
        }).await
    }

    /// 按需从存储层加载某个 session 的完整状态到内存
    pub async fn load_session_state(&self, session_id: &str) -> Result<(), String> {
        if let Some(session) = self.storage.get_session(session_id).await? {
            self.session_mgr.load_session(session.clone());
        }
        self.restore_session_tools(session_id)?;
        let messages = self.storage.get_messages(session_id).await?;
        self.context_mgr.load_messages(session_id, messages);
        let ctx = self.storage.get_context_state(session_id).await?;
        self.context_mgr.load_context_state(session_id, ctx);
        let seeds = self.storage.get_seeds(session_id).await?;
        self.context_mgr.load_seeds(session_id, seeds);
        Ok(())
    }

    /// 启动时只加载 session 索引/配置，不加载全量消息，避免启动和频繁 I/O 压力
    pub async fn load_session_index(&self) -> Result<usize, String> {
        let sessions = self.storage.list_sessions().await?;
        let count = sessions.len();
        for session in sessions {
            let snapshot = session.metadata.get("tools").cloned();
            let session_id = session.session_id.clone();
            self.session_mgr.load_session(session);
            if let Some(snapshot) = snapshot {
                self.restore_tool_snapshot_value(&session_id, &snapshot)?;
            }
        }
        Ok(count)
    }

    /// 获取 session 统计
    pub fn session_stats(&self, session_id: &str) -> core_context::ContextStats {
        self.context_mgr.stats(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_json, RepeatedToolCallGuard};

    #[test]
    fn canonicalize_json_sorts_object_keys_recursively() {
        let input = serde_json::json!({
            "b": 2,
            "a": {
                "z": true,
                "m": [ {"k": 2, "a": 1} ]
            }
        });

        assert_eq!(
            canonicalize_json(&input),
            r#"{"a":{"m":[{"a":1,"k":2}],"z":true},"b":2}"#
        );
    }

    #[test]
    fn repeated_tool_call_guard_detects_consecutive_identical_calls() {
        let mut guard = RepeatedToolCallGuard::default();
        let first = serde_json::json!({"b": 2, "a": 1});
        let same_different_order = serde_json::json!({"a": 1, "b": 2});

        for _ in 0..9 {
            assert!(guard.observe("calc", &first) < 10);
        }

        assert_eq!(guard.observe("calc", &same_different_order), 10);
    }

    #[test]
    fn repeated_tool_call_guard_resets_when_tool_or_input_changes() {
        let mut guard = RepeatedToolCallGuard::default();
        assert_eq!(guard.observe("calc", &serde_json::json!({"expression": "1+1"})), 1);
        assert_eq!(guard.observe("calc", &serde_json::json!({"expression": "1+1"})), 2);
        assert_eq!(guard.observe("calc", &serde_json::json!({"expression": "2+2"})), 1);
        assert_eq!(guard.observe("echo", &serde_json::json!({"text": "2+2"})), 1);
    }
}

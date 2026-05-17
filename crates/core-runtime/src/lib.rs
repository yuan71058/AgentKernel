//! # Core Runtime
//!
//! AgentKernel 的顶层运行时入口，串联所有子 crate。
//!
//! 用法：
//! ```rust,no_run
//! use core_runtime::AgentKernel;
//! use core_protocol::*;
//!
//! # async fn example() -> Result<(), String> {
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
//!     Tool { name: "calc".into(), description: "计算器".into(), input_schema: serde_json::json!({}) },
//!     ToolRegistration { tool_name: "calc".into(), client_id: "local".into(), .. },
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
use core_model::{Router, CallTrace, should_retry};
use core_protocol::*;
use core_session::SessionManager;
use core_storage::{Storage, MemoryStorage};
use core_tool::ToolManager;
use core_trace::TraceCollector;
use std::any::Any;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;
use futures::FutureExt;

/// 对话选项
#[derive(Debug, Clone)]
pub struct ChatOptions {
    pub session_id: String,
    pub run_id: String,
    pub message: String,
    pub images: Vec<String>,
    pub max_rounds: u32,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            run_id: String::new(),
            message: String::new(),
            images: Vec::new(),
            max_rounds: 10,
        }
    }
}

/// 对话响应
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub session_id: String,
    pub run_id: String,
    pub content: String,
    pub usage: Usage,
    pub traces: Vec<CallTrace>,
    pub tool_calls_made: u32,
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
    ) -> Result<(String, bool), String>;

    /// 支持 downcast（用于 WS 层解析工具结果）
    fn as_any(&self) -> &dyn Any;
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
}

impl AgentKernel {
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

    /// 注册工具
    pub fn register_tool(&self, tool: Tool, registration: ToolRegistration) {
        self.tool_manager.register(tool, registration);
    }

    /// 注销工具
    pub fn unregister_tool(&self, name: &str) {
        self.tool_manager.unregister(name);
    }

    /// 订阅事件流
    pub fn subscribe_events(&self) -> broadcast::Receiver<EventEnvelope> {
        self.event_bus.subscribe()
    }

    /// 核心对话循环
    pub async fn chat(&self, session_id: &str, message: &str) -> Result<ChatResponse, String> {
        let opts = ChatOptions {
            session_id: session_id.to_string(),
            message: message.to_string(),
            ..Default::default()
        };
        self.chat_with_options(opts).await
    }

    pub async fn chat_with_options(&self, mut opts: ChatOptions) -> Result<ChatResponse, String> {
        if opts.session_id.is_empty() {
            return Err("session_id is required".into());
        }
        if opts.run_id.is_empty() {
            opts.run_id = format!("run_{}", uuid::Uuid::new_v4());
        }
        let max_rounds = if opts.max_rounds > 0 { opts.max_rounds } else { 10 };

        // 优先用 session 级供应商配置，否则用全局默认
        let session_override = self.session_mgr.get_provider_override(&opts.session_id);
        let active_config: &ProviderConfig = match session_override {
            Some(ref cfg) => cfg,
            None => &self.config,
        };

        let adapter = self.provider_router.get(&active_config.protocol)
            .ok_or_else(|| format!("unsupported protocol: {:?}", active_config.protocol))?;

        // 追加用户消息
        let user_content = vec![ContentBlock::text(&opts.message)];
        let user_msg = Message::new(&opts.session_id, Role::User, user_content);
        self.context_mgr.add_message(&opts.session_id, user_msg.clone());
        self.storage.save_message(&user_msg).await?;

        // 获取当前 session 可用工具：如果 session metadata 有工具快照，则按快照过滤；否则使用全局已注册工具
        let all_tools = self.tool_manager.get_tools();
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

        for round in 0..max_rounds {
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
            let mut last_err = String::new();
            let mut resp_opt = None;
            let mut trace_opt = None;

            for attempt in 0..3u32 {
                let bus = self.event_bus.clone();
                let sid = opts.session_id.clone();
                let rid = opts.run_id.clone();
                let handler: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(move |event| {
                    let mut event = event;
                    event.session_id = sid.clone();
                    event.run_id = rid.clone();
                    bus.emit(EventEnvelope::new(MODEL_DELTA, &sid)
                        .with_run_id(&rid)
                        .with_payload(serde_json::json!({
                            "delta": event.delta,
                            "event_type": event.event,
                        })));
                });

                match adapter.stream_message(
                    active_config,
                    &system_prompt,
                    &self.context_mgr.build_model_input(&opts.session_id),
                    &active_tools,
                    handler,
                ).await {
                    Ok((resp, trace)) => {
                        resp_opt = Some(resp);
                        trace_opt = Some(trace);
                        break;
                    }
                    Err(e) => {
                        if !should_retry(&e) || attempt >= 2 {
                            last_err = e;
                            break;
                        }
                        self.event_bus.emit(EventEnvelope::new(ERROR, &opts.session_id)
                            .with_run_id(&opts.run_id)
                            .with_payload(serde_json::json!({"retry": attempt + 1, "error": e})));
                        last_err = e;
                        tokio::time::sleep(std::time::Duration::from_millis(
                            match attempt { 0 => 300, 1 => 1500, _ => 5000 }
                        )).await;
                    }
                }
            }

            let (resp, trace) = match (resp_opt, trace_opt) {
                (Some(r), Some(t)) => (r, t),
                _ => {
                    self.event_bus.emit(EventEnvelope::new(ERROR, &opts.session_id)
                        .with_run_id(&opts.run_id)
                        .with_payload(serde_json::json!({"error": last_err})));
                    return Err(last_err);
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
                // 无工具调用 → 结束
                let assistant_msg = Message {
                    run_id: opts.run_id.clone(),
                    ..Message::new(&opts.session_id, Role::Assistant, resp.content.clone())
                };
                self.context_mgr.add_message(&opts.session_id, assistant_msg.clone());
                self.storage.save_message(&assistant_msg).await?;

                let text: String = resp.content.iter().filter_map(|c| {
                    if let ContentBlock::Text { text, .. } = c { Some(text.as_str()) } else { None }
                }).collect();

                self.event_bus.emit(EventEnvelope::new(MODEL_COMPLETED, &opts.session_id)
                    .with_run_id(&opts.run_id)
                    .with_payload(serde_json::json!({"content": text})));

                return Ok(ChatResponse {
                    session_id: opts.session_id,
                    run_id: opts.run_id,
                    content: text,
                    usage: total_usage,
                    traces,
                    tool_calls_made,
                });
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
                    let validation_err = if !self.tool_manager.has_tool(&name) {
                        Some(format!("工具 '{}' 不存在。已注册的工具: {:?}", name, self.tool_manager.tool_names()))
                    } else {
                        // 检查 input_schema 中的 required 字段
                        self.tool_manager.get_tool(&name).and_then(|tool| {
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
                        tool_tasks.push(async move {
                            match router.execute(&session_id, &run_id, &name, &id, input).await {
                                Ok((result, is_error)) => (id, name, result, is_error),
                                Err(e) => (id, name, format!("tool routing error: {}", e), true),
                            }
                        }.boxed());
                    } else {
                        tool_tasks.push(futures::future::ready((id.clone(), name.clone(), format!("tool '{}' executed (no router)", name), false)).boxed());
                    }
                }
            }

            let tool_outputs = futures::future::join_all(tool_tasks).await;
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
        }

        Err(format!("tool call round limit ({}) exceeded", max_rounds))
    }

    /// 按需从存储层加载某个 session 的完整状态到内存
    pub async fn load_session_state(&self, session_id: &str) -> Result<(), String> {
        if let Some(session) = self.storage.get_session(session_id).await? {
            self.session_mgr.load_session(session.clone());
        }
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
            self.session_mgr.load_session(session);
        }
        Ok(count)
    }

    /// 获取 session 统计
    pub fn session_stats(&self, session_id: &str) -> core_context::ContextStats {
        self.context_mgr.stats(session_id)
    }

    /// 清除 session
    pub fn clear_session(&self, session_id: &str) {
        self.context_mgr.remove_session(session_id);
    }
}

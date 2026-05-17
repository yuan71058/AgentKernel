//! # Core Runtime
//!
//! AI Runtime Core 的顶层入口，串联所有子 crate。
//!
//! 用法：
//! ```rust,no_run
//! use core_runtime::Scaffold;
//! use core_protocol::*;
//!
//! # async fn example() -> Result<(), String> {
//! let scaffold = Scaffold::new(ProviderConfig {
//!     protocol: Protocol::OpenAI,
//!     base_url: "https://api.deepseek.com".into(),
//!     api_key: "sk-...".into(),
//!     model: "deepseek-chat".into(),
//!     ..Default::default()
//! });
//!
//! // 注册工具
//! scaffold.register_tool(
//!     Tool { name: "calc".into(), description: "计算器".into(), input_schema: serde_json::json!({}) },
//!     ToolRegistration { tool_name: "calc".into(), client_id: "local".into(), .. },
//! );
//!
//! // 对话
//! let resp = scaffold.chat("sess_1", "你好").await?;
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
use std::sync::Arc;
use tokio::sync::broadcast;

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

/// AI Runtime Core 脚手架
pub struct Scaffold {
    pub config: ProviderConfig,
    pub system_prompt: String,
    pub provider_router: Router,
    pub tool_manager: Arc<ToolManager>,
    pub event_bus: Arc<EventBus>,
    pub context_mgr: Arc<ContextManager>,
    pub session_mgr: Arc<SessionManager>,
    pub trace_collector: Arc<TraceCollector>,
    pub storage: Arc<dyn Storage>,
}

impl Scaffold {
    pub fn new(config: ProviderConfig) -> Self {
        let event_bus = Arc::new(EventBus::new(4096));
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let tool_manager = Arc::new(ToolManager::new(event_bus.clone()));
        let context_mgr = Arc::new(ContextManager::new(
            config.context_window_tokens,
            80, // 默认 80% 阈值
            event_bus.clone(),
        ));
        let session_mgr = Arc::new(SessionManager::new(storage.clone(), event_bus.clone()));

        Self {
            config,
            system_prompt: String::new(),
            provider_router: Router::default(),
            tool_manager,
            event_bus,
            context_mgr,
            session_mgr,
            trace_collector: Arc::new(TraceCollector::new()),
            storage,
        }
    }

    /// 设置 system prompt
    pub fn with_system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = prompt.to_string();
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
        self.context_mgr.add_message(&opts.session_id, Message::new(&opts.session_id, Role::User, user_content));

        // 获取活跃工具
        let active_tools = self.tool_manager.get_tools();

        // 构建模型输入（seeds + context view）
        let messages = self.context_mgr.build_model_input(&opts.session_id);

        let mut total_usage = Usage::default();
        let mut traces = Vec::new();

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
                    &self.system_prompt,
                    &messages,
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
                self.context_mgr.add_message(&opts.session_id, Message {
                    run_id: opts.run_id.clone(),
                    ..Message::new(&opts.session_id, Role::Assistant, resp.content.clone())
                });

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
                    tool_calls_made: round,
                });
            }

            // 保存助手消息（含 tool_use）
            self.context_mgr.add_message(&opts.session_id, Message {
                run_id: opts.run_id.clone(),
                ..Message::new(&opts.session_id, Role::Assistant, resp.content.clone())
            });

            // 执行工具
            let mut tool_results = Vec::new();
            for tu in &tool_uses {
                if let ContentBlock::ToolUse { id, name, input } = tu {
                    self.event_bus.emit(EventEnvelope::new(TOOL_CALL_REQUEST, &opts.session_id)
                        .with_run_id(&opts.run_id)
                        .with_payload(serde_json::json!({"tool_name": name, "call_id": id, "input": input})));

                    // 这里应路由给对应客户端执行，目前留空
                    let result = format!("tool '{}' executed (stub)", name);
                    let is_error = false;

                    tool_results.push(ContentBlock::tool_result(id, &result, is_error));

                    self.event_bus.emit(EventEnvelope::new(TOOL_CALL_RESULT, &opts.session_id)
                        .with_run_id(&opts.run_id)
                        .with_payload(serde_json::json!({
                            "tool_name": name, "call_id": id,
                            "result": result, "is_error": is_error,
                        })));
                }
            }

            // 追加工具结果
            self.context_mgr.add_message(&opts.session_id, Message::new(
                &opts.session_id, Role::User, tool_results,
            ));
        }

        Err(format!("tool call round limit ({}) exceeded", max_rounds))
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

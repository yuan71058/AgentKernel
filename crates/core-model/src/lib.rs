//! # Core Model
//!
//! 供应商抽象层。支持 Claude / OpenAI 兼容协议。
//! 关键：Claude 适配器过滤 assistant 消息中的 tool_use 块（ai.accbot.vip 兼容）。

pub mod claude;
pub mod openai;
pub mod trace;

use async_trait::async_trait;
use core_protocol::*;
use std::sync::Arc;
use tracing::info;

/// 供应商适配器 trait
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn protocol(&self) -> Protocol;
    fn name(&self) -> &str;

    async fn send_message(
        &self,
        config: &ProviderConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Response, CallTrace), String>;

    async fn stream_message(
        &self,
        config: &ProviderConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        handler: Box<dyn Fn(StreamEvent) + Send + Sync>,
    ) -> Result<(Response, CallTrace), String>;
}

/// 调用追踪
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CallTrace {
    pub trace_id: String,
    pub api_url: String,
    pub protocol: String,
    pub model: String,
    pub request_body: String,
    pub request_headers: std::collections::HashMap<String, String>,
    pub response_code: u16,
    pub response_body: String,
    pub response_headers: std::collections::HashMap<String, String>,
    pub provider_request_id: String,
    pub duration_ms: u64,
    pub attempt: u32,
    pub stream: bool,
    pub finish_reason: String,
    pub error: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// 供应商路由器
pub struct Router {
    adapters: std::collections::HashMap<Protocol, Arc<dyn ProviderAdapter>>,
}

impl Router {
    pub fn new() -> Self {
        Self { adapters: std::collections::HashMap::new() }
    }

    pub fn register(&mut self, adapter: Arc<dyn ProviderAdapter>) {
        info!(protocol = ?adapter.protocol(), name = adapter.name(), "adapter registered");
        self.adapters.insert(adapter.protocol(), adapter);
    }

    pub fn get(&self, protocol: &Protocol) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters.get(protocol).cloned()
    }
}

impl Default for Router {
    fn default() -> Self {
        let mut router = Self::new();
        router.register(Arc::new(claude::ClaudeAdapter));
        router.register(Arc::new(openai::OpenAIAdapter));
        router
    }
}

// ═══════════════════════════════════════════════════════════════
//  消息规范化（Claude 适配器关键：过滤 tool_use）
// ═══════════════════════════════════════════════════════════════

/// 过滤 assistant 消息中不能回传的内容块。
///
/// **关键**：ai.accbot.vip 不接受 assistant 消息中的 tool_use 块（返回 400）。
/// 只保留 text 块，过滤 tool_use / thinking / redacted_thinking。
/// tool_result 在 user 消息中保留，模型能从 tool_result 推断之前的调用。
pub fn normalize_for_claude(messages: &[Message]) -> Vec<Message> {
    messages.iter().map(|msg| {
        let content: Vec<ContentBlock> = msg.content.iter().filter_map(|c| {
            match c {
                ContentBlock::Text { text, .. } => Some(ContentBlock::text(text)),
                ContentBlock::ToolResult { tool_use_id, content, is_error } => {
                    Some(ContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: content.clone(),
                        is_error: *is_error,
                    })
                }
                ContentBlock::Image { source } => Some(ContentBlock::Image { source: source.clone() }),
                // 关键：过滤 tool_use（assistant 消息中）
                ContentBlock::ToolUse { .. } => None,
                // 过滤 thinking
                ContentBlock::Thinking { .. } => None,
            }
        }).collect();
        Message {
            content,
            ..msg.clone()
        }
    }).collect()
}

/// OpenAI 适配器的消息转换（tool_use → tool_calls，tool_result → role:"tool"）
pub fn convert_for_openai(messages: &[Message]) -> Vec<serde_json::Value> {
    let mut result = Vec::new();
    for msg in messages {
        match msg.role {
            Role::Assistant => {
                let mut text_parts = Vec::new();
                let mut tool_calls = Vec::new();
                for c in &msg.content {
                    match c {
                        ContentBlock::Text { text, .. } => text_parts.push(text.clone()),
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_calls.push(serde_json::json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(input).unwrap_or_default()
                                }
                            }));
                        }
                        _ => {}
                    }
                }
                let mut m = serde_json::json!({ "role": "assistant" });
                if !text_parts.is_empty() {
                    m["content"] = serde_json::Value::String(text_parts.join("\n"));
                }
                if !tool_calls.is_empty() {
                    m["tool_calls"] = serde_json::Value::Array(tool_calls);
                }
                result.push(m);
            }
            Role::User => {
                let mut text_parts = Vec::new();
                let mut tool_results = Vec::new();
                for c in &msg.content {
                    match c {
                        ContentBlock::Text { text, .. } => text_parts.push(text.clone()),
                        ContentBlock::ToolResult { tool_use_id, content, .. } => {
                            tool_results.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content.as_deref().unwrap_or("")
                            }));
                        }
                        _ => {}
                    }
                }
                for tr in tool_results {
                    result.push(tr);
                }
                if !text_parts.is_empty() {
                    result.push(serde_json::json!({
                        "role": "user",
                        "content": text_parts.join("\n")
                    }));
                }
            }
            Role::System => {
                let text: String = msg.content.iter().filter_map(|c| {
                    if let ContentBlock::Text { text, .. } = c { Some(text.as_str()) } else { None }
                }).collect();
                if !text.is_empty() {
                    result.push(serde_json::json!({ "role": "system", "content": text }));
                }
            }
            _ => {}
        }
    }
    result
}

/// 错误重试判断
pub fn should_retry(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("rate limit")
        || lower.contains("429")
        || lower.contains("timeout")
        || lower.contains("503")
        || lower.contains("502")
        || lower.contains("504")
        || lower.contains("overloaded")
        || lower.contains("eof")
}

//! # Core Model
//!
//! 供应商抽象层。支持 Claude / OpenAI 兼容协议。
//! 关键：协议共享一套工具链规范化逻辑，避免不同 provider 因 tool_use/tool_result
//! 链路不完整而出现格式错误。

pub mod claude;
pub mod openai;
pub mod trace;

use async_trait::async_trait;
use core_protocol::*;
use std::collections::{BTreeSet, HashSet};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_chain_report: Option<ToolChainReport>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ToolChainReport {
    pub original_message_count: usize,
    pub sanitized_message_count: usize,
    pub complete_tool_call_ids: Vec<String>,
    pub dropped_incomplete_tool_call_ids: Vec<String>,
    pub dropped_orphan_tool_result_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PreparedModelInput {
    pub original_messages: Vec<Message>,
    pub sanitized_messages: Vec<Message>,
    pub tool_chain_report: ToolChainReport,
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
//  消息规范化（协议共享）
// ═══════════════════════════════════════════════════════════════

/// 协议无关的工具链规范化：
/// - 丢弃没有前置 tool_use 的孤立 tool_result
/// - 丢弃没有完整 tool_result 闭环的 assistant tool_use
/// - 保留普通文本/图片等业务内容
pub fn prepare_model_input(messages: &[Message]) -> PreparedModelInput {
    let report = analyze_tool_chain(messages);
    let sanitized_messages = sanitize_tool_chain_messages(messages, &report);
    PreparedModelInput {
        original_messages: messages.to_vec(),
        sanitized_messages,
        tool_chain_report: report,
    }
}

pub fn analyze_tool_chain(messages: &[Message]) -> ToolChainReport {
    let mut complete_tool_call_ids = BTreeSet::new();
    let mut dropped_incomplete_tool_call_ids = BTreeSet::new();
    let mut dropped_orphan_tool_result_ids = BTreeSet::new();

    for (idx, msg) in messages.iter().enumerate() {
        match msg.role {
            Role::Assistant => {
                let tool_call_ids = assistant_tool_call_ids(Some(msg));
                if tool_call_ids.is_empty() {
                    continue;
                }

                if assistant_tool_calls_are_complete(messages, idx) {
                    complete_tool_call_ids.extend(tool_call_ids);
                } else {
                    dropped_incomplete_tool_call_ids.extend(tool_call_ids);
                }
            }
            Role::User => {
                for block in &msg.content {
                    if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                        if !tool_result_has_matching_call(messages, idx, tool_use_id) {
                            dropped_orphan_tool_result_ids.insert(tool_use_id.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let mut report = ToolChainReport {
        original_message_count: messages.len(),
        sanitized_message_count: 0,
        complete_tool_call_ids: complete_tool_call_ids.into_iter().collect(),
        dropped_incomplete_tool_call_ids: dropped_incomplete_tool_call_ids.into_iter().collect(),
        dropped_orphan_tool_result_ids: dropped_orphan_tool_result_ids.into_iter().collect(),
    };

    report.sanitized_message_count = sanitize_tool_chain_messages(messages, &report).len();
    report
}

pub fn sanitize_tool_chain_messages(messages: &[Message], report: &ToolChainReport) -> Vec<Message> {
    let mut sanitized = Vec::new();
    let complete_ids: HashSet<&str> = report.complete_tool_call_ids.iter().map(String::as_str).collect();

    for msg in messages {
        let content: Vec<ContentBlock> = msg.content.iter().filter_map(|c| {
            if let ContentBlock::ToolUse { id, .. } = c {
                if complete_ids.contains(id.as_str()) {
                    Some(c.clone())
                } else {
                    None
                }
            } else if let ContentBlock::ToolResult { tool_use_id, .. } = c {
                if complete_ids.contains(tool_use_id.as_str()) {
                    Some(c.clone())
                } else {
                    None
                }
            } else {
                Some(c.clone())
            }
        }).collect();

        if content.is_empty() {
            continue;
        }

        sanitized.push(Message {
            content,
            ..msg.clone()
        });
    }

    sanitized
}

/// Claude 协议发送前规范化：
/// - 先走共享的工具链规范化
/// - 再过滤 Claude 不需要回传的思维块
/// - Context Seed 在 Core 内部表现为 Role::System，但 Claude Messages API 的 messages[] 只允许 user/assistant，
///   所以这里把 seed 合并到 system 参数，禁止落进 messages[]。
pub fn normalize_for_claude(input: &PreparedModelInput) -> (String, Vec<Message>) {
    let mut extra_system_parts = Vec::new();
    let messages = input.sanitized_messages.clone().into_iter().filter_map(|msg| {
        let mut content = Vec::new();
        for c in &msg.content {
            match c {
                ContentBlock::Thinking { .. } => {}
                ContentBlock::Text { text, .. } if matches!(msg.role, Role::System) => {
                    if !text.trim().is_empty() {
                        extra_system_parts.push(text.clone());
                    }
                }
                _ if matches!(msg.role, Role::System) => {}
                _ => content.push(c.clone()),
            }
        }

        if matches!(msg.role, Role::System) || content.is_empty() {
            None
        } else {
            Some(Message { content, ..msg })
        }
    }).collect();

    (extra_system_parts.join("\n\n"), messages)
}

/// OpenAI 适配器的消息转换（tool_use → tool_calls，tool_result → role:"tool"）
pub fn convert_for_openai(input: &PreparedModelInput) -> Vec<serde_json::Value> {
    let mut result = Vec::new();

    for msg in &input.sanitized_messages {
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

                if !text_parts.is_empty() || !tool_calls.is_empty() {
                    let mut m = serde_json::json!({ "role": "assistant" });
                    if !text_parts.is_empty() {
                        m["content"] = serde_json::Value::String(text_parts.join("\n"));
                    }
                    if !tool_calls.is_empty() {
                        m["tool_calls"] = serde_json::Value::Array(tool_calls);
                    }
                    result.push(m);
                }
            }
            Role::User => {
                let mut content_parts = Vec::new();
                let mut tool_results = Vec::new();
                let mut has_multimodal = false;
                for c in &msg.content {
                    match c {
                        ContentBlock::Text { text, .. } => content_parts.push(serde_json::json!({"type": "text", "text": text})),
                        ContentBlock::Image { source } => {
                            has_multimodal = true;
                            content_parts.push(serde_json::json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:{};base64,{}", source.media_type, source.data)
                                }
                            }));
                        }
                        ContentBlock::Audio { source } => {
                            has_multimodal = true;
                            content_parts.push(serde_json::json!({
                                "type": "input_audio",
                                "input_audio": {
                                    "data": source.data,
                                    "format": source.format
                                }
                            }));
                        }
                        ContentBlock::ToolResult { tool_use_id, content, .. } => {
                            tool_results.push((tool_use_id.clone(), content.clone().unwrap_or_default()));
                        }
                        _ => {}
                    }
                }

                for (tool_use_id, content) in tool_results {
                    result.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": content
                    }));
                }

                if !content_parts.is_empty() {
                    if has_multimodal || content_parts.len() > 1 {
                        // 有图片/音频或多段内容 → 用数组格式
                        result.push(serde_json::json!({
                            "role": "user",
                            "content": content_parts
                        }));
                    } else if content_parts[0]["type"] == "text" {
                        // 纯文本 → 简单字符串格式
                        result.push(serde_json::json!({
                            "role": "user",
                            "content": content_parts[0]["text"]
                        }));
                    }
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

fn assistant_tool_calls_are_complete(messages: &[Message], assistant_index: usize) -> bool {
    let tool_call_ids = assistant_tool_call_ids(messages.get(assistant_index));
    if tool_call_ids.is_empty() {
        return false;
    }

    let mut remaining = tool_call_ids;
    for msg in messages.iter().skip(assistant_index + 1) {
        match msg.role {
            Role::User => {
                let mut has_non_tool_content = false;
                for block in &msg.content {
                    match block {
                        ContentBlock::ToolResult { tool_use_id, .. } => {
                            remaining.remove(tool_use_id);
                        }
                        ContentBlock::Text { text, .. } if !text.is_empty() => {
                            has_non_tool_content = true;
                        }
                        ContentBlock::Image { .. } | ContentBlock::Thinking { .. } | ContentBlock::ToolUse { .. } => {
                            has_non_tool_content = true;
                        }
                        _ => {}
                    }
                }

                if remaining.is_empty() {
                    return true;
                }

                if has_non_tool_content {
                    return false;
                }
            }
            _ => return false,
        }
    }

    false
}

fn tool_result_has_matching_call(messages: &[Message], user_index: usize, tool_use_id: &str) -> bool {
    for idx in (0..user_index).rev() {
        let Some(msg) = messages.get(idx) else {
            continue;
        };
        match msg.role {
            Role::Assistant => {
                let tool_call_ids = assistant_tool_call_ids(Some(msg));
                if tool_call_ids.contains(tool_use_id) {
                    return assistant_tool_calls_are_complete(messages, idx);
                }
                if !tool_call_ids.is_empty() {
                    return false;
                }
                if msg.content.iter().any(|block| matches!(block, ContentBlock::Text { text, .. } if !text.is_empty())) {
                    return false;
                }
            }
            Role::User => {
                let has_non_tool_content = msg.content.iter().any(|block| {
                    !matches!(block, ContentBlock::ToolResult { .. })
                });
                if has_non_tool_content {
                    return false;
                }
            }
            _ => return false,
        }
    }

    false
}

fn assistant_tool_call_ids(message: Option<&Message>) -> HashSet<String> {
    let Some(message) = message else {
        return HashSet::new();
    };
    if message.role != Role::Assistant {
        return HashSet::new();
    }

    message.content.iter().filter_map(|block| {
        if let ContentBlock::ToolUse { id, .. } = block {
            Some(id.clone())
        } else {
            None
        }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_tool_message(session_id: &str, call_id: &str) -> Message {
        Message::new(
            session_id,
            Role::Assistant,
            vec![ContentBlock::tool_use(call_id, "AskUserQuestion", serde_json::json!({}))],
        )
    }

    fn user_tool_result_message(session_id: &str, call_id: &str) -> Message {
        Message::new(
            session_id,
            Role::User,
            vec![ContentBlock::tool_result(call_id, "result", false)],
        )
    }

    #[test]
    fn sanitize_tool_chain_preserves_complete_chain() {
        let session_id = "sess";
        let messages = vec![
            assistant_tool_message(session_id, "call_1"),
            user_tool_result_message(session_id, "call_1"),
        ];

        let prepared = prepare_model_input(&messages);
        let sanitized = prepared.sanitized_messages;

        assert_eq!(sanitized.len(), 2);
        assert!(matches!(sanitized[0].content[0], ContentBlock::ToolUse { .. }));
        assert!(matches!(sanitized[1].content[0], ContentBlock::ToolResult { .. }));
    }

    #[test]
    fn convert_for_openai_drops_orphan_tool_results() {
        let session_id = "sess";
        let messages = vec![
            user_tool_result_message(session_id, "orphan_call"),
            Message::new(session_id, Role::Assistant, vec![ContentBlock::text("hello")]),
            assistant_tool_message(session_id, "call_1"),
            user_tool_result_message(session_id, "call_1"),
        ];

        let prepared = prepare_model_input(&messages);
        let converted = convert_for_openai(&prepared);

        assert_eq!(converted.len(), 3);
        assert_eq!(converted[0]["role"], "assistant");
        assert_eq!(converted[1]["role"], "assistant");
        assert_eq!(converted[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(converted[2]["role"], "tool");
        assert_eq!(converted[2]["tool_call_id"], "call_1");
        assert_eq!(prepared.tool_chain_report.dropped_orphan_tool_result_ids, vec!["orphan_call"]);
    }

    #[test]
    fn convert_for_openai_drops_incomplete_tool_call_chain_after_cancel() {
        let session_id = "sess";
        let messages = vec![
            Message::new(
                session_id,
                Role::Assistant,
                vec![
                    ContentBlock::Text { text: "let me call tools".into(), reasoning_content: None },
                    ContentBlock::tool_use("call_1", "AskUserQuestion", serde_json::json!({})),
                    ContentBlock::tool_use("call_2", "AskUserQuestion", serde_json::json!({})),
                ],
            ),
            Message::new(session_id, Role::User, vec![ContentBlock::text("我刚刚取消了")]),
        ];

        let prepared = prepare_model_input(&messages);
        let converted = convert_for_openai(&prepared);

        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["role"], "assistant");
        assert_eq!(converted[0]["content"], "let me call tools");
        assert!(converted[0].get("tool_calls").is_none());
        assert_eq!(converted[1]["role"], "user");
        assert_eq!(
            prepared.tool_chain_report.dropped_incomplete_tool_call_ids,
            vec!["call_1".to_string(), "call_2".to_string()]
        );
    }

    #[test]
    fn normalize_for_claude_keeps_complete_tool_chain() {
        let session_id = "sess";
        let messages = vec![
            assistant_tool_message(session_id, "call_1"),
            user_tool_result_message(session_id, "call_1"),
        ];

        let prepared = prepare_model_input(&messages);
        let (extra_system, normalized) = normalize_for_claude(&prepared);

        assert!(extra_system.is_empty());
        assert_eq!(normalized.len(), 2);
        assert!(matches!(normalized[0].content[0], ContentBlock::ToolUse { .. }));
        assert!(matches!(normalized[1].content[0], ContentBlock::ToolResult { .. }));
    }

    #[test]
    fn normalize_for_claude_drops_incomplete_tool_chain() {
        let session_id = "sess";
        let messages = vec![
            assistant_tool_message(session_id, "call_1"),
            Message::new(session_id, Role::User, vec![ContentBlock::text("继续")]),
        ];

        let prepared = prepare_model_input(&messages);
        let (extra_system, normalized) = normalize_for_claude(&prepared);

        assert!(extra_system.is_empty());
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].role, Role::User);
        assert!(matches!(normalized[0].content[0], ContentBlock::Text { .. }));
    }

    #[test]
    fn normalize_for_claude_moves_system_seed_out_of_messages() {
        let session_id = "sess";
        let messages = vec![
            Message::new(session_id, Role::System, vec![ContentBlock::text("历史摘要")]),
            Message::new(session_id, Role::User, vec![ContentBlock::text("继续")]),
        ];

        let prepared = prepare_model_input(&messages);
        let (extra_system, normalized) = normalize_for_claude(&prepared);

        assert_eq!(extra_system, "历史摘要");
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].role, Role::User);
    }
}

/// 错误重试判断
pub fn should_retry(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("x-should-retry") && lower.contains("true")
        || lower.contains("rate limit")
        || lower.contains("429")
        || lower.contains("408")
        || lower.contains("409")
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("529")
        || lower.contains("overloaded")
        || lower.contains("overloaded_error")
        || lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("connection closed")
        || lower.contains("connection aborted")
        || lower.contains("broken pipe")
        || lower.contains("network")
        || lower.contains("connect error")
        || lower.contains("error sending request")
        || lower.contains("error decoding response body")
        || lower.contains("incomplete message")
        || lower.contains("unexpected eof")
        || lower.contains("eof")
}

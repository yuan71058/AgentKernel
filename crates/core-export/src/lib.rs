//! # Core Export
//!
//! 导出层。Core 运行依赖 SQLite，AI 读取依赖 exports。
//! 支持 JSONL / Markdown 导出。

use core_protocol::*;

/// 导出格式
pub enum ExportFormat {
    Jsonl,
    Markdown,
    Json,
}

/// 将消息导出为 JSONL
pub fn messages_to_jsonl(messages: &[Message]) -> String {
    messages.iter()
        .filter_map(|m| serde_json::to_string(m).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 将消息导出为 Markdown 对话格式
pub fn messages_to_markdown(messages: &[Message]) -> String {
    let mut out = String::from("# Conversation\n\n");
    for msg in messages {
        let role = match msg.role {
            Role::User => "**User**",
            Role::Assistant => "**Assistant**",
            Role::System => "**System**",
            Role::Tool => "**Tool**",
        };
        let text: String = msg.content.iter().filter_map(|c| {
            match c {
                ContentBlock::Text { text, .. } => Some(text.clone()),
                ContentBlock::ToolResult { content, .. } => content.clone(),
                ContentBlock::ToolUse { name, input, .. } => {
                    Some(format!("[tool_use: {}({})]", name, input))
                }
                _ => None,
            }
        }).collect::<Vec<_>>()
        .join("\n");
        if !text.is_empty() {
            out.push_str(&format!("## {}\n\n{}\n\n", role, text));
        }
    }
    out
}

/// 将事件导出为 JSONL
pub fn events_to_jsonl(events: &[EventEnvelope]) -> String {
    events.iter()
        .filter_map(|e| serde_json::to_string(e).ok())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Context Preview 导出
pub fn context_preview(messages: &[Message], seeds: &[ContextSeed]) -> String {
    let mut out = String::from("# Context Preview\n\n");
    if !seeds.is_empty() {
        out.push_str("## Seeds\n\n");
        for seed in seeds {
            out.push_str(&format!("- **{}** ({}): {}\n", seed.kind, seed.seed_id, seed.content));
        }
        out.push('\n');
    }
    out.push_str("## Messages\n\n");
    for msg in messages {
        let role = format!("{:?}", msg.role);
        let text: String = msg.content.iter().filter_map(|c| {
            if let ContentBlock::Text { text, .. } = c { Some(text.as_str()) } else { None }
        }).collect();
        out.push_str(&format!("[{}] {}\n\n", role, text));
    }
    out
}

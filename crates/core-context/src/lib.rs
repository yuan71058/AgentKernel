//! # Core Context
//!
//! Context 是"上下文视图"，不是历史本身。
//! Message 永久保留，Context 决定哪些消息参与模型上下文。
//!
//! 职责：
//! - 管理 ContextState（当前上下文规则）
//! - 构建提交给模型的上下文视图
//! - 管理 Context Seed（记忆块）
//! - 阈值检测，触发压缩事件

use core_events::EventBus;
use core_protocol::*;
use std::collections::HashSet;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// 上下文管理器
pub struct ContextManager {
    /// 所有消息（永久保存）: session_id -> messages
    messages: RwLock<HashMap<String, Vec<Message>>>,
    /// 活跃 Context State: session_id -> ContextState
    contexts: RwLock<HashMap<String, ContextState>>,
    /// Context Seeds: session_id -> seeds
    seeds: RwLock<HashMap<String, Vec<ContextSeed>>>,
    /// Token 阈值百分比（0-100），达到此比例触发 context.threshold.reached
    compress_threshold_percent: u8,
    /// 系统 token 窗口
    context_window_tokens: u64,
    event_bus: Arc<EventBus>,
}

impl ContextManager {
    pub fn new(
        context_window_tokens: u64,
        compress_threshold_percent: u8,
        event_bus: Arc<EventBus>,
    ) -> Self {
        Self {
            messages: RwLock::new(HashMap::new()),
            contexts: RwLock::new(HashMap::new()),
            seeds: RwLock::new(HashMap::new()),
            compress_threshold_percent,
            context_window_tokens,
            event_bus,
        }
    }

    /// 追加消息（永久保存）
    pub fn add_message(&self, session_id: &str, message: Message) {
        let mut msgs = self.messages.write().unwrap();
        msgs.entry(session_id.to_string())
            .or_insert_with(Vec::new)
            .push(message);
    }

    /// 获取全部消息（用于全量历史读取）
    pub fn get_all_messages(&self, session_id: &str) -> Vec<Message> {
        let msgs = self.messages.read().unwrap();
        msgs.get(session_id).cloned().unwrap_or_default()
    }

    /// 从存储层加载消息到内存（按需加载 session 时使用）
    pub fn load_messages(&self, session_id: &str, messages: Vec<Message>) {
        self.messages.write().unwrap().insert(session_id.to_string(), messages);
    }

    /// 从存储层加载 ContextState 到内存
    pub fn load_context_state(&self, session_id: &str, ctx: Option<ContextState>) {
        let mut contexts = self.contexts.write().unwrap();
        if let Some(ctx) = ctx {
            contexts.insert(session_id.to_string(), ctx);
        } else {
            contexts.remove(session_id);
        }
    }

    /// 从存储层加载 ContextSeed 到内存
    pub fn load_seeds(&self, session_id: &str, seeds: Vec<ContextSeed>) {
        self.seeds.write().unwrap().insert(session_id.to_string(), seeds);
    }

    /// 构建上下文视图：根据 ContextState 规则，返回提交给模型的消息列表
    pub fn build_context_view(&self, session_id: &str) -> Vec<Message> {
        let all = self.get_all_messages(session_id);
        let contexts = self.contexts.read().unwrap();

        if let Some(ctx) = contexts.get(session_id) {
            self.apply_rules(&all, &ctx.rules)
        } else {
            all
        }
    }

    /// 构建完整模型输入：system prompts + context seeds + recent messages
    pub fn build_model_input(&self, session_id: &str) -> Vec<Message> {
        let mut result = Vec::new();

        // 1. Context Seeds 作为 system 消息
        let seeds = self.seeds.read().unwrap();
        if let Some(session_seeds) = seeds.get(session_id) {
            for seed in session_seeds.iter().filter(|s| s.enabled) {
                result.push(Message {
                    message_id: seed.seed_id.clone(),
                    session_id: session_id.to_string(),
                    run_id: String::new(),
                    role: Role::System,
                    kind: MessageKind::ContextSeed,
                    content: vec![ContentBlock::text(&seed.content)],
                    token_estimate: 0,
                    created_at: chrono::Utc::now(),
                    metadata: HashMap::new(),
                });
            }
        }

        // 2. 上下文视图中的消息
        let view = self.build_context_view(session_id);
        result.extend(view);
        result
    }

    /// 阈值检测：当前 token 占比是否超过阈值
    pub fn should_compress(&self, session_id: &str) -> bool {
        if self.compress_threshold_percent == 0 {
            return false;
        }
        let estimated = self.estimate_tokens(session_id);
        let threshold = self.context_window_tokens * self.compress_threshold_percent as u64 / 100;
        estimated > threshold
    }

    /// 估算当前 session 的 token 数
    pub fn estimate_tokens(&self, session_id: &str) -> u64 {
        let msgs = self.messages.read().unwrap();
        msgs.get(session_id)
            .map(|m| m.iter().map(|msg| msg.token_estimate).sum())
            .unwrap_or(0)
    }

    /// 获取统计信息
    pub fn stats(&self, session_id: &str) -> ContextStats {
        let msgs = self.messages.read().unwrap();
        let messages = msgs.get(session_id);
        let count = messages.map(|m| m.len()).unwrap_or(0);
        let tokens = messages.map(|m| m.iter().map(|msg| msg.token_estimate).sum()).unwrap_or(0);
        ContextStats {
            message_count: count as u64,
            estimated_tokens: tokens,
            window_tokens: self.context_window_tokens,
            usage_percent: if self.context_window_tokens > 0 {
                (tokens as f64 / self.context_window_tokens as f64 * 100.0) as u8
            } else {
                0
            },
        }
    }

    /// 获取当前 ContextState
    pub fn get_context(&self, session_id: &str) -> Option<ContextState> {
        self.contexts.read().unwrap().get(session_id).cloned()
    }

    /// 生成默认 ContextState（full mode）
    pub fn default_context_state(&self, session_id: &str) -> ContextState {
        ContextState {
            context_id: format!("ctx_{}", uuid::Uuid::new_v4()),
            session_id: session_id.to_string(),
            mode: ContextMode::Full,
            rules: ContextRules::default(),
            created_at: chrono::Utc::now(),
        }
    }

    /// 设置 ContextState
    pub fn set_context(&self, session_id: &str, ctx: ContextState) {
        let mut contexts = self.contexts.write().unwrap();
        contexts.insert(session_id.to_string(), ctx);
    }

    /// 重置 ContextState 为 full mode
    pub fn reset_context(&self, session_id: &str) -> ContextState {
        let ctx = self.default_context_state(session_id);
        self.set_context(session_id, ctx.clone());
        ctx
    }

    /// 排除一段消息范围
    pub fn exclude_range(&self, session_id: &str, start_message_id: &str, end_message_id: &str) -> ContextState {
        let mut ctx = self.get_context(session_id)
            .unwrap_or_else(|| self.default_context_state(session_id));
        ctx.mode = ContextMode::Sliding;
        ctx.rules.exclude_ranges.push((start_message_id.to_string(), end_message_id.to_string()));
        ctx.created_at = chrono::Utc::now();
        self.set_context(session_id, ctx.clone());
        ctx
    }

    /// 只保留某条消息之后的上下文
    pub fn include_after(&self, session_id: &str, message_id: &str) -> ContextState {
        let mut ctx = self.get_context(session_id)
            .unwrap_or_else(|| self.default_context_state(session_id));
        ctx.mode = ContextMode::Sliding;
        ctx.rules.include_after_message_id = Some(message_id.to_string());
        ctx.created_at = chrono::Utc::now();
        self.set_context(session_id, ctx.clone());
        ctx
    }

    /// 设置最近消息窗口
    pub fn set_keep_recent(&self, session_id: &str, keep: Option<u64>) -> ContextState {
        let mut ctx = self.get_context(session_id)
            .unwrap_or_else(|| self.default_context_state(session_id));
        ctx.mode = if keep.is_some() { ContextMode::Sliding } else { ContextMode::Full };
        ctx.rules.keep_recent_messages = keep;
        ctx.created_at = chrono::Utc::now();
        self.set_context(session_id, ctx.clone());
        ctx
    }

    /// 应用压缩摘要：写入 summary seed，并切换到 compacted context
    pub fn apply_compaction(&self, session_id: &str, summary: String, include_after_message_id: Option<String>) -> (ContextState, ContextSeed) {
        let seed = ContextSeed {
            seed_id: format!("seed_{}", uuid::Uuid::new_v4()),
            session_id: session_id.to_string(),
            kind: SeedKind::CompactionSummary,
            content: summary,
            enabled: true,
            priority: 100,
        };
        self.add_seed(session_id, seed.clone());
        let ctx = ContextState {
            context_id: format!("ctx_{}", uuid::Uuid::new_v4()),
            session_id: session_id.to_string(),
            mode: ContextMode::Compacted,
            rules: ContextRules {
                include_after_message_id,
                exclude_ranges: Vec::new(),
                keep_recent_messages: None,
                base_seed_ids: vec![seed.seed_id.clone()],
            },
            created_at: chrono::Utc::now(),
        };
        self.set_context(session_id, ctx.clone());
        (ctx, seed)
    }

    /// 添加 Context Seed
    pub fn add_seed(&self, session_id: &str, seed: ContextSeed) {
        let mut seeds = self.seeds.write().unwrap();
        seeds.entry(session_id.to_string())
            .or_insert_with(Vec::new)
            .push(seed);
    }

    /// 获取 session 的所有 seeds（用于导出/预览）
    pub fn get_seeds(&self, session_id: &str) -> Vec<ContextSeed> {
        let seeds = self.seeds.read().unwrap();
        seeds.get(session_id).cloned().unwrap_or_default()
    }

    /// 清除 session 所有状态
    pub fn remove_session(&self, session_id: &str) {
        self.messages.write().unwrap().remove(session_id);
        self.contexts.write().unwrap().remove(session_id);
        self.seeds.write().unwrap().remove(session_id);
    }

    fn apply_rules(&self, messages: &[Message], rules: &ContextRules) -> Vec<Message> {
        let mut result: Vec<Message> = messages.to_vec();

        // include_after_message_id 过滤
        if let Some(ref after_id) = rules.include_after_message_id {
            if let Some(pos) = result.iter().position(|m| &m.message_id == after_id) {
                result = result[pos + 1..].to_vec();
            }
        }

        // exclude_ranges 过滤
        for (start_id, end_id) in &rules.exclude_ranges {
            let start = result.iter().position(|m| &m.message_id == start_id);
            let end = result.iter().position(|m| &m.message_id == end_id);
            if let (Some(s), Some(e)) = (start, end) {
                if s <= e {
                    result.drain(s..=e);
                }
            }
        }

        // keep_recent_messages
        if let Some(keep) = rules.keep_recent_messages {
            let keep = keep as usize;
            if result.len() > keep {
                let start = Self::normalize_keep_recent_start(&result, result.len() - keep);
                result = result[start..].to_vec();
            }
        }

        result
    }

    fn normalize_keep_recent_start(messages: &[Message], mut start: usize) -> usize {
        loop {
            let Some(tool_result_ids) = Self::tool_result_ids(messages.get(start)) else {
                break;
            };
            if tool_result_ids.is_empty() {
                break;
            }

            let mut matched_index = None;
            for idx in (0..start).rev() {
                let tool_use_ids = Self::tool_use_ids(messages.get(idx));
                if !tool_use_ids.is_empty() && tool_result_ids.is_subset(&tool_use_ids) {
                    matched_index = Some(idx);
                    break;
                }
            }

            match matched_index {
                Some(idx) if idx < start => start = idx,
                _ => break,
            }
        }

        start
    }

    fn tool_result_ids(message: Option<&Message>) -> Option<HashSet<String>> {
        let message = message?;
        if message.role != Role::User {
            return None;
        }

        let ids: HashSet<String> = message.content.iter().filter_map(|block| {
            if let ContentBlock::ToolResult { tool_use_id, .. } = block {
                Some(tool_use_id.clone())
            } else {
                None
            }
        }).collect();

        Some(ids)
    }

    fn tool_use_ids(message: Option<&Message>) -> HashSet<String> {
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
}

#[derive(Debug, Clone)]
pub struct ContextStats {
    pub message_count: u64,
    pub estimated_tokens: u64,
    pub window_tokens: u64,
    pub usage_percent: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_text(session_id: &str, text: &str) -> Message {
        Message::new(session_id, Role::User, vec![ContentBlock::text(text)])
    }

    fn assistant_text(session_id: &str, text: &str) -> Message {
        Message::new(session_id, Role::Assistant, vec![ContentBlock::text(text)])
    }

    fn assistant_tool(session_id: &str, id: &str) -> Message {
        Message::new(
            session_id,
            Role::Assistant,
            vec![ContentBlock::tool_use(id, "AskUserQuestion", serde_json::json!({}))],
        )
    }

    fn user_tool_result(session_id: &str, id: &str) -> Message {
        Message::new(
            session_id,
            Role::User,
            vec![ContentBlock::tool_result(id, "ok", false)],
        )
    }

    #[test]
    fn keep_recent_expands_to_include_matching_tool_call() {
        let event_bus = Arc::new(EventBus::new(16));
        let manager = ContextManager::new(128_000, 80, event_bus);
        let session_id = "sess";

        let messages = vec![
            user_text(session_id, "older"),
            assistant_tool(session_id, "call_1"),
            user_tool_result(session_id, "call_1"),
            assistant_text(session_id, "after tool"),
            user_text(session_id, "continue"),
            assistant_tool(session_id, "call_2"),
            user_tool_result(session_id, "call_2"),
        ];

        let rules = ContextRules {
            keep_recent_messages: Some(5),
            ..Default::default()
        };

        let view = manager.apply_rules(&messages, &rules);

        assert_eq!(view.len(), 6);
        assert!(matches!(view[0].content[0], ContentBlock::ToolUse { .. }));
        assert!(matches!(view[1].content[0], ContentBlock::ToolResult { .. }));
    }
}

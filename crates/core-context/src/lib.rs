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

    /// 构建完整模型输入：enabled seeds + 上下文消息
    pub fn build_model_input(&self, session_id: &str) -> Vec<Message> {
        let mut result = Vec::new();

        // 1. Context Seeds — 独立的动态前置上下文块，不受消息裁剪规则影响
        let seeds = self.seeds.read().unwrap();
        if let Some(session_seeds) = seeds.get(session_id) {
            let mut enabled_seeds: Vec<_> = session_seeds.iter().filter(|s| s.enabled).collect();
            enabled_seeds.sort_by_key(|s| s.priority);
            for seed in enabled_seeds {
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

    /// 设置主裁剪策略；keep_recent_messages / include_after / checkpoint 互斥。
    pub fn set_trim_policy(&self, session_id: &str, policy: TrimPolicy) -> ContextState {
        let mut ctx = self.get_context(session_id)
            .unwrap_or_else(|| self.default_context_state(session_id));
        ctx.mode = if matches!(policy.mode, TrimMode::None) { ContextMode::Full } else { ContextMode::Sliding };
        ctx.rules.trim = policy;
        ctx.created_at = chrono::Utc::now();
        self.set_context(session_id, ctx.clone());
        ctx
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

    /// 删除指定 Context Seed
    pub fn delete_seed(&self, session_id: &str, seed_id: &str) -> Option<ContextSeed> {
        let mut seeds = self.seeds.write().unwrap();
        let session_seeds = seeds.get_mut(session_id)?;
        let pos = session_seeds.iter().position(|s| s.seed_id == seed_id)?;
        Some(session_seeds.remove(pos))
    }

    /// 按 kind 清空 Context Seeds；kind 为空则清空全部 seeds
    pub fn clear_seeds(&self, session_id: &str, kind: Option<SeedKind>) -> Vec<ContextSeed> {
        let mut seeds = self.seeds.write().unwrap();
        let session_seeds = seeds.entry(session_id.to_string()).or_insert_with(Vec::new);
        if let Some(kind) = kind {
            let mut removed = Vec::new();
            session_seeds.retain(|seed| {
                if seed.kind == kind {
                    removed.push(seed.clone());
                    false
                } else {
                    true
                }
            });
            removed
        } else {
            std::mem::take(session_seeds)
        }
    }

    /// 按 kind 覆盖写入 Context Seed
    pub fn set_seed_by_kind(&self, session_id: &str, seed: ContextSeed) -> Vec<ContextSeed> {
        let mut seeds = self.seeds.write().unwrap();
        let session_seeds = seeds.entry(session_id.to_string()).or_insert_with(Vec::new);
        let mut removed = Vec::new();
        session_seeds.retain(|existing| {
            if existing.kind == seed.kind {
                removed.push(existing.clone());
                false
            } else {
                true
            }
        });
        session_seeds.push(seed);
        removed
    }

    /// 清除 session 所有状态
    pub fn remove_session(&self, session_id: &str) {
        self.messages.write().unwrap().remove(session_id);
        self.contexts.write().unwrap().remove(session_id);
        self.seeds.write().unwrap().remove(session_id);
    }

    fn apply_rules(&self, messages: &[Message], rules: &ContextRules) -> Vec<Message> {
        let mut result: Vec<Message> = messages.to_vec();

        match rules.trim.mode {
            TrimMode::None => {}
            TrimMode::IncludeAfter => {
                if let Some(ref after_id) = rules.trim.message_id {
                    if let Some(pos) = result.iter().position(|m| &m.message_id == after_id) {
                        result = result[pos + 1..].to_vec();
                    }
                }
            }
            TrimMode::KeepRecentMessages => {
                if let Some(keep) = rules.trim.keep_messages {
                    let keep = keep as usize;
                    if result.len() > keep {
                        let start = Self::normalize_keep_recent_start(&result, result.len() - keep);
                        result = result[start..].to_vec();
                    }
                }
            }
            TrimMode::Checkpoint => {
                if let Some(ref after_id) = rules.trim.applied_after_message_id {
                    if let Some(pos) = result.iter().position(|m| &m.message_id == after_id) {
                        result = result[pos + 1..].to_vec();
                    }
                }
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

        result
    }

    /// 如果 checkpoint 达到阈值，按最近 N 个用户 turn 找到新的裁剪起点。
    /// 返回 Some(new_context) 表示策略发生了实际应用。
    pub fn apply_checkpoint_trim_if_needed(&self, session_id: &str) -> Option<ContextState> {
        let mut ctx = self.get_context(session_id)?;
        if ctx.rules.trim.mode != TrimMode::Checkpoint {
            return None;
        }
        let threshold = ctx.rules.trim.trigger_max_context_messages? as usize;
        let retain_turns = ctx.rules.trim.retain_recent_turns? as usize;
        if threshold == 0 || retain_turns == 0 {
            return None;
        }

        let all = self.get_all_messages(session_id);
        let active = self.apply_rules(&all, &ctx.rules);
        if active.len() <= threshold {
            return None;
        }

        let after_id = Self::checkpoint_after_message_id(&active, retain_turns)?;
        if ctx.rules.trim.applied_after_message_id.as_deref() == Some(after_id.as_str()) {
            return None;
        }

        ctx.mode = ContextMode::Sliding;
        ctx.rules.trim.applied_after_message_id = Some(after_id);
        ctx.created_at = chrono::Utc::now();
        self.set_context(session_id, ctx.clone());
        Some(ctx)
    }

    fn checkpoint_after_message_id(messages: &[Message], retain_turns: usize) -> Option<String> {
        let mut user_seen = 0usize;
        let mut first_retained_user_idx = None;

        for (idx, message) in messages.iter().enumerate().rev() {
            if message.role == Role::User && !Self::message_is_tool_result(message) {
                user_seen += 1;
                first_retained_user_idx = Some(idx);
                if user_seen >= retain_turns {
                    break;
                }
            }
        }

        let first_idx = first_retained_user_idx?;
        if first_idx == 0 {
            return None;
        }
        Some(messages[first_idx - 1].message_id.clone())
    }

    fn message_is_tool_result(message: &Message) -> bool {
        message.content.iter().any(|block| matches!(block, ContentBlock::ToolResult { .. }))
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
            trim: TrimPolicy {
                mode: TrimMode::KeepRecentMessages,
                keep_messages: Some(5),
                ..Default::default()
            },
            ..Default::default()
        };

        let view = manager.apply_rules(&messages, &rules);

        assert_eq!(view.len(), 6);
        assert!(matches!(view[0].content[0], ContentBlock::ToolUse { .. }));
        assert!(matches!(view[1].content[0], ContentBlock::ToolResult { .. }));
    }
}

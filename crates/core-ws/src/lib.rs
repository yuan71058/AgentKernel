//! # Core WS
//!
//! WebSocket 通讯层 — AI Runtime 协议。
//!
//! 核心原则：
//! - 所有操作都是 Command（客户端 → 服务端）
//! - 所有变化都是 Event（服务端 → 客户端）
//! - 每条消息显式携带 session_id / run_id / trace_id

pub mod types;
pub mod server;

use core_protocol::EventEnvelope;

/// WS 消息类型
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsMessage {
    /// 命令：客户端 → 服务端
    Command {
        command: String,
        request_id: String,
        session_id: String,
        payload: serde_json::Value,
    },
    /// 事件：服务端 → 客户端
    Event(EventEnvelope),
    /// 响应：服务端 → 客户端
    Response {
        request_id: String,
        success: bool,
        payload: serde_json::Value,
    },
    /// 流式数据：服务端 → 客户端
    Stream {
        session_id: String,
        run_id: String,
        event: String,
        data: serde_json::Value,
    },
}

/// WS 命令常量
pub mod commands {
    // ─── Tool ──────────────────────────────
    pub const REGISTER_TOOL: &str = "tool.register";
    pub const UNREGISTER_TOOL: &str = "tool.unregister";

    // ─── Provider ──────────────────────────
    pub const UPDATE_PROVIDER: &str = "provider.update";
    pub const GET_PROVIDER: &str = "provider.get";

    // ─── Session（session_id 非空时作用于该 session）──
    pub const SEND_MESSAGE: &str = "session.send";
    pub const GET_SESSION: &str = "session.get";
    pub const SESSION_INFO: &str = "session.info";
    pub const SESSION_DELETE: &str = "session.delete";
    pub const SESSION_CLEAR: &str = "session.clear";
    pub const SESSION_MESSAGES: &str = "session.messages";
    pub const CANCEL_RUN: &str = "run.cancel";
    pub const RUNTIME_SESSIONS: &str = "runtime.sessions";

    // ─── 系统级（session_id 为空或忽略）─────────
    pub const LIST_SESSIONS: &str = "session.list";
    pub const SYSTEM_STATS: &str = "system.stats";

    // ─── Context ───────────────────────────
    pub const CONTEXT_PREVIEW: &str = "context.preview";
    pub const CONTEXT_RESET: &str = "context.reset";
    pub const CONTEXT_EXCLUDE: &str = "context.exclude";
    pub const CONTEXT_INCLUDE_AFTER: &str = "context.include_after";
    pub const CONTEXT_KEEP_RECENT: &str = "context.keep_recent";
    pub const CONTEXT_SEED_ADD: &str = "context.seed.add";
    pub const COMPACTION_APPLY: &str = "context.compaction.apply";

    // ─── Events（断线补拉 + 订阅）──────────
    pub const EVENTS_PULL: &str = "events.pull";
    pub const EVENTS_SUBSCRIBE: &str = "events.subscribe";

    // ─── System Prompt ─────────────────────
    pub const SYSTEM_PROMPT_GET: &str = "system_prompt.get";
    pub const SYSTEM_PROMPT_SET: &str = "system_prompt.set";

    // ─── Tool List ─────────────────────────
    pub const TOOL_LIST: &str = "tool.list";
    pub const TOOL_GET: &str = "tool.get";
}

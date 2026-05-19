//! # Core Events
//!
//! 事件驱动系统。所有状态变化都发 Event。
//! 基于 tokio::broadcast 实现多订阅者事件总线。
//! 带 EventLog：按 session 记录事件日志，支持断线补拉。

use core_protocol::EventEnvelope;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;
use tracing::debug;

/// 事件类型常量
pub mod event_types {
    pub const SESSION_CREATED: &str = "session.created";
    pub const SESSION_CLOSED: &str = "session.closed";
    pub const RUN_STARTED: &str = "run.started";
    pub const RUN_CANCELLED: &str = "run.cancelled";
    pub const RUN_COMPLETED: &str = "run.completed";
    pub const MODEL_DELTA: &str = "model.delta";
    pub const MODEL_COMPLETED: &str = "model.completed";
    pub const TOOL_CALL_REQUEST: &str = "tool.call.request";
    pub const TOOL_CALL_RESULT: &str = "tool.call.result";
    pub const TOOL_CALL_ERROR: &str = "tool.call.error";
    pub const TOOL_REGISTERED: &str = "tool.registered";
    pub const CONTEXT_THRESHOLD_REACHED: &str = "context.threshold.reached";
    pub const CONTEXT_COMPACTION_APPLIED: &str = "context.compaction.applied";
    pub const PROMPT_ATTACHED: &str = "prompt.attached";
    pub const ERROR: &str = "error";
}

// ═══════════════════════════════════════════════════════════════
//  EventLog — 按 session 的事件日志（支持断线补拉）
// ═══════════════════════════════════════════════════════════════

/// 事件日志：按 session_id 存储，每个 session 有递增的 seq
pub struct EventLog {
    /// session_id -> (events, next_seq)
    inner: RwLock<HashMap<String, (Vec<EventEnvelope>, u64)>>,
    /// 每个 session 最多保留的事件数
    capacity: usize,
}

impl EventLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
            capacity,
        }
    }

    /// 记录事件并分配 seq
    pub fn record(&self, event: &mut EventEnvelope) -> u64 {
        let mut log = self.inner.write().unwrap();
        let entry = log.entry(event.session_id.clone())
            .or_insert_with(|| (Vec::new(), 1));
        let seq = entry.1;
        event.event_seq = Some(seq);
        entry.1 = seq + 1;
        entry.0.push(event.clone());
        // 滚动清理
        if entry.0.len() > self.capacity {
            let drain_count = entry.0.len() - self.capacity;
            entry.0.drain(0..drain_count);
        }
        seq
    }

    /// 拉取指定 session 中 seq > since_seq 的所有事件
    pub fn pull_since(&self, session_id: &str, since_seq: u64) -> Vec<EventEnvelope> {
        let log = self.inner.read().unwrap();
        log.get(session_id)
            .map(|(events, _)| {
                events.iter()
                    .filter(|e| e.event_seq.unwrap_or(0) > since_seq)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 获取指定 session 当前最大 seq
    pub fn current_seq(&self, session_id: &str) -> u64 {
        let log = self.inner.read().unwrap();
        log.get(session_id)
            .map(|(_, next)| next - 1)
            .unwrap_or(0)
    }

    /// 获取指定 session 的事件总数
    pub fn event_count(&self, session_id: &str) -> usize {
        let log = self.inner.read().unwrap();
        log.get(session_id)
            .map(|(events, _)| events.len())
            .unwrap_or(0)
    }
}

// ═══════════════════════════════════════════════════════════════
//  EventBus — 广播 + 日志
// ═══════════════════════════════════════════════════════════════

/// 事件总线，基于 tokio::broadcast，支持多订阅者 + 事件日志
pub struct EventBus {
    tx: broadcast::Sender<EventEnvelope>,
    pub event_log: Arc<EventLog>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            event_log: Arc::new(EventLog::new(10000)),
        }
    }

    /// 发布事件：自动分配 seq 并记录到日志，然后广播
    pub fn emit(&self, mut event: EventEnvelope) {
        // 分配 seq 并记录
        let seq = self.event_log.record(&mut event);
        debug!(
            event_type = %event.event_type,
            session_id = %event.session_id,
            seq = seq,
            "emit event"
        );
        // 广播（忽略无订阅者）
        if self.tx.send(event).is_err() {
            // 无订阅者，静默忽略
        }
    }

    /// 创建订阅者
    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.tx.subscribe()
    }

    /// 拉取指定 session 中 seq > since_seq 的事件（断线补拉）
    pub fn pull_since(&self, session_id: &str, since_seq: u64) -> Vec<EventEnvelope> {
        self.event_log.pull_since(session_id, since_seq)
    }

    /// 获取指定 session 当前最大 seq
    pub fn current_seq(&self, session_id: &str) -> u64 {
        self.event_log.current_seq(session_id)
    }
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            event_log: self.event_log.clone(),
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

/// 事件过滤器，用于订阅时筛选特定事件类型
pub struct EventFilter {
    pub event_types: Vec<String>,
    pub session_id: Option<String>,
}

impl EventFilter {
    pub fn matches(&self, event: &EventEnvelope) -> bool {
        if !self.event_types.is_empty() && !self.event_types.contains(&event.event_type) {
            return false;
        }
        if let Some(ref sid) = self.session_id {
            if &event.session_id != sid {
                return false;
            }
        }
        true
    }
}

/// 便捷宏：构建 EventEnvelope
#[macro_export]
macro_rules! event {
    ($type:expr, $session:expr) => {
        core_protocol::EventEnvelope::new($type, $session)
    };
    ($type:expr, $session:expr, $payload:expr) => {
        core_protocol::EventEnvelope::new($type, $session).with_payload($payload)
    };
}

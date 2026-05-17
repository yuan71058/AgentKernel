//! # Core Storage
//!
//! 持久化层。SQLite 主存储 + JSONL 日志。
//! 写操作必须走 Core API，AI 只读导出文件。

use core_protocol::*;
use std::path::PathBuf;

/// 存储配置
#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub data_dir: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from(".aicore"),
        }
    }
}

/// 存储层 trait（可替换实现：SQLite / PostgreSQL / 内存）
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    // Message
    async fn save_message(&self, msg: &Message) -> Result<(), String>;
    async fn get_messages(&self, session_id: &str) -> Result<Vec<Message>, String>;
    async fn get_messages_after(&self, session_id: &str, after_id: &str) -> Result<Vec<Message>, String>;

    // Session
    async fn save_session(&self, session: &Session) -> Result<(), String>;
    async fn get_session(&self, session_id: &str) -> Result<Option<Session>, String>;

    // Run
    async fn save_run(&self, run: &Run) -> Result<(), String>;
    async fn get_runs(&self, session_id: &str) -> Result<Vec<Run>, String>;

    // ContextState
    async fn save_context_state(&self, ctx: &ContextState) -> Result<(), String>;
    async fn get_context_state(&self, session_id: &str) -> Result<Option<ContextState>, String>;

    // ContextSeed
    async fn save_seed(&self, seed: &ContextSeed) -> Result<(), String>;
    async fn get_seeds(&self, session_id: &str) -> Result<Vec<ContextSeed>, String>;

    // JSONL 事件日志
    async fn append_event_log(&self, event: &EventEnvelope) -> Result<(), String>;
}

/// 内存存储（测试用）
pub struct MemoryStorage {
    messages: std::sync::RwLock<std::collections::HashMap<String, Vec<Message>>>,
    sessions: std::sync::RwLock<std::collections::HashMap<String, Session>>,
    runs: std::sync::RwLock<std::collections::HashMap<String, Vec<Run>>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            messages: std::sync::RwLock::new(std::collections::HashMap::new()),
            sessions: std::sync::RwLock::new(std::collections::HashMap::new()),
            runs: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self { Self::new() }
}

#[async_trait::async_trait]
impl Storage for MemoryStorage {
    async fn save_message(&self, msg: &Message) -> Result<(), String> {
        let mut msgs = self.messages.write().unwrap();
        msgs.entry(msg.session_id.clone()).or_insert_with(Vec::new).push(msg.clone());
        Ok(())
    }

    async fn get_messages(&self, session_id: &str) -> Result<Vec<Message>, String> {
        let msgs = self.messages.read().unwrap();
        Ok(msgs.get(session_id).cloned().unwrap_or_default())
    }

    async fn get_messages_after(&self, session_id: &str, after_id: &str) -> Result<Vec<Message>, String> {
        let msgs = self.messages.read().unwrap();
        let all = msgs.get(session_id).cloned().unwrap_or_default();
        if let Some(pos) = all.iter().position(|m| m.message_id == after_id) {
            Ok(all[pos + 1..].to_vec())
        } else {
            Ok(all)
        }
    }

    async fn save_session(&self, session: &Session) -> Result<(), String> {
        self.sessions.write().unwrap().insert(session.session_id.clone(), session.clone());
        Ok(())
    }

    async fn get_session(&self, session_id: &str) -> Result<Option<Session>, String> {
        Ok(self.sessions.read().unwrap().get(session_id).cloned())
    }

    async fn save_run(&self, run: &Run) -> Result<(), String> {
        self.runs.write().unwrap()
            .entry(run.session_id.clone())
            .or_insert_with(Vec::new)
            .push(run.clone());
        Ok(())
    }

    async fn get_runs(&self, session_id: &str) -> Result<Vec<Run>, String> {
        Ok(self.runs.read().unwrap().get(session_id).cloned().unwrap_or_default())
    }

    async fn save_context_state(&self, _ctx: &ContextState) -> Result<(), String> { Ok(()) }
    async fn get_context_state(&self, _session_id: &str) -> Result<Option<ContextState>, String> { Ok(None) }
    async fn save_seed(&self, _seed: &ContextSeed) -> Result<(), String> { Ok(()) }
    async fn get_seeds(&self, _session_id: &str) -> Result<Vec<ContextSeed>, String> { Ok(vec![]) }
    async fn append_event_log(&self, _event: &EventEnvelope) -> Result<(), String> { Ok(()) }
}

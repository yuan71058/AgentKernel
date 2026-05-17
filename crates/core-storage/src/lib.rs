//! # Core Storage
//!
//! 持久化层。SQLite 主存储 + JSONL 日志。
//! 写操作必须走 Core API，AI 只读导出文件。

use core_protocol::*;
use std::path::{Path, PathBuf};

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
    async fn list_sessions(&self) -> Result<Vec<Session>, String> { Ok(vec![]) }

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

/// 文件存储（阶段性实现）
///
/// 目录结构：
/// .aicore/sessions/<session_id>/
///   session.json        Session 配置/metadata
///   messages.jsonl      全量消息日志（追加）
///   runs.jsonl          Run 日志（追加）
///   events.jsonl        Event 日志（追加）
///   context_state.json  当前 ContextState
///   seeds.jsonl         ContextSeed 日志（追加）
pub struct FileStorage {
    root: PathBuf,
}

impl FileStorage {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self { root: data_dir.into() }
    }

    pub fn default_project() -> Self {
        Self::new(".aicore")
    }

    fn sessions_dir(&self) -> PathBuf { self.root.join("sessions") }
    fn session_dir(&self, session_id: &str) -> PathBuf { self.sessions_dir().join(sanitize_id(session_id)) }
    fn session_file(&self, session_id: &str) -> PathBuf { self.session_dir(session_id).join("session.json") }
    fn messages_file(&self, session_id: &str) -> PathBuf { self.session_dir(session_id).join("messages.jsonl") }
    fn runs_file(&self, session_id: &str) -> PathBuf { self.session_dir(session_id).join("runs.jsonl") }
    fn events_file(&self, session_id: &str) -> PathBuf { self.session_dir(session_id).join("events.jsonl") }
    fn context_file(&self, session_id: &str) -> PathBuf { self.session_dir(session_id).join("context_state.json") }
    fn seeds_file(&self, session_id: &str) -> PathBuf { self.session_dir(session_id).join("seeds.jsonl") }

    async fn ensure_session_dir(&self, session_id: &str) -> Result<(), String> {
        tokio::fs::create_dir_all(self.session_dir(session_id)).await.map_err(|e| e.to_string())
    }

    async fn write_json<T: serde::Serialize>(&self, path: &Path, value: &T) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
        tokio::fs::write(path, data).await.map_err(|e| e.to_string())
    }

    async fn read_json<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Result<Option<T>, String> {
        match tokio::fs::read(path).await {
            Ok(data) => serde_json::from_slice(&data).map(Some).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn append_jsonl<T: serde::Serialize>(&self, path: &Path, value: &T) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }
        let mut line = serde_json::to_string(value).map_err(|e| e.to_string())?;
        line.push('\n');
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
            .map_err(|e| e.to_string())?;
        file.write_all(line.as_bytes()).await.map_err(|e| e.to_string())
    }

    async fn read_jsonl<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Result<Vec<T>, String> {
        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.to_string()),
        };
        let mut out = Vec::new();
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            out.push(serde_json::from_str(line).map_err(|e| e.to_string())?);
        }
        Ok(out)
    }
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' { c } else { '_' })
        .collect()
}

#[async_trait::async_trait]
impl Storage for FileStorage {
    async fn save_message(&self, msg: &Message) -> Result<(), String> {
        self.ensure_session_dir(&msg.session_id).await?;
        self.append_jsonl(&self.messages_file(&msg.session_id), msg).await
    }

    async fn get_messages(&self, session_id: &str) -> Result<Vec<Message>, String> {
        self.read_jsonl(&self.messages_file(session_id)).await
    }

    async fn get_messages_after(&self, session_id: &str, after_id: &str) -> Result<Vec<Message>, String> {
        let all: Vec<Message> = self.get_messages(session_id).await?;
        if let Some(pos) = all.iter().position(|m| m.message_id == after_id) {
            Ok(all[pos + 1..].to_vec())
        } else {
            Ok(all)
        }
    }

    async fn save_session(&self, session: &Session) -> Result<(), String> {
        self.ensure_session_dir(&session.session_id).await?;
        self.write_json(&self.session_file(&session.session_id), session).await
    }

    async fn get_session(&self, session_id: &str) -> Result<Option<Session>, String> {
        self.read_json(&self.session_file(session_id)).await
    }

    async fn list_sessions(&self) -> Result<Vec<Session>, String> {
        let mut out = Vec::new();
        let mut entries = match tokio::fs::read_dir(self.sessions_dir()).await {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.to_string()),
        };
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            let path = entry.path().join("session.json");
            if let Some(session) = self.read_json::<Session>(&path).await? {
                out.push(session);
            }
        }
        Ok(out)
    }

    async fn save_run(&self, run: &Run) -> Result<(), String> {
        self.ensure_session_dir(&run.session_id).await?;
        self.append_jsonl(&self.runs_file(&run.session_id), run).await
    }

    async fn get_runs(&self, session_id: &str) -> Result<Vec<Run>, String> {
        self.read_jsonl(&self.runs_file(session_id)).await
    }

    async fn save_context_state(&self, ctx: &ContextState) -> Result<(), String> {
        self.ensure_session_dir(&ctx.session_id).await?;
        self.write_json(&self.context_file(&ctx.session_id), ctx).await
    }

    async fn get_context_state(&self, session_id: &str) -> Result<Option<ContextState>, String> {
        self.read_json(&self.context_file(session_id)).await
    }

    async fn save_seed(&self, seed: &ContextSeed) -> Result<(), String> {
        self.ensure_session_dir(&seed.session_id).await?;
        self.append_jsonl(&self.seeds_file(&seed.session_id), seed).await
    }

    async fn get_seeds(&self, session_id: &str) -> Result<Vec<ContextSeed>, String> {
        self.read_jsonl(&self.seeds_file(session_id)).await
    }

    async fn append_event_log(&self, event: &EventEnvelope) -> Result<(), String> {
        if event.session_id.is_empty() {
            return Ok(());
        }
        self.ensure_session_dir(&event.session_id).await?;
        self.append_jsonl(&self.events_file(&event.session_id), event).await
    }
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

    async fn list_sessions(&self) -> Result<Vec<Session>, String> {
        Ok(self.sessions.read().unwrap().values().cloned().collect())
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

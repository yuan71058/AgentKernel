//! # Core Storage
//!
//! 持久化层。SQLite 主存储 + JSONL 日志。
//! 写操作必须走 Core API，AI 只读导出文件。

use core_protocol::*;
use std::collections::HashMap;
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
    async fn save_seeds(&self, session_id: &str, seeds: &[ContextSeed]) -> Result<(), String>;
    async fn get_seeds(&self, session_id: &str) -> Result<Vec<ContextSeed>, String>;

    // JSONL 事件日志
    async fn append_event_log(&self, event: &EventEnvelope) -> Result<(), String>;

    // Session Fork：将源 session 数据复制到新 session
    async fn copy_session_data(&self, _src_session_id: &str, _dst_session_id: &str) -> Result<(), String> {
        Err("copy_session_data not implemented".to_string())
    }

    // Session Delete：永久删除 session 持久化数据
    async fn delete_session_data(&self, _session_id: &str) -> Result<(), String> {
        Err("delete_session_data not implemented".to_string())
    }
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

    async fn save_seeds(&self, session_id: &str, seeds: &[ContextSeed]) -> Result<(), String> {
        self.ensure_session_dir(session_id).await?;
        let path = self.seeds_file(session_id);
        if seeds.is_empty() {
            tokio::fs::write(path, "").await.map_err(|e| e.to_string())?;
            return Ok(());
        }
        let mut content = String::new();
        for seed in seeds {
            let mut line = serde_json::to_string(seed).map_err(|e| e.to_string())?;
            line.push('\n');
            content.push_str(&line);
        }
        tokio::fs::write(path, content).await.map_err(|e| e.to_string())
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

    async fn copy_session_data(&self, src_session_id: &str, dst_session_id: &str) -> Result<(), String> {
        // 1. 复制 session.json，更新 session_id 和时间戳
        let mut session: Session = self.read_json(&self.session_file(src_session_id)).await?
            .ok_or_else(|| format!("source session '{}' not found", src_session_id))?;
        session.session_id = dst_session_id.to_string();
        session.active_context_id = String::new();
        session.created_at = chrono::Utc::now();
        session.updated_at = chrono::Utc::now();
        session.status = SessionStatus::Active;
        self.ensure_session_dir(dst_session_id).await?;
        self.write_json(&self.session_file(dst_session_id), &session).await?;

        // 2. 复制 messages（保留原 message_id，更新 session_id）
        let messages: Vec<Message> = self.get_messages(src_session_id).await?;
        if !messages.is_empty() {
            let mut lines = String::new();
            for mut msg in messages {
                msg.session_id = dst_session_id.to_string();
                lines.push_str(&serde_json::to_string(&msg).map_err(|e| e.to_string())?);
                lines.push('\n');
            }
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true).write(true).truncate(true)
                .open(&self.messages_file(dst_session_id)).await.map_err(|e| e.to_string())?;
            file.write_all(lines.as_bytes()).await.map_err(|e| e.to_string())?;
        }

        // 3. 复制 seeds，生成新 seed_id，记录映射关系（旧→新）
        let seeds: Vec<ContextSeed> = self.get_seeds(src_session_id).await?;
        let mut seed_id_map: HashMap<String, String> = HashMap::new();
        if !seeds.is_empty() {
            let mut lines = String::new();
            for mut seed in seeds {
                let old_id = seed.seed_id.clone();
                seed.seed_id = format!("seed_{}", uuid::Uuid::new_v4());
                seed.session_id = dst_session_id.to_string();
                seed_id_map.insert(old_id, seed.seed_id.clone());
                lines.push_str(&serde_json::to_string(&seed).map_err(|e| e.to_string())?);
                lines.push('\n');
            }
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true).write(true).truncate(true)
                .open(&self.seeds_file(dst_session_id)).await.map_err(|e| e.to_string())?;
            file.write_all(lines.as_bytes()).await.map_err(|e| e.to_string())?;
        }

        // 4. 复制 context_state，生成新 context_id，remap base_seed_ids
        if let Some(mut ctx) = self.get_context_state(src_session_id).await? {
            ctx.context_id = format!("ctx_{}", uuid::Uuid::new_v4());
            ctx.session_id = dst_session_id.to_string();
            ctx.created_at = chrono::Utc::now();
            // 将 base_seed_ids 中的旧 seed_id 映射为新 seed_id
            ctx.rules.base_seed_ids = ctx.rules.base_seed_ids.iter()
                .filter_map(|old_id| seed_id_map.get(old_id).cloned())
                .collect();
            self.write_json(&self.context_file(dst_session_id), &ctx).await?;
        }

        // 5. 复制 runs，更新 session_id
        let runs: Vec<Run> = self.get_runs(src_session_id).await?;
        if !runs.is_empty() {
            let mut lines = String::new();
            for mut run in runs {
                run.session_id = dst_session_id.to_string();
                lines.push_str(&serde_json::to_string(&run).map_err(|e| e.to_string())?);
                lines.push('\n');
            }
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true).write(true).truncate(true)
                .open(&self.runs_file(dst_session_id)).await.map_err(|e| e.to_string())?;
            file.write_all(lines.as_bytes()).await.map_err(|e| e.to_string())?;
        }

        // 6. 不复制 events（新 session 从零开始）

        Ok(())
    }

    async fn delete_session_data(&self, session_id: &str) -> Result<(), String> {
        let dir = self.session_dir(session_id);
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
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
    async fn save_seeds(&self, _session_id: &str, _seeds: &[ContextSeed]) -> Result<(), String> { Ok(()) }
    async fn get_seeds(&self, _session_id: &str) -> Result<Vec<ContextSeed>, String> { Ok(vec![]) }
    async fn append_event_log(&self, _event: &EventEnvelope) -> Result<(), String> { Ok(()) }

    async fn copy_session_data(&self, src_session_id: &str, dst_session_id: &str) -> Result<(), String> {
        // 复制 session
        {
            let sessions = self.sessions.read().unwrap();
            if let Some(mut session) = sessions.get(src_session_id).cloned() {
                session.session_id = dst_session_id.to_string();
                session.created_at = chrono::Utc::now();
                session.updated_at = chrono::Utc::now();
                self.sessions.write().unwrap().insert(dst_session_id.to_string(), session);
            } else {
                return Err(format!("source session '{}' not found", src_session_id));
            }
        }
        // 复制 messages
        {
            let msgs = self.messages.read().unwrap();
            if let Some(src_msgs) = msgs.get(src_session_id) {
                let cloned: Vec<Message> = src_msgs.iter().map(|m| {
                    let mut c = m.clone();
                    c.session_id = dst_session_id.to_string();
                    c
                }).collect();
                self.messages.write().unwrap().insert(dst_session_id.to_string(), cloned);
            }
        }
        // 复制 runs
        {
            let runs = self.runs.read().unwrap();
            if let Some(src_runs) = runs.get(src_session_id) {
                let cloned: Vec<Run> = src_runs.iter().map(|r| {
                    let mut c = r.clone();
                    c.session_id = dst_session_id.to_string();
                    c
                }).collect();
                self.runs.write().unwrap().insert(dst_session_id.to_string(), cloned);
            }
        }
        Ok(())
    }

    async fn delete_session_data(&self, session_id: &str) -> Result<(), String> {
        self.sessions.write().unwrap().remove(session_id);
        self.messages.write().unwrap().remove(session_id);
        self.runs.write().unwrap().remove(session_id);
        Ok(())
    }
}

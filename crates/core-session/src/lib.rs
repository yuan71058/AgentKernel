//! # Core Session
//!
//! Session 是最核心的对象，代表一个独立 AI 会话运行环境。
//! 支持：全量历史保留、Context 动态切换、Prompt 动态附加、Tool 动态启用。
//! Provider 配置存储在 session metadata 中，随 session 持久化。

use core_events::EventBus;
use core_events::event_types::*;
use core_protocol::*;
use core_storage::Storage;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;

/// Session 管理器
pub struct SessionManager {
    sessions: std::sync::RwLock<HashMap<String, Session>>,
    /// 内存缓存的 provider override（从 session metadata 恢复）
    provider_overrides: std::sync::RwLock<HashMap<String, ProviderConfig>>,
    storage: Arc<dyn Storage>,
    event_bus: Arc<EventBus>,
}

/// session metadata 中 provider config 的 key
const META_KEY_PROVIDER: &str = "provider_config";
/// session metadata 中 system prompt 的 key
const META_KEY_SYSTEM_PROMPT: &str = "system_prompt";
/// session metadata 中注册工具列表的 key
const META_KEY_TOOLS: &str = "tools";

impl SessionManager {
    pub fn new(storage: Arc<dyn Storage>, event_bus: Arc<EventBus>) -> Self {
        Self {
            sessions: std::sync::RwLock::new(HashMap::new()),
            provider_overrides: std::sync::RwLock::new(HashMap::new()),
            storage,
            event_bus,
        }
    }

    // ─── Provider Override（持久化到 session metadata）──────

    /// 设置 session 级供应商配置覆盖（写入 metadata + 持久化 + 内存缓存）
    pub async fn set_provider_override(&self, session_id: &str, config: ProviderConfig) -> Result<(), String> {
        // 1. 写入内存缓存
        self.provider_overrides.write().unwrap()
            .insert(session_id.to_string(), config.clone());

        // 2. 写入 session metadata（先 clone 再 drop lock，避免跨 await 持锁）
        let updated_session = {
            let mut sessions = self.sessions.write().unwrap();
            if let Some(session) = sessions.get_mut(session_id) {
                session.metadata.insert(
                    META_KEY_PROVIDER.to_string(),
                    serde_json::to_value(&config).unwrap_or(serde_json::Value::Null),
                );
                session.updated_at = chrono::Utc::now();
                Some(session.clone())
            } else {
                None
            }
        }; // RwLockWriteGuard dropped here

        // 3. 持久化（不持锁）
        if let Some(session) = updated_session {
            self.storage.save_session(&session).await?;
        }

        info!(session_id = %session_id, model = %config.model, "provider override saved");
        Ok(())
    }

    /// 获取 session 级供应商配置覆盖（内存缓存 → session metadata 兜底）
    pub fn get_provider_override(&self, session_id: &str) -> Option<ProviderConfig> {
        // 1. 先查内存缓存
        if let Some(cfg) = self.provider_overrides.read().unwrap().get(session_id) {
            return Some(cfg.clone());
        }
        // 2. 从 session metadata 恢复
        let sessions = self.sessions.read().unwrap();
        if let Some(session) = sessions.get(session_id) {
            if let Some(val) = session.metadata.get(META_KEY_PROVIDER) {
                if let Ok(cfg) = serde_json::from_value::<ProviderConfig>(val.clone()) {
                    // 回填内存缓存
                    self.provider_overrides.write().unwrap()
                        .insert(session_id.to_string(), cfg.clone());
                    return Some(cfg);
                }
            }
        }
        None
    }

    /// 清除 session 级供应商配置覆盖（从 metadata 移除 + 持久化）
    pub async fn clear_provider_override(&self, session_id: &str) -> Result<(), String> {
        self.provider_overrides.write().unwrap().remove(session_id);

        let updated_session = {
            let mut sessions = self.sessions.write().unwrap();
            if let Some(session) = sessions.get_mut(session_id) {
                session.metadata.remove(META_KEY_PROVIDER);
                session.updated_at = chrono::Utc::now();
                Some(session.clone())
            } else {
                None
            }
        };

        if let Some(session) = updated_session {
            self.storage.save_session(&session).await?;
        }
        Ok(())
    }

    // ─── Session System Prompt（持久化到 session metadata）──────

    /// 设置 session 级 system prompt（写入 metadata + 持久化）
    pub async fn set_system_prompt(&self, session_id: &str, prompt: String) -> Result<(), String> {
        let updated_session = {
            let mut sessions = self.sessions.write().unwrap();
            if let Some(session) = sessions.get_mut(session_id) {
                session.metadata.insert(
                    META_KEY_SYSTEM_PROMPT.to_string(),
                    serde_json::Value::String(prompt),
                );
                session.updated_at = chrono::Utc::now();
                Some(session.clone())
            } else {
                None
            }
        };

        if let Some(session) = updated_session {
            self.storage.save_session(&session).await?;
            Ok(())
        } else {
            Err(format!("session '{}' not found", session_id))
        }
    }

    /// 获取 session 级 system prompt
    pub fn get_system_prompt(&self, session_id: &str) -> Option<String> {
        self.sessions.read().unwrap()
            .get(session_id)
            .and_then(|s| s.metadata.get(META_KEY_SYSTEM_PROMPT))
            .and_then(|v| v.as_str().map(String::from))
    }

    // ─── Session Tools（持久化到 session metadata）──────

    /// 设置 session 已注册工具列表快照（定义 + 注册信息）
    pub async fn set_session_tools(
        &self,
        session_id: &str,
        tools: serde_json::Value,
    ) -> Result<(), String> {
        let updated_session = {
            let mut sessions = self.sessions.write().unwrap();
            if let Some(session) = sessions.get_mut(session_id) {
                session.metadata.insert(META_KEY_TOOLS.to_string(), tools);
                session.updated_at = chrono::Utc::now();
                Some(session.clone())
            } else {
                None
            }
        };

        if let Some(session) = updated_session {
            self.storage.save_session(&session).await?;
            Ok(())
        } else {
            Err(format!("session '{}' not found", session_id))
        }
    }

    /// 获取 session 已注册工具列表快照
    pub fn get_session_tools(&self, session_id: &str) -> Option<serde_json::Value> {
        self.sessions.read().unwrap()
            .get(session_id)
            .and_then(|s| s.metadata.get(META_KEY_TOOLS).cloned())
    }

    // ─── Session 生命周期 ──────────────────────────────────

    /// 创建 Session
    pub async fn create(&self, session_type: SessionType, title: &str) -> Result<Session, String> {
        let session = Session {
            session_id: format!("sess_{}", uuid::Uuid::new_v4()),
            session_type,
            title: title.to_string(),
            active_context_id: String::new(),
            status: SessionStatus::Active,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: HashMap::new(),
        };
        self.storage.save_session(&session).await?;
        self.sessions.write().unwrap().insert(session.session_id.clone(), session.clone());
        self.event_bus.emit(core_protocol::EventEnvelope::new(SESSION_CREATED, &session.session_id)
            .with_payload(serde_json::json!({"title": title})));
        info!(session_id = %session.session_id, "session created");
        Ok(session)
    }

    /// 获取或自动创建 session（WS 场景：客户端直接用 session_id 发消息）
    pub async fn get_or_create(&self, session_id: &str) -> Result<Session, String> {
        // 先查内存
        if let Some(s) = self.sessions.read().unwrap().get(session_id) {
            return Ok(s.clone());
        }
        // 自动创建
        let session = Session {
            session_id: session_id.to_string(),
            session_type: SessionType::Chat,
            title: session_id.to_string(),
            active_context_id: String::new(),
            status: SessionStatus::Active,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: HashMap::new(),
        };
        self.storage.save_session(&session).await?;
        self.sessions.write().unwrap().insert(session_id.to_string(), session.clone());
        self.event_bus.emit(core_protocol::EventEnvelope::new(SESSION_CREATED, session_id)
            .with_payload(serde_json::json!({"auto_created": true})));
        info!(session_id = %session_id, "session auto-created");
        Ok(session)
    }

    /// 从存储层加载 Session 到内存索引（启动加载 session 列表时使用）
    pub fn load_session(&self, session: Session) {
        self.sessions.write().unwrap().insert(session.session_id.clone(), session);
    }

    /// 获取 Session
    pub fn get(&self, session_id: &str) -> Option<Session> {
        self.sessions.read().unwrap().get(session_id).cloned()
    }

    /// 关闭 Session
    pub async fn close(&self, session_id: &str) -> Result<(), String> {
        let updated_session = {
            let mut sessions = self.sessions.write().unwrap();
            if let Some(session) = sessions.get_mut(session_id) {
                session.status = SessionStatus::Closed;
                session.updated_at = chrono::Utc::now();
                Some(session.clone())
            } else {
                None
            }
        };
        if let Some(session) = updated_session {
            self.storage.save_session(&session).await?;
            self.event_bus.emit(core_protocol::EventEnvelope::new(SESSION_CLOSED, session_id));
        }
        Ok(())
    }

    /// 归档 Session（保留所有持久化数据，仅通过状态从默认列表隐藏）
    pub async fn archive(&self, session_id: &str) -> Result<Session, String> {
        let mut session = self.get_or_load_session(session_id).await?;
        session.status = SessionStatus::Archived;
        session.updated_at = chrono::Utc::now();
        session.metadata.insert(
            "archived_at".to_string(),
            serde_json::Value::String(session.updated_at.to_rfc3339()),
        );
        self.storage.save_session(&session).await?;
        self.sessions.write().unwrap().insert(session_id.to_string(), session.clone());
        self.event_bus.emit(core_protocol::EventEnvelope::new(SESSION_ARCHIVED, session_id)
            .with_payload(serde_json::json!({"archived_at": session.updated_at.to_rfc3339()})));
        info!(session_id = %session_id, "session archived");
        Ok(session)
    }

    /// 取消归档 Session（恢复为 closed，不自动启动运行）
    pub async fn unarchive(&self, session_id: &str) -> Result<Session, String> {
        let mut session = self.get_or_load_session(session_id).await?;
        session.status = SessionStatus::Closed;
        session.updated_at = chrono::Utc::now();
        session.metadata.remove("archived_at");
        self.storage.save_session(&session).await?;
        self.sessions.write().unwrap().insert(session_id.to_string(), session.clone());
        self.event_bus.emit(core_protocol::EventEnvelope::new(SESSION_UNARCHIVED, session_id));
        info!(session_id = %session_id, "session unarchived");
        Ok(session)
    }

    async fn get_or_load_session(&self, session_id: &str) -> Result<Session, String> {
        if let Some(session) = self.sessions.read().unwrap().get(session_id).cloned() {
            return Ok(session);
        }
        if let Some(session) = self.storage.get_session(session_id).await? {
            self.sessions.write().unwrap().insert(session_id.to_string(), session.clone());
            return Ok(session);
        }
        Err(format!("session '{}' not found", session_id))
    }

    /// Fork Session：将源 session 数据复制到新 session
    /// 要求源 session 存在，目标 session_id 不存在
    pub async fn fork(&self, src_session_id: &str, dst_session_id: &str) -> Result<Session, String> {
        // 1. 校验源 session 存在
        if self.sessions.read().unwrap().get(src_session_id).is_none() {
            // 尝试从 storage 加载
            if let Some(session) = self.storage.get_session(src_session_id).await? {
                self.sessions.write().unwrap().insert(session.session_id.clone(), session);
            } else {
                return Err(format!("source session '{}' not found", src_session_id));
            }
        }

        // 2. 校验目标 session 不存在
        if self.sessions.read().unwrap().contains_key(dst_session_id) {
            return Err(format!("destination session '{}' already exists", dst_session_id));
        }
        if self.storage.get_session(dst_session_id).await?.is_some() {
            return Err(format!("destination session '{}' already exists in storage", dst_session_id));
        }

        // 3. 通过 storage 复制数据
        self.storage.copy_session_data(src_session_id, dst_session_id).await?;

        // 4. 加载新 session 到内存
        let new_session = self.storage.get_session(dst_session_id).await?
            .ok_or("fork succeeded but new session not found in storage")?;

        // 5. provider override 也复制一份
        if let Some(cfg) = self.get_provider_override(src_session_id) {
            self.provider_overrides.write().unwrap()
                .insert(dst_session_id.to_string(), cfg);
        }

        self.sessions.write().unwrap().insert(dst_session_id.to_string(), new_session.clone());
        self.event_bus.emit(core_protocol::EventEnvelope::new(SESSION_CREATED, dst_session_id)
            .with_payload(serde_json::json!({"forked_from": src_session_id})));
        info!(src = %src_session_id, dst = %dst_session_id, "session forked");
        Ok(new_session)
    }

    /// 创建 Run
    pub async fn create_run(&self, session_id: &str, provider: &str, model: &str) -> Result<Run, String> {
        let run = Run {
            run_id: format!("run_{}", uuid::Uuid::new_v4()),
            session_id: session_id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            status: RunStatus::Running,
            started_at: chrono::Utc::now(),
            completed_at: None,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
        };
        self.storage.save_run(&run).await?;
        self.event_bus.emit(core_protocol::EventEnvelope::new(RUN_STARTED, session_id)
            .with_run_id(&run.run_id)
            .with_payload(serde_json::json!({"provider": provider, "model": model})));
        Ok(run)
    }

    // ─── 查询 ────────────────────────────────────────────

    pub fn list_sessions(&self) -> Vec<Session> {
        self.sessions.read().unwrap().values().cloned().collect()
    }

    /// 分页查询 session 列表
    pub fn list_sessions_paged(&self, page: u32, limit: u32, status: Option<&str>) -> (Vec<Session>, u64) {
        let sessions = self.sessions.read().unwrap();
        let mut all: Vec<&Session> = sessions.values().collect();

        if let Some(s) = status {
            let target = match s {
                "active" => SessionStatus::Active,
                "paused" => SessionStatus::Paused,
                "closed" => SessionStatus::Closed,
                "archived" => SessionStatus::Archived,
                _ => return (Vec::new(), 0),
            };
            all.retain(|s| s.status == target);
        } else {
            all.retain(|s| s.status != SessionStatus::Archived);
        }

        all.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let total = all.len() as u64;
        let offset = (page as usize) * (limit as usize);
        let paged: Vec<Session> = all.into_iter()
            .skip(offset)
            .take(limit as usize)
            .cloned()
            .collect();

        (paged, total)
    }

    /// 关闭 Session（更新状态并从内存运行索引移除，storage 保留历史）
    pub async fn close_and_unload(&self, session_id: &str) -> Result<bool, String> {
        let mut session = self.get_or_load_session(session_id).await?;
        session.status = SessionStatus::Closed;
        session.updated_at = chrono::Utc::now();
        self.storage.save_session(&session).await?;
        let removed = self.sessions.write().unwrap().remove(session_id).is_some();
        self.provider_overrides.write().unwrap().remove(session_id);
        self.event_bus.emit(core_protocol::EventEnvelope::new(SESSION_CLOSED, session_id)
            .with_payload(serde_json::json!({"reason": "closed"})));
        info!(session_id = %session_id, "session closed and unloaded");
        Ok(removed)
    }

    /// 永久删除 Session（删除持久化目录 + 内存索引）
    pub async fn delete_permanently(&self, session_id: &str) -> Result<bool, String> {
        let existed = self.sessions.write().unwrap().remove(session_id).is_some()
            || self.storage.get_session(session_id).await?.is_some();
        self.provider_overrides.write().unwrap().remove(session_id);
        self.storage.delete_session_data(session_id).await?;
        self.event_bus.emit(core_protocol::EventEnvelope::new(SESSION_DELETED, session_id)
            .with_payload(serde_json::json!({"permanent": true})));
        info!(session_id = %session_id, "session permanently deleted");
        Ok(existed)
    }

    /// 删除 Session（从内存中移除，storage 保留历史）
    pub fn remove_session(&self, session_id: &str) -> bool {
        let removed = self.sessions.write().unwrap().remove(session_id).is_some();
        self.provider_overrides.write().unwrap().remove(session_id);
        if removed {
            self.event_bus.emit(core_protocol::EventEnvelope::new(SESSION_CLOSED, session_id)
                .with_payload(serde_json::json!({"reason": "removed_from_runtime"})));
            info!(session_id = %session_id, "session removed from runtime");
        }
        removed
    }

    /// 获取 session 总数
    pub fn session_count(&self) -> u64 {
        self.sessions.read().unwrap().len() as u64
    }
}

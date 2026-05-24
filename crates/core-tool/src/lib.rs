//! # Core Tool
//!
//! 工具能力系统。Core 不执行工具，只负责：
//! - 保存 tool 定义
//! - 告诉模型有哪些 tool
//! - 接收 tool call → 路由给正确客户端
//! - 等待结果 → 回填

use core_events::{event, EventBus};
use core_events::event_types::*;
use core_protocol::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use async_trait::async_trait;
use tracing::info;

/// 工具执行上下文
#[derive(Debug, Clone)]
pub struct ToolExecutionContext {
    pub session_id: String,
    pub run_id: String,
    pub client_id: String,
}

/// 工具执行器 trait（外部实现）
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        ctx: &ToolExecutionContext,
    ) -> Result<String, String>;
}

/// 工具调用请求（发给客户端执行）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallRequest {
    pub call_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub session_id: String,
    pub run_id: String,
    pub timeout_ms: u64,
}

/// 工具调用结果（客户端返回）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallResult {
    pub call_id: String,
    pub result: String,
    pub is_error: bool,
}

/// 工具管理器
pub struct ToolManager {
    /// 已注册工具定义: session_id -> (tool_name -> ToolRegistration)
    registrations: RwLock<HashMap<String, HashMap<String, ToolRegistration>>>,
    /// 工具定义（协议层）: session_id -> (tool_name -> Tool)
    definitions: RwLock<HashMap<String, HashMap<String, Tool>>>,
    event_bus: Arc<EventBus>,
}

impl ToolManager {
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self {
            registrations: RwLock::new(HashMap::new()),
            definitions: RwLock::new(HashMap::new()),
            event_bus,
        }
    }

    fn scope_key(session_id: &str) -> String {
        session_id.to_string()
    }

    /// 注册工具
    pub fn register(&self, session_id: &str, tool: Tool, registration: ToolRegistration) -> Result<(), String> {
        let tool = tool.with_compiled_schemas()?;
        let scope = Self::scope_key(session_id);
        let name = tool.name.clone();
        info!(session_id = %session_id, tool_name = %name, client_id = %registration.client_id, "tool registered");
        self.definitions.write().unwrap()
            .entry(scope.clone())
            .or_default()
            .insert(name.clone(), tool);
        self.registrations.write().unwrap()
            .entry(scope)
            .or_default()
            .insert(name.clone(), registration);
        self.event_bus.emit(event!(
            TOOL_REGISTERED,
            session_id,
            serde_json::json!({ "tool_name": name, "session_id": session_id })
        ));
        Ok(())
    }

    /// 从持久化快照恢复工具，不触发运行时注册事件。
    pub fn restore(&self, session_id: &str, tool: Tool, registration: ToolRegistration) -> Result<(), String> {
        let tool = tool.with_compiled_schemas()?;
        let scope = Self::scope_key(session_id);
        let name = tool.name.clone();
        info!(session_id = %session_id, tool_name = %name, client_id = %registration.client_id, "tool restored from snapshot");
        self.definitions.write().unwrap()
            .entry(scope.clone())
            .or_default()
            .insert(name.clone(), tool);
        self.registrations.write().unwrap()
            .entry(scope)
            .or_default()
            .insert(name, registration);
        Ok(())
    }

    /// 注销工具
    pub fn unregister(&self, session_id: &str, name: &str) {
        let scope = Self::scope_key(session_id);
        let mut defs_guard = self.definitions.write().unwrap();
        if let Some(defs) = defs_guard.get_mut(&scope) {
            defs.remove(name);
            let should_remove_scope = defs.is_empty();
            if should_remove_scope {
                defs_guard.remove(&scope);
            }
        }
        drop(defs_guard);

        let mut regs_guard = self.registrations.write().unwrap();
        if let Some(regs) = regs_guard.get_mut(&scope) {
            regs.remove(name);
            let should_remove_scope = regs.is_empty();
            if should_remove_scope {
                regs_guard.remove(&scope);
            }
        }
    }

    /// 获取工具定义列表（传给模型）
    pub fn get_tools(&self, session_id: &str) -> Vec<Tool> {
        let scope = Self::scope_key(session_id);
        self.definitions.read().unwrap()
            .get(&scope)
            .map(|defs| defs.values().cloned().collect())
            .unwrap_or_default()
    }

    /// 按权限过滤工具
    pub fn filter_by_permissions(&self, session_id: &str, permissions: &[String]) -> Vec<Tool> {
        let scope = Self::scope_key(session_id);
        let regs = self.registrations.read().unwrap();
        let defs = self.definitions.read().unwrap();
        defs.get(&scope)
            .into_iter()
            .flat_map(|m| m.iter())
            .filter(|(name, _)| {
                if let Some(reg) = regs.get(&scope).and_then(|m| m.get(*name)) {
                    reg.permissions.iter().any(|p| permissions.contains(p))
                } else {
                    true // 无权限要求的工具默认可用
                }
            })
            .map(|(_, tool)| tool.clone())
            .collect()
    }

    /// 获取工具的注册信息
    pub fn get_registration(&self, session_id: &str, name: &str) -> Option<ToolRegistration> {
        let scope = Self::scope_key(session_id);
        self.registrations.read().unwrap()
            .get(&scope)
            .and_then(|regs| regs.get(name))
            .cloned()
    }

    /// 检查工具是否已注册
    pub fn has_tool(&self, session_id: &str, name: &str) -> bool {
        let scope = Self::scope_key(session_id);
        self.definitions.read().unwrap()
            .get(&scope)
            .map(|defs| defs.contains_key(name))
            .unwrap_or(false)
    }

    pub fn count(&self) -> usize {
        self.definitions.read().unwrap().values().map(|defs| defs.len()).sum()
    }

    /// 获取单个工具定义（含 schema，用于校验）
    pub fn get_tool(&self, session_id: &str, name: &str) -> Option<Tool> {
        let scope = Self::scope_key(session_id);
        self.definitions.read().unwrap()
            .get(&scope)
            .and_then(|defs| defs.get(name))
            .cloned()
    }

    /// 获取所有已注册工具名
    pub fn tool_names(&self, session_id: &str) -> Vec<String> {
        let scope = Self::scope_key(session_id);
        self.definitions.read().unwrap()
            .get(&scope)
            .map(|defs| defs.keys().cloned().collect())
            .unwrap_or_default()
    }
}

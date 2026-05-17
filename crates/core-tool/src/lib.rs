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
    /// 已注册工具定义: tool_name -> ToolRegistration
    registrations: RwLock<HashMap<String, ToolRegistration>>,
    /// 工具定义（协议层）: tool_name -> Tool
    definitions: RwLock<HashMap<String, Tool>>,
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

    /// 注册工具
    pub fn register(&self, tool: Tool, registration: ToolRegistration) {
        let name = tool.name.clone();
        info!(tool_name = %name, client_id = %registration.client_id, "tool registered");
        self.definitions.write().unwrap().insert(name.clone(), tool);
        self.registrations.write().unwrap().insert(name.clone(), registration);
        self.event_bus.emit(event!(
            TOOL_REGISTERED,
            "",
            serde_json::json!({ "tool_name": name })
        ));
    }

    /// 注销工具
    pub fn unregister(&self, name: &str) {
        self.definitions.write().unwrap().remove(name);
        self.registrations.write().unwrap().remove(name);
    }

    /// 获取工具定义列表（传给模型）
    pub fn get_tools(&self) -> Vec<Tool> {
        self.definitions.read().unwrap().values().cloned().collect()
    }

    /// 按权限过滤工具
    pub fn filter_by_permissions(&self, permissions: &[String]) -> Vec<Tool> {
        let regs = self.registrations.read().unwrap();
        let defs = self.definitions.read().unwrap();
        defs.iter()
            .filter(|(name, _)| {
                if let Some(reg) = regs.get(*name) {
                    reg.permissions.iter().any(|p| permissions.contains(p))
                } else {
                    true // 无权限要求的工具默认可用
                }
            })
            .map(|(_, tool)| tool.clone())
            .collect()
    }

    /// 获取工具的注册信息
    pub fn get_registration(&self, name: &str) -> Option<ToolRegistration> {
        self.registrations.read().unwrap().get(name).cloned()
    }

    /// 检查工具是否已注册
    pub fn has_tool(&self, name: &str) -> bool {
        self.definitions.read().unwrap().contains_key(name)
    }

    pub fn count(&self) -> usize {
        self.definitions.read().unwrap().len()
    }

    /// 获取所有已注册工具名
    pub fn tool_names(&self) -> Vec<String> {
        self.definitions.read().unwrap().keys().cloned().collect()
    }
}

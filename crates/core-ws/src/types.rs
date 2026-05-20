//! WS 类型定义

use serde::{Deserialize, Serialize};

/// 工具注册请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterToolRequest {
    pub tool_name: String,
    pub description: String,
    pub schema: serde_json::Value,
    pub client_id: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub timeout_ms: u64,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// 供应商更新请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateProviderRequest {
    pub protocol: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub tools_mode: Option<String>,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
}

/// 发送消息请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageRequest {
    pub session_id: String,
    pub message: String,
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub max_rounds: u32,
}

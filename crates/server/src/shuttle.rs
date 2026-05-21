//! AgentKernel — Shuttle 部署入口
//!
//! 通过 `cargo shuttle deploy` 部署到 Shuttle 平台。
//! 环境变量通过 Shuttle Secrets 或 Shuttle.toml 注入。

use core_protocol::*;
use core_runtime::AgentKernel;
use core_storage::FileStorage;
use core_ws::server::{WsServer, WsToolRouter};
use std::sync::Arc;

#[shuttle_runtime::main]
async fn main() -> shuttle_axum::ShuttleAxum {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    // 从环境变量读取配置（Shuttle Secrets 或 Shuttle.toml）
    let protocol_str = std::env::var("PROTOCOL").unwrap_or_else(|_| "openai".to_string());
    let protocol = match protocol_str.as_str() {
        "claude" => Protocol::Claude,
        _ => Protocol::OpenAI,
    };

    let (default_key_env, default_url, default_model) = match protocol {
        Protocol::Claude => ("CLAUDE_API_KEY", "https://ai.accbot.vip", "claude-sonnet-4-20250514"),
        _ => ("OPENAI_API_KEY", "https://api.deepseek.com", "deepseek-chat"),
    };

    let api_key = std::env::var("API_KEY")
        .or_else(|_| std::env::var(default_key_env))
        .unwrap_or_default();

    let base_url = std::env::var("BASE_URL")
        .or_else(|_| std::env::var(&format!("{}_BASE_URL", default_key_env.replace("_API_KEY", ""))))
        .unwrap_or_else(|_| default_url.to_string());

    let model = std::env::var("MODEL")
        .or_else(|_| std::env::var(&format!("{}_MODEL", default_key_env.replace("_API_KEY", ""))))
        .unwrap_or_else(|_| default_model.to_string());

    let system_prompt = std::env::var("SYSTEM_PROMPT")
        .unwrap_or_else(|_| "你是一个有帮助的 AI 助手。".to_string());

    // Shuttle 环境中使用 /tmp 持久化目录（注意：Shuttle 免费层重启后会丢失）
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "/tmp/.aicore".to_string());

    let config = ProviderConfig {
        protocol: protocol.clone(),
        base_url: base_url.clone(),
        api_key,
        model: model.clone(),
        ..Default::default()
    };

    tracing::info!("═══════════════════════════════════════════");
    tracing::info!("  AgentKernel — Shuttle Deployment");
    tracing::info!("  协议:  {:?}", protocol);
    tracing::info!("  模型:  {}", model);
    tracing::info!("  地址:  {}", base_url);
    tracing::info!("  API:   {}", if config.api_key.is_empty() { "未设置" } else { "已设置" });
    tracing::info!("  存储:  {}", data_dir);
    tracing::info!("═══════════════════════════════════════════");

    let scaffold = {
        let storage = Arc::new(FileStorage::new(data_dir));
        let s = AgentKernel::with_storage(config, storage).with_system_prompt(&system_prompt);
        let loaded = s.load_session_index().await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "load session index failed");
            0
        });
        tracing::info!(sessions = loaded, "session index loaded");
        let tool_router = Arc::new(WsToolRouter::new(s.event_bus.clone()));
        Arc::new(s.with_tool_router(tool_router))
    };

    let server = WsServer::new(scaffold);
    let router = server.router();

    Ok(router.into())
}

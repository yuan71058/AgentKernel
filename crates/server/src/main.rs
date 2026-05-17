//! AgentKernel WS Server
//!
//! 启动一个 WebSocket 服务器，通过 Agent Runtime Protocol 与客户端交互。
//!
//! 用法：
//!   export OPENAI_API_KEY="sk-..."
//!   agentkernel
//!
//!   agentkernel --addr 0.0.0.0:9991 --protocol claude --model claude-sonnet-4-20250514
//!
//! 客户端通过 ws://<addr>/ws 连接，发送 Command 消息进行交互。

use core_protocol::*;
use core_runtime::AgentKernel;
use core_storage::FileStorage;
use core_ws::server::{WsServer, WsToolRouter};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let args: Vec<String> = std::env::args().collect();
    let get_arg = |name: &str, default: &str| -> String {
        args.iter().position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };

    let addr = get_arg("--addr", "0.0.0.0:9991");
    let protocol_str = get_arg("--protocol", "openai");

    let protocol = match protocol_str.as_str() {
        "claude" => Protocol::Claude,
        _ => Protocol::OpenAI,
    };

    let (default_key_env, default_url, default_model) = match protocol {
        Protocol::Claude => ("CLAUDE_API_KEY", "https://ai.accbot.vip", "claude-sonnet-4-20250514"),
        _ => ("OPENAI_API_KEY", "https://api.deepseek.com", "deepseek-chat"),
    };

    let api_key = get_arg("--api-key", "");
    let api_key = if api_key.is_empty() {
        std::env::var(default_key_env).unwrap_or_default()
    } else {
        api_key
    };

    let base_url = get_arg("--base-url", "");
    let base_url = if base_url.is_empty() {
        std::env::var(&format!("{}_BASE_URL", default_key_env.replace("_API_KEY", "")))
            .unwrap_or_else(|_| default_url.to_string())
    } else {
        base_url
    };

    let model = get_arg("--model", "");
    let model = if model.is_empty() {
        std::env::var(&format!("{}_MODEL", default_key_env.replace("_API_KEY", "")))
            .unwrap_or_else(|_| default_model.to_string())
    } else {
        model
    };

    let system_prompt = get_arg("--system-prompt", "你是一个有帮助的 AI 助手。");
    let data_dir = get_arg("--data-dir", ".aicore");

    let config = ProviderConfig {
        protocol: protocol.clone(),
        base_url: base_url.clone(),
        api_key,
        model: model.clone(),
        ..Default::default()
    };

    tracing::info!("═══════════════════════════════════════════");
    tracing::info!("  AgentKernel — Rust Runtime");
    tracing::info!("  协议:  {:?}", protocol);
    tracing::info!("  模型:  {}", model);
    tracing::info!("  地址:  {}", base_url);
    tracing::info!("  API:   {}", if config.api_key.is_empty() { "未设置（需通过 WS provider.update 配置）" } else { "已设置" });
    tracing::info!("  存储:  {}", data_dir);
    tracing::info!("  监听:  ws://{}/ws", addr);
    tracing::info!("═══════════════════════════════════════════");

    let scaffold = {
        let storage = Arc::new(FileStorage::new(data_dir.clone()));
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
    server.start(&addr).await?;

    Ok(())
}

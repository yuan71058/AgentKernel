//! AI Scaffold CLI 对话示例
//!
//! 用法：
//!   export OPENAI_API_KEY="sk-..."
//!   export OPENAI_BASE_URL="https://api.deepseek.com"
//!   export OPENAI_MODEL="deepseek-chat"
//!   cargo run --example chat
//!
//! Claude：
//!   export CLAUDE_API_KEY="sk-ant-..."
//!   export CLAUDE_BASE_URL="https://ai.accbot.vip"
//!   export CLAUDE_MODEL="claude-sonnet-4-20250514"
//!   cargo run --example chat -- --protocol claude

use core_runtime::Scaffold;
use core_protocol::*;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // 解析命令行参数
    let args: Vec<String> = std::env::args().collect();
    let protocol = if args.iter().any(|a| a == "--protocol" || a == "-p") {
        let idx = args.iter().position(|a| a == "--protocol" || a == "-p").unwrap();
        args.get(idx + 1).map(|s| s.as_str()).unwrap_or("openai")
    } else {
        "openai"
    };

    let config = match protocol {
        "claude" => ProviderConfig {
            protocol: Protocol::Claude,
            base_url: std::env::var("CLAUDE_BASE_URL").unwrap_or_else(|_| "https://ai.accbot.vip".into()),
            api_key: std::env::var("CLAUDE_API_KEY").unwrap_or_default(),
            model: std::env::var("CLAUDE_MODEL").unwrap_or_else(|_| "claude-sonnet-4-20250514".into()),
            ..Default::default()
        },
        _ => ProviderConfig {
            protocol: Protocol::OpenAI,
            base_url: std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com".into()),
            api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "deepseek-chat".into()),
            ..Default::default()
        },
    };

    if config.api_key.is_empty() {
        eprintln!("错误：请设置 API KEY 环境变量");
        eprintln!("  OpenAI: OPENAI_API_KEY=sk-...");
        eprintln!("  Claude: CLAUDE_API_KEY=sk-ant-...");
        std::process::exit(1);
    }

    println!("AI Scaffold — Rust Runtime");
    println!("协议: {:?} | 模型: {} | 地址: {}", config.protocol, config.model, config.base_url);
    println!("输入消息开始对话，输入 /quit 退出\n");

    let scaffold = Scaffold::new(config)
        .with_system_prompt("你是一个有帮助的 AI 助手。");

    let session_id = "cli_session";

    loop {
        print!("You > ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() { continue; }
        if input == "/quit" || input == "/exit" { break; }

        match scaffold.chat(session_id, input).await {
            Ok(resp) => {
                println!("\nAssistant > {}\n", resp.content);
                println!("  [tokens: in={} out={} | rounds={}]",
                    resp.usage.input_tokens, resp.usage.output_tokens, resp.tool_calls_made);
                for (i, t) in resp.traces.iter().enumerate() {
                    println!("  [trace-{}: {}ms {} {:?}]",
                        i, t.duration_ms, t.model, t.finish_reason);
                }
                println!();
            }
            Err(e) => {
                eprintln!("\n错误: {}\n", e);
            }
        }
    }

    println!("再见！");
    Ok(())
}

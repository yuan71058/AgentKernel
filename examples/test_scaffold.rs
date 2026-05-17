//! AI Scaffold — 完整单文件测试示例
//! 运行: cd rust && cargo run --example test_scaffold

use std::collections::HashMap;
use std::sync::Arc;

use ai_scaffold::*;
use ai_scaffold::config::*;
use ai_scaffold::hooks::{EventType, EventContext};
use ai_scaffold::tools::ToolDef;

#[tokio::main]
async fn main() -> Result<(), String> {
    println!("══════════════════════════════════════════════════");
    println!("  AI Scaffold — Rust 完整测试示例");
    println!("══════════════════════════════════════════════════");

    // ── 1. 配置 ──
    let api_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("DEEPSEEK_API_KEY"))
        .unwrap_or_default();

    if api_key.is_empty() {
        println!("\n⚠️  未设置 API Key，将使用模拟模式");
        println!("   设置方法: export OPENAI_API_KEY=sk-...");
    }

    let config = RuntimeConfig {
        provider: ProviderConfig {
            name: "deepseek".into(),
            protocol: Protocol::OpenAI,
            base_url: "https://api.deepseek.com".into(),
            api_key: api_key.clone(),
            model: "deepseek-chat".into(),
            context_window_tokens: 128_000,
            max_tokens: 4096,
            supports_vision: false,
            supports_tool_use: true,
            tools_mode: ToolsMode::Standard,
            temperature: 0.7,
        },
        tools: ToolsConfig {
            capabilities: vec![
                ToolCapability { key: "web.search".into(), enabled: true, label: "网络搜索".into() },
                ToolCapability { key: "math.calc".into(), enabled: true, label: "数学计算".into() },
                ToolCapability { key: "file.read".into(), enabled: true, label: "文件读取".into() },
                ToolCapability { key: "weather.query".into(), enabled: false, label: "天气查询".into() },
            ],
            max_tool_rounds: 10,
        },
        context: ContextConfig::default(),
        system_prompt: "你是一个有用的 AI 助手。你可以使用工具来帮助用户完成任务。请用中文回答。".into(),
    };

    // ── 2. 创建脚手架 ──
    let scaffold = Scaffold::new(config);

    // ── 3. 注册工具 ──
    register_tools(&scaffold);

    // ── 4. 注册钩子 ──
    register_hooks(&scaffold);

    // ── 5. 显示信息 ──
    println!("\n已注册工具: {} 个", scaffold.tool_manager.count());
    for t in scaffold.tool_manager.list_all() {
        println!("  - {}: {}...", t.name, &t.description[..t.description.len().min(50)]);
    }

    // ── 6. 测试 ──
    if !api_key.is_empty() {
        run_chat(&scaffold, "你好，请介绍一下你自己。", "test_1").await?;
        run_chat(&scaffold, "帮我搜索一下 Python 3.12 的新特性", "test_1").await?;
        run_chat(&scaffold, "计算一下 (15 + 27) * 3 - 8", "test_1").await?;
    } else {
        println!("\n── 模拟模式: 直接测试工具执行 ──\n");

        let mut input = HashMap::new();
        input.insert("query".to_string(), serde_json::json!("Python 3.12"));
        let r = scaffold.tool_manager.execute("search", &input, None).unwrap();
        println!("  search('Python 3.12') => {r}");

        let mut input = HashMap::new();
        input.insert("expression".to_string(), serde_json::json!("(15+27)*3-8"));
        let r = scaffold.tool_manager.execute("calculator", &input, None).unwrap();
        println!("  calculator => {r}");

        // 测试上下文统计
        scaffold.context_mgr.add_message("sim", Message::user("你好"));
        scaffold.context_mgr.add_message("sim", Message::assistant("你好！有什么可以帮助你的？"));
        let stats = scaffold.context_mgr.analyze("sim");
        println!("\n  [统计] 总消息: {} | 估算Token: {} | 压缩比: {:.2}%",
            stats.total_messages, stats.estimated_tokens, stats.compression_ratio);
    }

    scaffold.clear_session("test_1");
    println!("\n✅ 测试完成");
    Ok(())
}

fn register_tools(scaffold: &Scaffold) {
    scaffold.register_tool(ToolDef {
        name: "search".into(),
        description: "在互联网上搜索信息。当用户需要查找资料、新闻、知识时使用。".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": { "query": { "type": "string", "description": "搜索关键词" } },
            "required": ["query"]
        }).as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        capability_key: "web.search".into(),
        category: "web".into(),
        function: Some(Arc::new(|input| {
            let query = input.get("query").and_then(|v| v.as_str()).unwrap_or("");
            Ok(format!("搜索结果: 关于\"{query}\"，找到相关文档和使用教程"))
        })),
        context_function: None,
    }).unwrap();

    scaffold.register_tool(ToolDef {
        name: "calculator".into(),
        description: "执行数学计算。支持基本四则运算。".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": { "expression": { "type": "string", "description": "数学表达式" } },
            "required": ["expression"]
        }).as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        capability_key: "math.calc".into(),
        category: "math".into(),
        function: Some(Arc::new(|input| {
            let expr = input.get("expression").and_then(|v| v.as_str()).unwrap_or("");
            Ok(format!("计算结果: {expr} = (模拟值 42)"))
        })),
        context_function: None,
    }).unwrap();

    scaffold.register_tool(ToolDef {
        name: "file_read".into(),
        description: "读取文件内容。".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "文件路径" } },
            "required": ["path"]
        }).as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        capability_key: "file.read".into(),
        category: "file".into(),
        function: Some(Arc::new(|input| {
            let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
            Ok(format!("文件内容 ({path}):\n这是一段模拟的文件内容。"))
        })),
        context_function: None,
    }).unwrap();
}

fn register_hooks(scaffold: &Scaffold) {
    scaffold.register_hook(EventType::BeforeChat, "log_before", |ctx| {
        println!("  [Hook] BeforeChat | session={} | msg={}", ctx.session_id, &ctx.get_str("message")[..50.min(ctx.get_str("message").len())]);
        Ok(())
    });

    scaffold.register_hook(EventType::AfterChat, "log_after", |ctx| {
        println!("  [Hook] AfterChat  | {}", &ctx.get_str("content")[..80.min(ctx.get_str("content").len())]);
        Ok(())
    });

    scaffold.register_hook(EventType::ToolBefore, "log_tool_before", |ctx| {
        println!("  [Hook] ToolBefore | calling: {}", ctx.get_str("tool_name"));
        Ok(())
    });

    scaffold.register_hook(EventType::ToolAfter, "log_tool_after", |ctx| {
        let is_err = ctx.data.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
        let status = if is_err { "ERROR" } else { "OK" };
        println!("  [Hook] ToolAfter  | {} -> [{}] {}", ctx.get_str("tool_name"), status, &ctx.get_str("result")[..60.min(ctx.get_str("result").len())]);
        Ok(())
    });
}

async fn run_chat(scaffold: &Scaffold, message: &str, session_id: &str) -> Result<(), String> {
    println!("\n────────────────────────────────");
    println!("用户: {message}");
    println!("────────────────────────────────");

    match scaffold.chat(ChatOptions {
        session_id: session_id.into(),
        message: message.into(),
        ..Default::default()
    }).await {
        Ok(resp) => {
            println!("\n助手: {}", resp.content);
            println!("  Token: input={} output={} total={}",
                resp.usage.input_tokens, resp.usage.output_tokens, resp.usage.total());
            println!("  统计: 总消息={} | 估算Token={} | 压缩比={:.2}%",
                resp.stats.total_messages, resp.stats.estimated_tokens, resp.stats.compression_ratio);
        }
        Err(e) => println!("\n❌ 错误: {e}"),
    }
    Ok(())
}

#!/usr/bin/env python3
"""
AI Scaffold — 完整单文件测试示例
直接运行: python3 examples/test_scaffold.py

演示内容:
  1. 配置供应商（DeepSeek / OpenAI 兼容）
  2. 注册多个工具（带能力开关）
  3. 注册 Hook 事件监听
  4. 发送消息，触发 Agent 工具循环
  5. 查看上下文统计
"""

import sys
import os

# 确保能 import 到 ai_scaffold 包
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "python"))

from ai_scaffold import (
    Scaffold, RuntimeConfig, ProviderConfig, Protocol,
    ChatOptions, ChatResponse, ContextStats,
)
from ai_scaffold.config import ToolsConfig, ToolCapability, ContextConfig
from ai_scaffold.tools import ToolDef
from ai_scaffold.hooks import EventType, EventContext


# ═══════════════════════════════════════════════════════════════════════
#  1. 定义工具
# ═══════════════════════════════════════════════════════════════════════

def tool_search(input_data: dict) -> str:
    """模拟搜索工具"""
    query = input_data.get("query", "")
    return f'搜索结果: 关于"{query}"，找到以下内容:\n1. {query}的相关文档\n2. {query}的使用教程'


def tool_calculator(input_data: dict) -> str:
    """模拟计算器工具"""
    expr = input_data.get("expression", "0")
    try:
        # 注意: 生产环境不要用 eval
        result = eval(expr, {"__builtins__": {}}, {})
        return f"计算结果: {expr} = {result}"
    except Exception as e:
        return f"计算错误: {e}"


def tool_file_read(input_data: dict) -> str:
    """模拟文件读取工具"""
    path = input_data.get("path", "")
    return f'文件内容 ({path}):\n这是一段模拟的文件内容。\n用于测试工具调用是否正常工作。'


def tool_weather(input_data: dict) -> str:
    """模拟天气查询工具"""
    city = input_data.get("city", "未知")
    return f"{city} 今日天气: 晴, 25°C, 湿度 60%, 微风"


# ═══════════════════════════════════════════════════════════════════════
#  2. 定义 Hook 回调
# ═══════════════════════════════════════════════════════════════════════

def hook_before_chat(ctx: EventContext) -> None:
    """BeforeChat 钩子 — 记录日志"""
    msg = ctx.get_str("message")
    tools = ctx.data.get("tool_count", 0)
    print(f"  [Hook] BeforeChat | session={ctx.session_id} | tools={tools} | msg={msg[:50]}...")


def hook_after_chat(ctx: EventContext) -> None:
    """AfterChat 钩子 — 记录完成"""
    content = ctx.get_str("content")
    print(f"  [Hook] AfterChat  | response={content[:80]}...")


def hook_tool_before(ctx: EventContext) -> None:
    """ToolBefore 钩子 — 记录工具调用"""
    name = ctx.get_str("tool_name")
    print(f"  [Hook] ToolBefore | calling: {name}")


def hook_tool_after(ctx: EventContext) -> None:
    """ToolAfter 钩子 — 记录工具结果"""
    name = ctx.get_str("tool_name")
    result = ctx.get_str("result")
    is_error = ctx.data.get("is_error", False)
    status = "ERROR" if is_error else "OK"
    print(f"  [Hook] ToolAfter  | {name} -> [{status}] {result[:60]}")


def hook_on_error(ctx: EventContext) -> None:
    """OnError 钩子"""
    print(f"  [Hook] OnError    | {ctx.error}")


def hook_on_retry(ctx: EventContext) -> None:
    """OnRetry 钩子"""
    attempt = ctx.data.get("attempt", "?")
    print(f"  [Hook] OnRetry    | attempt #{attempt}")


def hook_on_compress(ctx: EventContext) -> None:
    """OnCompress 钩子 — 上下文压缩触发"""
    stats = ctx.data.get("stats")
    print(f"  [Hook] OnCompress | tokens={getattr(stats, 'estimated_tokens', '?')}")


# ═══════════════════════════════════════════════════════════════════════
#  3. 初始化脚手架
# ═══════════════════════════════════════════════════════════════════════

def create_scaffold(api_key: str = "", base_url: str = "", model: str = "") -> Scaffold:
    """创建并配置脚手架实例"""

    # ── 供应商配置 ──
    provider_cfg = ProviderConfig(
        name="deepseek",
        protocol=Protocol.OPENAI,           # OpenAI 兼容协议
        base_url=base_url or "https://api.deepseek.com",
        api_key=api_key or os.environ.get("OPENAI_API_KEY", ""),
        model=model or "deepseek-chat",
        context_window_tokens=128000,
        max_tokens=4096,
        supports_vision=False,
        supports_tool_use=True,
        temperature=0.7,
    )

    # ── 工具能力开关 ──
    tools_cfg = ToolsConfig(
        capabilities=[
            ToolCapability(key="web.search", enabled=True, label="网络搜索"),
            ToolCapability(key="math.calc", enabled=True, label="数学计算"),
            ToolCapability(key="file.read", enabled=True, label="文件读取"),
            ToolCapability(key="weather.query", enabled=False, label="天气查询"),  # 故意关闭
        ],
        max_tool_rounds=10,
    )

    # ── 上下文配置 ──
    context_cfg = ContextConfig(
        compress_threshold_percent=70,
        image_retention="latest_user_turn_only",
    )

    # ── 总配置 ──
    config = RuntimeConfig(
        provider=provider_cfg,
        tools=tools_cfg,
        context=context_cfg,
        system_prompt="你是一个有用的 AI 助手。你可以使用工具来帮助用户完成任务。请用中文回答。",
    )

    # ── 创建脚手架 ──
    scaffold = Scaffold(config)

    # ── 注册工具 ──
    import json

    scaffold.register_tool(ToolDef(
        name="search",
        description="在互联网上搜索信息。当用户需要查找资料、新闻、知识时使用。",
        input_schema={
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "搜索关键词"},
            },
            "required": ["query"],
        },
        capability_key="web.search",
        category="web",
        function=tool_search,
    ))

    scaffold.register_tool(ToolDef(
        name="calculator",
        description="执行数学计算。支持基本四则运算。",
        input_schema={
            "type": "object",
            "properties": {
                "expression": {"type": "string", "description": "数学表达式，如 '2+3*4'"},
            },
            "required": ["expression"],
        },
        capability_key="math.calc",
        category="math",
        function=tool_calculator,
    ))

    scaffold.register_tool(ToolDef(
        name="file_read",
        description="读取文件内容。",
        input_schema={
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "文件路径"},
            },
            "required": ["path"],
        },
        capability_key="file.read",
        category="file",
        function=tool_file_read,
    ))

    scaffold.register_tool(ToolDef(
        name="weather",
        description="查询指定城市的天气信息。",
        input_schema={
            "type": "object",
            "properties": {
                "city": {"type": "string", "description": "城市名称"},
            },
            "required": ["city"],
        },
        capability_key="weather.query",
        category="info",
        function=tool_weather,
    ))

    # ── 注册钩子 ──
    scaffold.register_hook(EventType.BEFORE_CHAT, "log_before", hook_before_chat)
    scaffold.register_hook(EventType.AFTER_CHAT, "log_after", hook_after_chat)
    scaffold.register_hook(EventType.TOOL_BEFORE, "log_tool_before", hook_tool_before)
    scaffold.register_hook(EventType.TOOL_AFTER, "log_tool_after", hook_tool_after)
    scaffold.register_hook(EventType.ON_ERROR, "log_error", hook_on_error)
    scaffold.register_hook(EventType.ON_RETRY, "log_retry", hook_on_retry)
    scaffold.register_hook(EventType.ON_COMPRESS, "log_compress", hook_on_compress)

    return scaffold


# ═══════════════════════════════════════════════════════════════════════
#  4. 运行测试
# ═══════════════════════════════════════════════════════════════════════

def print_stats(stats: ContextStats):
    """打印上下文统计"""
    print(f"\n  ┌─ 上下文统计 ─────────────────────────────")
    print(f"  │ 总消息轮次:    {stats.total_rounds}")
    print(f"  │ 用户消息:      {stats.user_messages}")
    print(f"  │ 助手消息:      {stats.assistant_messages}")
    print(f"  │ 工具调用次数:  {stats.tool_use_count}")
    print(f"  │ 工具调用轮次:  {stats.tool_round_count}")
    print(f"  │ 纯对话轮次:    {stats.total_rounds - stats.tool_round_count}")
    print(f"  │ 估算 Token:    {stats.estimated_tokens}")
    print(f"  │ 内容字节数:    {stats.content_bytes}")
    print(f"  │ 图片数量:      {stats.image_count}")
    print(f"  │ 压缩比:        {stats.compression_ratio:.2f}%")
    print(f"  │ 需要压缩:      {'是' if stats.should_compress else '否'}")
    print(f"  └─────────────────────────────────────────\n")


def run_test(scaffold: Scaffold, message: str, session_id: str = "test_session"):
    """执行单次对话测试"""
    print(f"\n{'='*60}")
    print(f"用户: {message}")
    print(f"{'='*60}")

    try:
        resp = scaffold.chat(ChatOptions(
            session_id=session_id,
            message=message,
        ))

        print(f"\n助手: {resp.content}")
        print(f"  Token 消耗: input={resp.usage.input_tokens} output={resp.usage.output_tokens} total={resp.usage.total}")

        # 打印上下文统计
        print_stats(resp.stats)

        # 打印调用追踪
        if resp.traces:
            for i, t in enumerate(resp.traces):
                print(f"  Trace #{i}: {t.api_url} | {t.duration_ms}ms | attempt={t.attempt}")

        return resp

    except Exception as e:
        print(f"\n错误: {e}")
        return None


def main():
    print("=" * 60)
    print("  AI Scaffold — 完整测试示例")
    print("=" * 60)

    # 检查 API Key
    api_key = os.environ.get("OPENAI_API_KEY") or os.environ.get("DEEPSEEK_API_KEY", "")
    if not api_key:
        print("\n⚠️  未设置 API Key，将使用模拟模式（工具直接返回结果，不调用真实 AI）")
        print("   设置方法: export OPENAI_API_KEY=sk-...")
        print("   或:       export DEEPSEEK_API_KEY=sk-...\n")

    # 创建脚手架
    scaffold = create_scaffold(api_key=api_key)

    # 显示已注册的工具和钩子
    print(f"\n已注册工具: {scaffold.tool_manager.count} 个")
    for t in scaffold.tool_manager.list_all():
        cap_status = "ON" if scaffold.config.tools.is_capability_enabled(
            next((c.key for c in scaffold.config.tools.capabilities if c.key), "")
        ) else "?"
        print(f"  - {t.name}: {t.description[:40]}...")

    print(f"\n已注册钩子事件: {len(scaffold.hooks.list_registered())} 种")
    for et in scaffold.hooks.list_registered():
        print(f"  - {et.value}")

    # ── 测试对话 ──
    if api_key:
        # 有 API Key 时，进行真实对话
        run_test(scaffold, "你好，请介绍一下你自己。", "test_1")
        run_test(scaffold, "帮我搜索一下 Python 3.12 的新特性", "test_1")
        run_test(scaffold, "计算一下 (15 + 27) * 3 - 8", "test_1")
        run_test(scaffold, "帮我查一下北京的天气", "test_1")  # weather 能力关闭，AI 应无法调用
    else:
        # 无 API Key 时，只测试工具直接执行
        print("\n── 模拟模式: 直接测试工具执行 ──\n")

        result = scaffold.tool_manager.execute("search", {"query": "Python 3.12"})
        print(f"  search('Python 3.12') => {result}")

        result = scaffold.tool_manager.execute("calculator", {"expression": "(15+27)*3-8"})
        print(f"  calculator('(15+27)*3-8') => {result}")

        result = scaffold.tool_manager.execute("file_read", {"path": "/etc/hosts"})
        print(f"  file_read('/etc/hosts') => {result}")

        # 测试被关闭的能力
        try:
            result = scaffold.tool_manager.execute("weather", {"city": "北京"})
            print(f"  weather('北京') => {result}")
        except KeyError as e:
            print(f"  weather('北京') => ❌ {e}")

        # 测试不存在的工具
        try:
            result = scaffold.tool_manager.execute("nonexistent", {})
        except KeyError as e:
            print(f"  nonexistent() => ❌ {e}")

        # 测试上下文统计（手动添加消息模拟）
        from ai_scaffold.protocol import new_user_message, new_assistant_message
        scaffold.context_mgr.add_message("sim_session", new_user_message("你好"))
        scaffold.context_mgr.add_message("sim_session", new_assistant_message("你好！有什么可以帮助你的？"))
        scaffold.context_mgr.add_message("sim_session", new_user_message("搜索 Python 教程"))
        stats = scaffold.context_mgr.analyze("sim_session")
        print_stats(stats)

    # ── 清理 ──
    scaffold.clear_session("test_1")
    print("\n✅ 测试完成")


if __name__ == "__main__":
    main()

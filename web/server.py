#!/usr/bin/env python3
"""
AI Scaffold — Web 可视化测试服务器

启动: python3 web/server.py [端口]
默认端口: 8899

功能:
  - POST /api/chat         发送消息（SSE 流式响应）
  - GET  /api/stats         获取会话上下文统计
  - GET  /api/tools         获取已注册工具列表
  - POST /api/tools         动态注册/注销工具
  - GET  /api/config        获取当前配置
  - POST /api/config        更新配置
  - POST /api/session/clear 清除会话
  - GET  /                  Web 测试界面
"""

import sys
import os
import json
import time
import asyncio
from http.server import HTTPServer, SimpleHTTPRequestHandler
from urllib.parse import urlparse, parse_qs
from io import BytesIO

# 确保能 import
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "python"))

from ai_scaffold import (
    Scaffold, RuntimeConfig, ProviderConfig, Protocol,
    ChatOptions, ChatResponse,
)
from ai_scaffold.config import ToolsConfig, ToolCapability, ContextConfig, ToolsMode
from ai_scaffold.tools import ToolDef
from ai_scaffold.hooks import EventType, EventContext


# ─── 内置工具 ─────────────────────────────────────────────────────────

BUILTIN_TOOLS = {
    "search": {
        "description": "在互联网上搜索信息",
        "input_schema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
        "capability_key": "web.search",
        "category": "web",
        "function": lambda inp: f'搜索结果: 关于"{inp.get("query", "")}"的相关内容',
    },
    "calculator": {
        "description": "执行数学计算",
        "input_schema": {"type": "object", "properties": {"expression": {"type": "string"}}, "required": ["expression"]},
        "capability_key": "math.calc",
        "category": "math",
        "function": lambda inp: f"计算结果: {inp.get('expression', '0')} = (模拟)",
    },
    "file_read": {
        "description": "读取文件内容",
        "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]},
        "capability_key": "file.read",
        "category": "file",
        "function": lambda inp: f"文件内容 ({inp.get('path', '')}):\n模拟文件数据",
    },
    "weather": {
        "description": "查询天气信息",
        "input_schema": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]},
        "capability_key": "weather.query",
        "category": "info",
        "function": lambda inp: f"{inp.get('city', '')} 天气: 晴, 25°C",
    },
}


# ─── 全局状态 ─────────────────────────────────────────────────────────

_scaffold: Scaffold = None
_hook_events: list = []  # 用于前端展示的事件日志
MAX_EVENTS = 200


def _hook_logger(event_type: str):
    def handler(ctx: EventContext):
        entry = {
            "time": time.strftime("%H:%M:%S"),
            "type": ctx.type.value if hasattr(ctx.type, 'value') else str(ctx.type),
            "session": ctx.session_id,
            "data": {k: str(v)[:200] for k, v in ctx.data.items()},
        }
        _hook_events.append(entry)
        if len(_hook_events) > MAX_EVENTS:
            _hook_events.pop(0)
        return None
    return handler


def init_scaffold(api_key="", base_url="", model="", system_prompt=""):
    global _scaffold

    provider = ProviderConfig(
        name="custom",
        protocol=Protocol.OPENAI,
        base_url=base_url or "https://api.deepseek.com",
        api_key=api_key or os.environ.get("OPENAI_API_KEY", ""),
        model=model or "deepseek-chat",
        context_window_tokens=128000,
        max_tokens=4096,
        supports_tool_use=True,
        temperature=0.7,
    )

    tools_cfg = ToolsConfig(
        capabilities=[
            ToolCapability(key="web.search", enabled=True, label="网络搜索"),
            ToolCapability(key="math.calc", enabled=True, label="数学计算"),
            ToolCapability(key="file.read", enabled=True, label="文件读取"),
            ToolCapability(key="weather.query", enabled=False, label="天气查询"),
        ],
        max_tool_rounds=10,
    )

    config = RuntimeConfig(
        provider=provider,
        tools=tools_cfg,
        context=ContextConfig(compress_threshold_percent=70),
        system_prompt=system_prompt or "你是一个有用的 AI 助手。可以使用工具帮助用户。请用中文回答。",
    )

    _scaffold = Scaffold(config)

    # 注册内置工具
    for name, tool_def in BUILTIN_TOOLS.items():
        _scaffold.register_tool(ToolDef(
            name=name,
            description=tool_def["description"],
            input_schema=tool_def["input_schema"],
            capability_key=tool_def["capability_key"],
            category=tool_def["category"],
            function=tool_def["function"],
        ))

    # 注册钩子（记录事件日志给前端）
    for et in EventType:
        _scaffold.register_hook(et, f"log_{et.value}", _hook_logger(et))

    return _scaffold


# ─── HTTP 处理器 ──────────────────────────────────────────────────────

class APIHandler(SimpleHTTPRequestHandler):

    def do_GET(self):
        parsed = urlparse(self.path)
        path = parsed.path

        if path == "/" or path == "/index.html":
            self._serve_file("index.html", "text/html")
        elif path == "/api/stats":
            self._json_response(self._get_stats())
        elif path == "/api/tools":
            self._json_response(self._get_tools())
        elif path == "/api/config":
            self._json_response(self._get_config())
        elif path == "/api/events":
            self._json_response({"events": _hook_events[-50:]})
        else:
            self._json_response({"error": "not found"}, 404)

    def do_POST(self):
        parsed = urlparse(self.path)
        path = parsed.path
        body = self._read_body()

        if path == "/api/chat":
            self._handle_chat(body)
        elif path == "/api/tools":
            self._handle_tool_action(body)
        elif path == "/api/config":
            self._handle_config_update(body)
        elif path == "/api/session/clear":
            session_id = body.get("session_id", "web_session")
            _scaffold.clear_session(session_id)
            self._json_response({"ok": True, "cleared": session_id})
        else:
            self._json_response({"error": "not found"}, 404)

    def _handle_chat(self, body):
        """处理对话请求，SSE 流式返回"""
        message = body.get("message", "")
        session_id = body.get("session_id", "web_session")

        if not message:
            self._json_response({"error": "message is required"}, 400)
            return

        # 检查是否有 API Key
        if not _scaffold.config.provider.api_key:
            # 模拟模式：直接调用工具
            self._handle_mock_chat(message, session_id)
            return

        # 打印当前协议便于调试
        print(f"[CHAT] Protocol: {_scaffold.config.provider.protocol}, Model: {_scaffold.config.provider.model}", flush=True)

        try:
            resp = _scaffold.chat(ChatOptions(
                session_id=session_id,
                message=message,
            ))

            result = {
                "ok": True,
                "content": resp.content,
                "usage": {
                    "input_tokens": resp.usage.input_tokens,
                    "output_tokens": resp.usage.output_tokens,
                    "total": resp.usage.total,
                },
                "stats": {
                    "total_rounds": resp.stats.total_rounds,
                    "user_messages": resp.stats.user_messages,
                    "assistant_messages": resp.stats.assistant_messages,
                    "tool_use_count": resp.stats.tool_use_count,
                    "tool_round_count": resp.stats.tool_round_count,
                    "estimated_tokens": resp.stats.estimated_tokens,
                    "content_bytes": resp.stats.content_bytes,
                    "image_count": resp.stats.image_count,
                    "compression_ratio": round(resp.stats.compression_ratio, 2),
                    "should_compress": resp.stats.should_compress,
                },
                "traces": [
                    {"api_url": t.api_url, "duration_ms": t.duration_ms, "attempt": t.attempt}
                    for t in (resp.traces or [])
                ],
            }
            self._json_response(result)

        except Exception as e:
            # 调试：打印完整请求体到控制台
            import traceback
            print(f"\n=== API ERROR ===\n{e}")
            traceback.print_exc()
            # 尝试从 trace 中获取请求体
            req_info = {}
            try:
                if hasattr(e, '__traceback__'):
                    pass
            except Exception:
                pass
            self._json_response({"ok": False, "error": str(e)}, 500)

    def _handle_mock_chat(self, message, session_id):
        """模拟模式：不调用真实 AI，模拟工具调用结果"""
        from ai_scaffold.protocol import new_user_message, new_assistant_message

        # 简单关键词匹配模拟
        response_text = f"收到你的消息: {message}\n\n"
        tools_used = []

        if "搜索" in message or "search" in message.lower():
            result = BUILTIN_TOOLS["search"]["function"]({"query": message})
            response_text += f"🔍 {result}"
            tools_used.append("search")
        elif "计算" in message or any(c in message for c in "+-*/="):
            import re
            expr_match = re.search(r'[\d+\-*/().\s]+', message)
            expr = expr_match.group().strip() if expr_match else "0"
            result = BUILTIN_TOOLS["calculator"]["function"]({"expression": expr})
            response_text += f"🧮 {result}"
            tools_used.append("calculator")
        elif "天气" in message:
            result = BUILTIN_TOOLS["weather"]["function"]({"city": "北京"})
            response_text += f"🌤 {result}"
            tools_used.append("weather")
        else:
            response_text += "(模拟模式: 未设置 API Key，无法调用真实 AI。\n设置 OPENAI_API_KEY 环境变量后重启即可。)"

        _scaffold.context_mgr.add_message(session_id, new_user_message(message))
        _scaffold.context_mgr.add_message(session_id, new_assistant_message(response_text))
        stats = _scaffold.context_mgr.analyze(session_id)

        self._json_response({
            "ok": True,
            "content": response_text,
            "mock": True,
            "tools_used": tools_used,
            "usage": {"input_tokens": 0, "output_tokens": 0, "total": 0},
            "stats": {
                "total_rounds": stats.total_rounds,
                "tool_use_count": stats.tool_use_count,
                "estimated_tokens": stats.estimated_tokens,
                "content_bytes": stats.content_bytes,
                "compression_ratio": round(stats.compression_ratio, 2),
                "should_compress": stats.should_compress,
            },
        })

    def _get_stats(self):
        session_id = parse_qs(urlparse(self.path).query).get("session_id", ["web_session"])[0]
        stats = _scaffold.context_mgr.analyze(session_id)
        return {
            "session_id": session_id,
            "total_rounds": stats.total_rounds,
            "user_messages": stats.user_messages,
            "assistant_messages": stats.assistant_messages,
            "system_messages": stats.system_messages,
            "tool_use_count": stats.tool_use_count,
            "tool_result_count": stats.tool_result_count,
            "tool_round_count": stats.tool_round_count,
            "non_tool_rounds": stats.total_rounds - stats.tool_round_count,
            "estimated_tokens": stats.estimated_tokens,
            "content_bytes": stats.content_bytes,
            "total_messages": stats.total_messages,
            "image_count": stats.image_count,
            "compression_ratio": round(stats.compression_ratio, 2),
            "should_compress": stats.should_compress,
        }

    def _get_tools(self):
        tools = _scaffold.tool_manager.list_all()
        caps = {c.key: c.enabled for c in _scaffold.config.tools.capabilities}
        return {
            "tools": [
                {
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                    "capability_key": next((c.key for c in _scaffold.config.tools.capabilities), ""),
                    "enabled": True,
                }
                for t in tools
            ],
            "capabilities": [
                {"key": c.key, "enabled": c.enabled, "label": c.label}
                for c in _scaffold.config.tools.capabilities
            ],
            "count": len(tools),
        }

    def _get_config(self):
        p = _scaffold.config.provider
        return {
            "provider": {
                "name": p.name,
                "protocol": p.protocol.value if hasattr(p.protocol, 'value') else str(p.protocol),
                "base_url": p.base_url,
                "model": p.model,
                "has_api_key": bool(p.api_key),
                "context_window_tokens": p.context_window_tokens,
                "max_tokens": p.max_tokens,
                "temperature": p.temperature,
                "tools_mode": p.tools_mode.value if hasattr(p.tools_mode, 'value') else str(p.tools_mode),
            },
            "system_prompt": _scaffold.config.system_prompt[:200],
            "max_tool_rounds": _scaffold.config.tools.max_tool_rounds,
        }

    def _handle_tool_action(self, body):
        action = body.get("action", "")
        if action == "toggle":
            key = body.get("key", "")
            enabled = body.get("enabled", False)
            for cap in _scaffold.config.tools.capabilities:
                if cap.key == key:
                    cap.enabled = enabled
                    self._json_response({"ok": True, "key": key, "enabled": enabled})
                    return
            self._json_response({"error": f"capability '{key}' not found"}, 404)
        else:
            self._json_response({"error": "unknown action"}, 400)

    def _handle_config_update(self, body):
        p = _scaffold.config.provider
        if "protocol" in body:
            proto_str = body["protocol"].lower().strip()
            old_proto = p.protocol
            if proto_str == "claude":
                p.protocol = Protocol.CLAUDE
            else:
                p.protocol = Protocol.OPENAI
            print(f"[CONFIG] Protocol: {old_proto} -> {p.protocol}", flush=True)
        if "api_key" in body:
            p.api_key = body["api_key"]
        if "base_url" in body:
            p.base_url = body["base_url"]
        if "model" in body:
            p.model = body["model"]
        if "temperature" in body:
            p.temperature = float(body["temperature"])
        if "max_tokens" in body:
            p.max_tokens = int(body["max_tokens"])
        if "tools_mode" in body:
            mode_str = body["tools_mode"].lower().strip()
            if mode_str == "text_match":
                p.tools_mode = ToolsMode.TEXT_MATCH
            else:
                p.tools_mode = ToolsMode.STANDARD
            print(f"[CONFIG] ToolsMode: {p.tools_mode}", flush=True)
        if "system_prompt" in body:
            _scaffold.config.system_prompt = body["system_prompt"]
        self._json_response({"ok": True})

    # ─── 辅助方法 ─────────────────────────────────────────────────────

    def _read_body(self):
        length = int(self.headers.get("Content-Length", 0))
        if length == 0:
            return {}
        raw = self.rfile.read(length)
        try:
            return json.loads(raw)
        except json.JSONDecodeError:
            return {}

    def _json_response(self, data, status=200):
        body = json.dumps(data, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()
        self.wfile.write(body)

    def _serve_file(self, filename, content_type):
        filepath = os.path.join(os.path.dirname(__file__), filename)
        if not os.path.exists(filepath):
            self._json_response({"error": f"{filename} not found"}, 404)
            return
        with open(filepath, "rb") as f:
            body = f.read()
        self.send_response(200)
        self.send_header("Content-Type", f"{content_type}; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_OPTIONS(self):
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()

    def log_message(self, format, *args):
        # 静默 2xx 成功日志，只打印错误
        if args and "200" not in str(args[0]) and "204" not in str(args[0]):
            super().log_message(format, *args)


# ─── 启动 ─────────────────────────────────────────────────────────────

def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8899
    init_scaffold()

    print(f"""
╔══════════════════════════════════════════════════╗
║   AI Scaffold — Web 可视化测试界面               ║
║                                                  ║
║   访问: http://127.0.0.1:{port}                   ║
║                                                  ║
║   API 端点:                                      ║
║   POST /api/chat          发送消息               ║
║   GET  /api/stats          上下文统计            ║
║   GET  /api/tools          工具列表              ║
║   POST /api/tools          工具操作              ║
║   GET  /api/config         配置信息              ║
║   POST /api/config         更新配置              ║
║   GET  /api/events         Hook 事件日志         ║
║   POST /api/session/clear  清除会话              ║
╚══════════════════════════════════════════════════╝
""")

    server = HTTPServer(("0.0.0.0", port), APIHandler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n服务器已停止")
        server.server_close()


if __name__ == "__main__":
    main()

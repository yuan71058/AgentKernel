#!/usr/bin/env python3
"""
AgentKernel Web Debug Server

用途：
  - 仅托管 `web/static/` 调试页面
  - 不内置内核，不代理 AI 请求
  - 启动前先检查 AgentKernel Core 是否已运行

推荐流程：
  1. 在项目根目录运行：cargo run -p agentkernel-server
  2. 进入 web 目录运行：python3 server.py
  3. 浏览器打开：http://127.0.0.1:8899
"""

from __future__ import annotations

import os
import socket
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse


DEFAULT_WEB_HOST = "127.0.0.1"
DEFAULT_WEB_PORT = 8899
DEFAULT_KERNEL_WS = "ws://127.0.0.1:9991/ws"


def parse_host_port_from_ws(ws_url: str) -> tuple[str, int]:
    parsed = urlparse(ws_url)
    host = parsed.hostname or "127.0.0.1"
    if parsed.port is not None:
        return host, parsed.port
    if parsed.scheme == "wss":
        return host, 443
    return host, 80


def check_kernel_running(ws_url: str, timeout: float = 1.5) -> tuple[bool, str]:
    host, port = parse_host_port_from_ws(ws_url)
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True, f"AgentKernel Core 已运行: {ws_url}"
    except OSError as exc:
        return False, f"无法连接 AgentKernel Core: {ws_url} ({exc})"


def build_handler(static_dir: Path):
    class StaticHandler(SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(static_dir), **kwargs)

        def log_message(self, format: str, *args) -> None:
            print("[web]", format % args)

    return StaticHandler


def main() -> int:
    web_host = os.environ.get("AGENTKERNEL_WEB_HOST", DEFAULT_WEB_HOST)
    web_port = int(os.environ.get("AGENTKERNEL_WEB_PORT", str(DEFAULT_WEB_PORT)))
    kernel_ws = os.environ.get("AGENTKERNEL_WS", DEFAULT_KERNEL_WS)

    current_dir = Path(__file__).resolve().parent
    static_dir = current_dir / "static"
    if not static_dir.exists():
        print(f"[error] 静态目录不存在: {static_dir}")
        return 1

    ok, message = check_kernel_running(kernel_ws)
    if not ok:
        print(f"[error] {message}")
        print("[hint] 请先在项目根目录启动内核服务：")
        print("       cargo run -p agentkernel-server")
        print("[hint] 待看到 ws://127.0.0.1:9991/ws 已监听后，再进入 web 目录运行：")
        print("       python3 server.py")
        return 1

    print(f"[ok] {message}")

    handler = build_handler(static_dir)
    server = ThreadingHTTPServer((web_host, web_port), handler)

    print(f"[web] 调试页面已启动: http://{web_host}:{web_port}")
    print(f"[web] 页面默认连接内核: {kernel_ws}")
    print("[web] 按 Ctrl+C 停止")

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n[web] 已停止")
    finally:
        server.server_close()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

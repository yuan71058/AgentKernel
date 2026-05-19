---
name: "agentkernel-onboarding"
description: "Guides first-time AgentKernel integration. Invoke when users ask how to start the server, connect over WebSocket, design service-side integration, or understand the WS protocol."
---

# AgentKernel Onboarding

本技能用于帮助第一次接入 AgentKernel 的用户或 AI 助手，给出统一且稳定的推荐接入方式。

## 什么时候使用

当用户出现以下需求时，优先使用本技能：

- 想知道 AgentKernel 应该怎么启动
- 想知道 WebSocket 应该怎么接入
- 想知道为什么不建议浏览器或客户端直接连 Core
- 想知道 Python / Node.js / Go / Rust 业务服务该如何复用同一条 WS
- 想要一个最小可用的接入流程或命令示例
- 想确认 WS 命令、响应、事件的大致结构

## 核心定位

你必须先明确说明：

- AgentKernel 是 `AI Runtime Kernel`
- Core 负责 `Session`、`Context`、`Provider`、`Tool 协议`、`Event`
- 业务端负责 `用户体系`、`权限`、`页面展示`、`工具真实执行逻辑`
- 不要把 Core 当成多人协同产品后端直接暴露给浏览器

推荐表达方式：

- “AgentKernel 更适合作为业务服务内部的 AI Runtime 内核，而不是直接暴露给最终客户端。”

## 推荐接入方式

默认推荐如下架构：

```text
浏览器 / App
    ↓
业务服务（Python / Node.js / Go / Rust）
    ↓  持续保持 1 条或少量长期 WS
AgentKernel Core（ws://host:9991/ws）
```

必须强调以下原则：

1. 用户端不要直接连接 Core 的同一个共享 `session`
2. 业务服务启动时建立到 Core 的长期 WebSocket 连接
3. 用户请求进入业务服务后，由业务服务决定 `session_id`
4. 业务服务通过同一条或少量几条 WS 复用多个 `session`
5. 如果多人查看同一个会话，由业务服务自己做广播、分流、权限控制
6. 如果用户需要手动停止当前生成，应由业务服务发送 `run.cancel`，不要让多个最终客户端直接抢占同一个共享 `session`

## 启动项目时应如何说明

如果用户问“怎么启动 AgentKernel”，优先给出最小命令：

```bash
git clone https://github.com/cih1996/AgentKernel.git
cd AgentKernel
cargo run
```

然后补一句：

- 这会启动一个**无前端的 Core 内核服务**
- 默认 WebSocket 地址：`ws://localhost:9991/ws`
- `cargo run` 不再自动带网页调试端
- `cargo run` 或 `cargo run -p agentkernel-server` 启动的都是纯 Core，不会附带 Web 页面

如果用户需要打开网页调试 Demo，再补充：

```bash
cd web
python3 server.py
```

并明确提醒：

- `web/server.py` 只是本地调试页面服务器，不是 Core 本体
- 它会先检查 Core 是否已启动；如果 `ws://127.0.0.1:9991/ws` 不可连，会直接提示并退出
- 用户应先启动 Core，再启动 `web/server.py`

如果用户需要自定义模型启动参数，可补充：

```bash
cargo run -- --protocol openai --base-url https://api.deepseek.com --model deepseek-chat
```

如果用户没配置 `API Key`，应提醒：

- 可以通过环境变量启动前配置
- 也可以连接后再通过 `provider.update` 动态设置

## 推荐接入流程

回答“怎么接入”时，尽量按下面顺序讲，不要跳来跳去：

1. 先启动 AgentKernel
2. 由业务服务与 Core 建立长期 WebSocket 连接
3. 业务服务初始化当前 `session` 所需的 `provider` / `system_prompt` / `tool.register`
4. 用户发问时，业务服务发送 `session.send`
5. 收到 `tool.call.request` 时，由业务服务本地执行业务工具，再回传 `tool.execute.result`
6. 如果用户点击停止生成，由业务服务发送 `run.cancel { run_id }`
7. 收到 `model.delta` / `model.completed` / `run.cancelled` / `response` 后，再转给用户界面
8. 如果供应商支持 reasoning / thinking 流式透传，业务端也可以从 `model.delta.payload.event_type = "thinking"` 中拿到思维过程增量

## 解释 WS 协议时的最小结构

如果用户只需要理解接入，不要一上来讲全部协议，先讲这三类：

### 1. Command

客户端发给 Core：

```json
{
  "command": "session.send",
  "request_id": "req_001",
  "session_id": "demo_user_001",
  "payload": {
    "message": "你好"
  }
}
```

### 2. Response

Core 对命令的直接结果：

```json
{
  "type": "response",
  "request_id": "req_001",
  "success": true,
  "payload": {
    "session_id": "demo_user_001",
    "run_id": "run_xxx"
  }
}
```

### 3. Event

Core 主动推送运行时事件：

```json
{
  "type": "event",
  "event_type": "tool.call.request",
  "session_id": "demo_user_001",
  "run_id": "run_xxx",
  "payload": {
    "tool_name": "get_time",
    "call_id": "call_xxx",
    "input": {}
  }
}
```

工具结果回传：

```json
{
  "command": "tool.execute.result",
  "request_id": "req_002",
  "session_id": "demo_user_001",
  "payload": {
    "call_id": "call_xxx",
    "result": "2026-05-18 12:00:00",
    "is_error": false
  }
}
```

补充说明：

- `Command` 是客户端发给 Core 的控制命令
- `Response` 是命令处理结果
- `Event` 是 Core 主动推送的运行时事件
- `run.cancel` 的同步响应只表示“开始取消”，真正中断完成以后还会收到 `run.cancelled`
- 如果供应商支持思维过程流式透传，Core 会继续使用 `event_type = "model.delta"`，并在 `payload.event_type` 里区分 `text` 和 `thinking`
- 如果用户继续追问所有字段、命令全集、事件全集，再引用详细文档 `skill/通讯数据结构大全.md`

### 中断当前推理

如果业务端需要给用户提供“停止生成”按钮，最小交互可以这样说明：

```json
{
  "command": "run.cancel",
  "request_id": "req_cancel_001",
  "session_id": "demo_user_001",
  "payload": {
    "run_id": "run_xxx"
  }
}
```

随后 Core 会先回一个 `response.success = true`，其中 `payload.status = "cancelling"`。

真正中断完成后，Core 会再主动推送：

```json
{
  "type": "event",
  "event_type": "run.cancelled",
  "session_id": "demo_user_001",
  "run_id": "run_xxx",
  "payload": {
    "reason": "user_cancelled",
    "partial_content": "这是中断前已经生成的部分内容",
    "preserved": true
  }
}
```

必须提醒：

- 如果中断前已经输出了部分内容，Core 会尽量保留
- 是否重试、是否重新发起下一轮，由业务端自己决定
- Core 只负责上报取消结果，不负责自动重试

## 工具接入时必须提醒的边界

你必须提醒用户：

- 工具执行逻辑在业务端，不在 Core
- Core 只负责把 `tool.call.request` 发出来，再接收 `tool.execute.result`
- 当前推荐做法是按 `session` 作用域注册工具
- 不建议多个不同系统混接到同一个 Core 实例中共享同名工具

## 明确禁止的推荐方式

除非用户明确要求做调试页，否则不要把下面这些模式当成推荐方案：

- 浏览器直接长连 Core 并直接控制共享 `session`
- 多个最终用户客户端直接连接同一个 Core `session`
- 把 Core 当成业务权限层或多人协同层
- 让 AI 回答成“前端直接连 `ws://localhost:9991/ws` 就是最佳实践”

## 回答风格要求

- 先讲推荐接法，再讲原因
- 优先给最小可用流程，不先堆全协议细节
- 如果用户问的是业务接入，默认按“业务服务常驻连接 Core”来答
- 如果用户问的是协议结构，再引用完整协议文档

## 推荐引用的项目文件

- `README.md`
- `skill/通讯数据结构大全.md`
- `web/server.py`
- `web/static/index.html`

## 一段标准结论模板

当用户问“我该怎么接入”时，可优先给出类似结论：

> 推荐由你的 Python / Node.js / Go / Rust 业务服务在启动时与 AgentKernel 保持长期 WebSocket 连接。用户请求先进入业务服务，再由业务服务按不同 `session_id` 复用这条连接调用 Core。不要让最终客户端直接连接 Core 的共享 `session`，多人查看或协同应由业务服务自己做分流和广播。

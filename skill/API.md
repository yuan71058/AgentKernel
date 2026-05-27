# AgentKernel WebSocket API 文档

> **版本**: v1.1.1  
> **最后更新**: 2026-05-24  
> **状态**: 基于代码分析 + 实际数据包抓取验证

---

## 目录

1. [连接信息](#1-连接信息)
2. [消息总结构](#2-消息总结构)
3. [Hello 握手](#3-hello-握手)
4. [Command 命令详解](#4-command-命令详解)
5. [Event 事件详解](#5-event-事件详解)
6. [典型接入流程](#6-典型接入流程)
7. [注意事项与已知差异](#7-注意事项与已知差异)

---

## 1. 连接信息

### 1.1 连接地址

| 环境 | 地址 | 说明 |
|------|------|------|
| 本地开发 | `ws://localhost:9991/ws` | 默认端口 9991 |
| Fly.io 线上 | `wss://agentkernel.fly.dev/ws` | HTTPS 页面必须用 wss:// |

### 1.2 连接特征

- 基于 WebSocket，支持长连接、双向通信、流式推送
- 连接后服务端立即推送 Hello 响应，无需客户端主动发送
- 所有消息均为 JSON 文本帧（`Message::Text`）
- 支持 WebSocket Ping/Pong 保活

### 1.3 通讯日志

AgentKernel 会把业务端与 Kernel 的 WebSocket 通讯独立落盘，便于排查 session 历史不完整时的问题。

默认位置：

```text
.aicore/logs/comm.jsonl
.aicore/logs/comm.1.jsonl
.aicore/logs/comm.2.jsonl
...
```

每行是一个 JSON 记录，包含：

| 字段 | 说明 |
|------|------|
| `ts` | UTC 时间 |
| `direction` | `client_to_server` / `server_to_client` / `connection.open` / `connection.close` |
| `conn_id` | WS 连接 ID |
| `command` | 客户端命令名，服务端事件/响应可为空 |
| `session_id` | 会话 ID |
| `payload` | 原始命令、响应或事件内容 |

启动参数：

```bash
agentkernel --comm-log-dir .aicore/logs --comm-log-max-bytes 10485760 --comm-log-keep-files 10
```

默认单文件 10MB，保留 10 个历史文件。

### 1.4 连接示例

**JavaScript**:
```javascript
const ws = new WebSocket('ws://localhost:9991/ws');
ws.onopen = () => console.log('已连接');
ws.onmessage = (e) => {
  const data = JSON.parse(e.data);
  // data.type: 'response' | 'event' | 'stream'
};
ws.onclose = () => console.log('已断开');
```

**Python**:
```python
import asyncio, json, websockets

async def connect():
    async with websockets.connect('ws://localhost:9991/ws') as ws:
        hello = json.loads(await ws.recv())
        print('已连接:', hello['payload']['connection_id'])
```

---

## 2. 消息总结构

### 2.1 客户端 → 服务端：Command

```json
{
  "command": "命令名",
  "request_id": "请求唯一ID",
  "session_id": "会话ID",
  "payload": {}
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `command` | string | 是 | 命令名称，如 `session.send` |
| `request_id` | string | 推荐 | 请求唯一标识，用于和 Response 对应；不传时 Response 中 `request_id` 为空 |
| `session_id` | string | 视命令 | **顶层字段，不在 payload 内**；系统级命令可为空 |
| `payload` | object | 否 | 命令参数体；部分命令可传空 `{}` |

> **重要**: `session_id` 位于消息顶层，不是 `payload` 的子字段。这是一个常见错误。

### 2.2 服务端 → 客户端：Response

```json
{
  "type": "response",
  "request_id": "对应的请求ID",
  "success": true,
  "payload": {}
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | 固定为 `"response"` |
| `request_id` | string | 对应发起命令时的 `request_id` |
| `success` | bool | 命令是否成功 |
| `payload` | object | 成功时为返回数据；失败时至少含 `error` 字段 |

失败响应示例（已验证）:
```json
{
  "type": "response",
  "request_id": "c10",
  "success": false,
  "payload": {
    "error": "session_id and message are required"
  }
}
```

### 2.3 服务端 → 客户端：Event

```json
{
  "type": "event",
  "id": "evt_xxx",
  "event_type": "事件名",
  "session_id": "会话ID",
  "run_id": "run_xxx",
  "trace_id": "",
  "timestamp": "2026-05-21T18:50:21Z",
  "payload": {},
  "event_seq": 6
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | 固定为 `"event"` |
| `id` | string | 事件唯一 UUID |
| `event_type` | string | 事件类型名 |
| `session_id` | string | 事件所属会话 |
| `run_id` | string | 事件所属运行流程（可为空） |
| `trace_id` | string | 预留链路追踪字段 |
| `timestamp` | string (ISO 8601) | 事件时间 |
| `payload` | object/null | 事件具体内容 |
| `event_seq` | u64 | Session 内单调递增序号，用于断线补拉 |

### 2.4 服务端 → 客户端：Stream

```json
{
  "type": "stream",
  "session_id": "",
  "run_id": "",
  "event": "ping",
  "data": { "ok": true }
}
```

当前主要用于 WebSocket Ping 保活。模型流式输出不走 `stream`，而是通过 `event`（`event_type: "model.delta"`）推送。

---

## 3. Hello 握手

连接建立后，服务端立即发送一条 Hello Response，无需客户端请求。

### 请求

无（服务端主动推送）。

### 响应（已验证）

```json
{
  "type": "response",
  "request_id": "hello",
  "success": true,
  "payload": {
    "commands": [
      "session.send",
      "session.retry",
      "session.message.insert",
      "tool.register",
      "tool.unregister",
      "provider.update",
      "provider.get",
      "run.cancel",
      "runtime.sessions",
      "session.get",
      "session.info",
      "session.close",
      "session.archive",
      "session.unarchive",
      "session.delete",
      "session.fork",
      "session.messages",
      "session.list",
      "system.stats",
      "context.preview",
      "context.seed.add",
      "context.seed.delete",
      "context.seed.clear",
      "context.seed.set",
      "system_prompt.get",
      "system_prompt.set",
      "tool.list",
      "tool.get",
      "tool.execute.result"
    ],
    "connection_id": "d748bd30-c1fa-4e08-8312-f3dbbe5f3f02",
    "server_version": "1.1.0"
  }
}
```

| 字段 | 说明 |
|------|------|
| `commands` | 当前支持的全部命令列表 |
| `connection_id` | 本次连接的唯一标识 |
| `server_version` | 服务端版本 |

---

## 4. Command 命令详解

### 4.1 `session.send` — 发送消息并触发 AI 推理

**作用**: 发起一次用户消息推理。若 session 不存在会自动创建。触发模型流式事件链。

**请求**:

```json
{
  "command": "session.send",
  "request_id": "r1",
  "session_id": "my_session",
  "payload": {
    "message": "你好，请用一句话介绍你自己"
  }
}
```

| payload 字段 | 类型 | 必填 | 说明 |
|--------------|------|------|------|
| `message` | string | **是** | 用户输入文本 |
| `images` | string[] | 否 | 图像列表，每项为纯 Base64 字符串或 `data:image/png;base64,xxx` 格式均支持；Runtime 自动解析并转为 `ContentBlock::Image` 持久化。需要模型本身支持视觉（如 GPT-4o、Claude 3.5 Sonnet 等） |
| `audio` | object[] | 否 | 音频列表，每项含 `data`（base64 字符串）和 `format`（`"wav"` 或 `"mp3"`，默认 `"wav"`）。仅 OpenAI 协议支持，Claude 协议会返回不支持错误。需模型本身支持音频（如 Gemma 4 + mmproj） |
| `max_repeated_tool_calls` | u32 | 否 | 连续相同工具调用检测阈值，默认 10；仅防死循环，不限制总调用次数 |

> **注意**: payload 中的字段名是 `message`，不是 `content`。`message`、`images`、`audio` 可以同时传。

**带图片的请求示例**:
```json
{
  "command": "session.send",
  "request_id": "r_img1",
  "session_id": "my_session",
  "payload": {
    "message": "这张图片里是什么？",
    "images": ["iVBORw0KGgo..."]
  }
}
```

**带音频的请求示例**:
```json
{
  "command": "session.send",
  "request_id": "r_audio1",
  "session_id": "my_session",
  "payload": {
    "message": "这段音频说了什么？",
    "audio": [{"data": "UklGRi...", "format": "wav"}]
  }
}
```

> **并发限制**: 同一 session 同时只能有一个活跃 run。如果 session 已有 run 在执行，再次调用 `session.send` 会返回错误，包含当前活跃的 `run_id`。业务端需自行排队：等收到 `run.completed` / `run.cancelled` 事件后再发送下一条。`session.message.insert` 不受此限制，运行中可安全调用。

> **能力校验**: 如果发送了图片或音频，但当前 session 的供应商配置中 `supports_image=false` 或 `supports_audio=false`，Core 会在推理前直接返回 `provider.capability` 错误，不会发给模型。请先在供应商设置中开启对应能力开关。

**成功响应**（已验证）:

> 注意：`session.send` 最终响应只返回轻量统计，不再内联返回完整 `trace_details`。详细 Trace 应通过事件、日志或后续专门 trace 查询能力获取，避免把 request/response body 反复塞进 WebSocket 响应。

```json
  "success": true,
  "payload": {
    "session_id": "my_session",
    "run_id": "run_xxx",
    "status": "completed",
    "content": "我是DeepSeek，一个由深度求索公司创造的免费AI助手！",
    "usage": { "input_tokens": 123, "output_tokens": 45 },
    "traces": 1,
    "tool_calls_made": 0
  }
}
```

**执行过程中的事件流**（已验证）:

```
→ Command: session.send / session.retry
← Event: run.started           (provider/model 信息)
← Event: tool_chain.diagnosed  (工具链诊断)
← Event: model.delta           (流式文本增量，多次)
← Event: model.delta
← Event: model.delta
  ...
← Event: model.completed       (完整文本)
← Event: run.completed         (运行统计)
← Response: session.send / session.retry       (最终结果)
```

**run.started 实际数据**:
```json
{
  "type": "event",
  "id": "evt_70eef542-...",
  "event_type": "run.started",
  "session_id": "api-verify-001",
  "run_id": "run_577ca3bf-...",
  "timestamp": "2026-05-21T18:52:32.592696Z",
  "payload": {
    "model": "deepseek-reasoner",
    "provider": "claude"
  },
  "event_seq": 6
}
```

**model.completed 实际数据**:
```json
{
  "type": "event",
  "event_type": "model.completed",
  "payload": {
    "content": "我是DeepSeek，一个由深度求索公司创造的免费AI助手..."
  }
}
```

**run.completed 实际数据**:
```json
{
  "type": "event",
  "event_type": "run.completed",
  "payload": {
    "duration_ms": 3325,
    "input_tokens": 0,
    "output_tokens": 0,
    "status": "completed",
    "tool_calls_made": 0,
    "total_tokens": 0
  }
}
```

---

### 4.2 `session.retry` — 续跑上一轮失败推理

**作用**: 基于当前已落盘消息历史继续推理，不新增 user message。适合上一轮在工具结果回填后、继续请求模型时失败的场景。

**请求**:

```json
{
  "command": "session.retry",
  "request_id": "r_retry1",
  "session_id": "my_session",
  "payload": {
    "max_repeated_tool_calls": 10
  }
}
```

| payload 字段 | 类型 | 必填 | 说明 |
|--------------|------|------|------|
| `max_repeated_tool_calls` | u32 | 否 | 连续相同工具调用检测阈值，默认 10 |

**允许重试**:
- 最后一条有效消息是 `user`，表示用户输入后还没有最终 assistant 输出。
- 最后一条有效消息是 `tool_result`，表示工具结果已回填，可直接带着工具结果继续请求模型。

**拒绝重试**:
- 最后一条有效消息是普通 `assistant` 且不含 `tool_use`，说明已经有最终 AI 输出。
- 最后一条有效消息是含 `tool_use` 的 `assistant`，说明还有 pending tool call 没有结果，不能凭空续跑。
- 当前 session 仍有活跃 run。

**成功响应**: 与 `session.send` 基本一致，额外包含：

```json
{
  "retried": true
}
```

---

### 4.3 `session.message.insert` — 插入消息（不触发推理）

**作用**: 往 session 消息历史中插入一条 user 或 assistant 消息，不触发模型推理。下次 `session.send` 时这些消息会作为上下文的一部分被模型看到。

**典型场景**:
- 业务端需要注入用户侧备注、系统采集的数据
- 注入 assistant 回复（如预设回答、外部生成的内容）
- 不需要模型处理，只需要作为上下文存在的消息

**请求**（已验证）:

```json
{
  "type": "command",
  "request_id": "r2",
  "command": "session.message.insert",
  "session_id": "my_session",
  "payload": {
    "role": "user",
    "content": "这是插入的消息内容"
  }
}
```

| payload 字段 | 类型 | 必填 | 说明 |
|--------------|------|------|------|
| `role` | string | 是 | `"user"` 或 `"assistant"` |
| `content` | string | 是 | 消息文本内容 |

**成功响应**（已验证）:

```json
{
  "type": "response",
  "request_id": "r2",
  "success": true,
  "payload": {
    "message_id": "msg_3d2bcd54-0e56-47b3-a567-7ed681601522",
    "role": "user",
    "session_id": "my_session"
  }
}
```

**错误响应**:

| 场景 | error |
|------|-------|
| role 不是 user/assistant | `"role must be 'user' or 'assistant'"` |
| content 为空 | `"content is required"` |
| session_id 为空 | `"session_id is required"` |

> **注意**: 如果 session 不存在会自动创建。插入的消息 `kind` 为 `normal`，与普通对话消息无异。消息持久化到内存和存储层，刷新后仍然存在。

---

### 4.4 `provider.update` — 更新 Session 级 Provider 配置

**作用**: 为指定 session 设置模型供应商覆盖配置，持久化到 session metadata。

**请求**（已验证）:

```json
{
  "command": "provider.update",
  "request_id": "r2",
  "session_id": "my_session",
  "payload": {
    "protocol": "claude",
    "base_url": "https://api.deepseek.com/anthropic",
    "api_key": "sk-xxx",
    "model": "deepseek-reasoner"
  }
}
```

| payload 字段 | 类型 | 必填 | 说明 |
|--------------|------|------|------|
| `protocol` | string | 否 | `"openai"` 或 `"claude"` |
| `base_url` | string | 否 | API 地址 |
| `api_key` | string | 否 | 密钥 |
| `model` | string | 否 | 模型名 |
| `max_tokens` | u64 | 否 | 最大生成 token 数 |
| `temperature` | f64 | 否 | 温度参数 |
| `supports_image` | bool | 否 | 是否支持图片输入（默认 false）；开启后发送图片会校验此标志，不支持则提前报错 |
| `supports_audio` | bool | 否 | 是否支持音频输入（默认 false）；开启后发送音频会校验此标志，不支持则提前报错 |

**成功响应**（已验证）:

```json
{
  "type": "response",
  "request_id": "r2",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "provider": {
      "base_url": "https://api.deepseek.com/anthropic",
      "max_tokens": 4096,
      "model": "deepseek-reasoner",
      "protocol": "claude",
      "temperature": 0.0,
      "supports_image": false,
      "supports_audio": false
    }
  }
}
```

> **注意**: 成功响应中不返回 `api_key`，但 `provider.get` 会返回。

---

### 4.5 `provider.get` — 读取 Provider 配置

**作用**: 获取当前 session 的 provider 配置；无覆盖则返回全局默认。

**请求**:

```json
{
  "command": "provider.get",
  "request_id": "r3",
  "session_id": "my_session",
  "payload": {}
}
```

**成功响应**（已验证）:

```json
{
  "type": "response",
  "request_id": "r3",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "is_override": true,
    "provider": {
      "api_key": "sk-dcace9be12db4faca9b7b4d73edc2544",
      "base_url": "https://api.deepseek.com/anthropic",
      "max_tokens": 4096,
      "model": "deepseek-reasoner",
      "protocol": "claude",
      "temperature": 0.0
    }
  }
}
```

| 字段 | 说明 |
|------|------|
| `is_override` | `true` 表示使用 session 级覆盖，`false` 表示使用全局默认 |
| `provider.api_key` | 密钥完整返回（内部 API 设计） |

---

### 4.6 `system_prompt.set` — 设置系统提示词

**作用**: 设置系统提示词。带 `session_id` 时持久化为 session override。

**请求**:

```json
{
  "command": "system_prompt.set",
  "request_id": "r4",
  "session_id": "my_session",
  "payload": {
    "system_prompt": "你是一个可靠的中文助手"
  }
}
```

| payload 字段 | 类型 | 必填 | 说明 |
|--------------|------|------|------|
| `system_prompt` | string | 是 | 系统提示词文本 |

> **重要**: 字段名是 `system_prompt`，不是 `prompt`。

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r4",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "system_prompt": "你是一个可靠的中文助手",
    "is_session_override": true,
    "updated": true
  }
}
```

---

### 4.7 `system_prompt.get` — 读取系统提示词

**请求**:

```json
{
  "command": "system_prompt.get",
  "request_id": "r5",
  "session_id": "my_session",
  "payload": {}
}
```

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r5",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "system_prompt": "你是一个可靠的中文助手",
    "is_session_override": true
  }
}
```

---

### 4.8 `tool.register` — 注册工具

**作用**: 向 Kernel 注册一个可被模型调用的工具。带 `session_id` 时持久化工具快照到 session metadata。

**请求**（已验证）:

```json
{
  "command": "tool.register",
  "request_id": "r6",
  "session_id": "my_session",
  "payload": {
    "tool_name": "web_search",
    "description": "搜索互联网内容",
    "schema": {
      "type": "object",
      "properties": {
        "query": { "type": "string", "description": "搜索关键词" }
      },
      "required": ["query"]
    }
  }
}
```

| payload 字段 | 类型 | 必填 | 说明 |
|--------------|------|------|------|
| `tool_name` | string | **是** | 工具名称 |
| `description` | string | 否 | 工具功能描述 |
| `schema` | object | 否 | JSON Schema 定义输入参数；注册时会编译为 provider-specific schema，无法适配会拒绝注册 |
| `client_id` | string | 否 | 注册来源标识 |
| `permissions` | string[] | 否 | 权限标签 |
| `timeout_ms` | u64 | 否 | 超时毫秒数；`0` 表示无限等待 |
| `tags` | string[] | 否 | 分类标签 |

> **Claude 协议限制**: 工具名必须匹配 `^[a-zA-Z0-9_-]+$`，不能包含 `.` 等特殊字符。Claude 的 `input_schema` 顶层必须可编译为 `type: "object"`，且不能向 Claude 透传顶层 `oneOf` / `anyOf` / `allOf` / `enum` / `not`。
>
> **注册期 schema 检测**: Kernel 会在 `tool.register` 阶段把原始 `schema` 编译为 `compiled_schemas.claude` / `compiled_schemas.openai`。可转换的顶层 `oneOf` / `anyOf` / `allOf` 会合并为 Claude 可接受的 object schema；无法适配的 schema 会直接返回失败响应，不会注册进 session。编译后的 object schema 会统一补齐 `properties` 和 `required: []`，避免 OpenAI 兼容接口因 `required: null` / 缺失数组而拒绝。
>
> **失败响应示例**:
> ```json
> {
>   "type": "response",
>   "request_id": "r6",
>   "success": false,
>   "payload": {
>     "error": "invalid tool schema for 'bad_tool': Claude tool input_schema top-level must be object and cannot be adapted from this schema"
>   }
> }
> ```

**成功响应**（已验证）:

```json
{
  "type": "response",
  "request_id": "r6",
  "success": true,
  "payload": {
    "registered": "web_search",
    "session_id": "my_session"
  }
}
```

---

### 4.9 `tool.unregister` — 注销工具

**请求**:

```json
{
  "command": "tool.unregister",
  "request_id": "r7",
  "session_id": "my_session",
  "payload": {
    "tool_name": "web_search"
  }
}
```

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r7",
  "success": true,
  "payload": {
    "unregistered": "web_search",
    "session_id": "my_session"
  }
}
```

---

### 4.10 `tool.list` — 获取工具列表

**请求**:

```json
{
  "command": "tool.list",
  "request_id": "r8",
  "session_id": "my_session",
  "payload": {}
}
```

**成功响应**（已验证）:

```json
{
  "type": "response",
  "request_id": "r8",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "count": 1,
    "tools": [
      {
        "name": "calc_add",
        "description": "加法计算",
        "input_schema": {
          "type": "object",
          "properties": { "a": { "type": "number" }, "b": { "type": "number" } },
          "required": ["a", "b"]
        },
        "compiled_schemas": {
          "claude": {
            "type": "object",
            "properties": { "a": { "type": "number" }, "b": { "type": "number" } },
            "required": ["a", "b"]
          },
          "openai": {
            "type": "object",
            "properties": { "a": { "type": "number" }, "b": { "type": "number" } },
            "required": ["a", "b"]
          }
        },
        "client_id": "unknown",
        "timeout_ms": 0,
        "tags": []
      }
    ],
    "persisted_snapshot": [
      {
        "registration": {
          "tool_name": "calc_add",
          "description": "加法计算",
          "client_id": "unknown",
          "permissions": [],
          "tags": [],
          "timeout_ms": 0
        },
        "tool": {
          "name": "calc_add",
          "description": "加法计算",
          "input_schema": {},
          "compiled_schemas": {
            "claude": { "type": "object", "properties": {}, "additionalProperties": true },
            "openai": { "type": "object", "properties": {}, "additionalProperties": true }
          }
        }
      }
    ]
  }
}
```

---

### 4.11 `tool.get` — 获取单个工具详情

**请求**:

```json
{
  "command": "tool.get",
  "request_id": "r9",
  "session_id": "my_session",
  "payload": {
    "tool_name": "calc_add"
  }
}
```

**成功响应**（已验证）:

```json
{
  "type": "response",
  "request_id": "r9",
  "success": true,
  "payload": {
    "registration": {
      "tool_name": "calc_add",
      "description": "加法计算",
      "client_id": "unknown",
      "permissions": [],
      "tags": [],
      "timeout_ms": 0
    },
    "tool": {
      "name": "calc_add",
      "description": "加法计算",
      "input_schema": { "type": "object", "properties": {}, "required": [] },
      "compiled_schemas": {
        "claude": { "type": "object", "properties": {}, "required": [] },
        "openai": { "type": "object", "properties": {}, "required": [] }
      }
    }
  }
}
```

---

### 4.12 `tool.execute.result` — 回传工具执行结果

**作用**: 客户端收到 `tool.call.request` 事件后，执行工具并通过此命令回传最终结果。

> **标准约定**: `tool.call.request` 和 `tool.execute.result` 共同组成一组工具调用闭环。前者是请求，后者是结果。外部能力套件以后按这个格式接入即可。

#### 4.11.1 `tool.call.request` 标准请求格式

服务端发给业务端/能力执行器的标准事件：

```json
{
  "type": "event",
  "id": "evt_xxx",
  "event_type": "tool.call.request",
  "session_id": "my_session",
  "run_id": "run_xxx",
  "trace_id": "trace_xxx",
  "timestamp": "2026-05-22T12:00:00Z",
  "payload": {
    "call_id": "call_xxx",
    "tool_name": "web_search",
    "input": { "query": "AgentKernel" },
    "timeout_ms": 5000
  },
  "event_seq": 12
}
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `session_id` | string | **是** | 所属会话 |
| `run_id` | string | **是** | 所属运行流程 |
| `trace_id` | string | 否 | 链路追踪 ID，预留 |
| `payload.call_id` | string | **是** | 本次工具调用唯一 ID，用于结果回填 |
| `payload.tool_name` | string | **是** | 工具名 |
| `payload.input` | object | **是** | 工具输入参数 |
| `payload.timeout_ms` | u64 | 否 | 允许等待的超时时间；`0` 表示无限等待 |

#### 4.11.2 `tool.execute.result` 标准回传格式

```json
{
  "command": "tool.execute.result",
  "request_id": "r10",
  "session_id": "my_session",
  "payload": {
    "call_id": "call_xxx",
    "result": "2026-05-22 12:00:00",
    "is_error": false
  }
}
```

| payload 字段 | 类型 | 必填 | 说明 |
|--------------|------|------|------|
| `call_id` | string | **是** | 对应 `tool.call.request` 中的 `call_id` |
| `result` | string | **是** | 执行结果，**必须是字符串**；如果你要回传结构化数据，请先 `JSON.stringify` 后再放进来 |
| `is_error` | bool | 否 | 是否为错误结果，默认 `false` |

#### 4.11.3 结果回传规则

- `call_id` 必须原样回传，不能改写
- 一次 `call_id` 只允许最终确认一次
- `tool.execute.result` **不会返回标准 Response**，服务端消费后直接唤醒内部等待通道
- 如果执行失败，建议把错误信息放进 `result`，并将 `is_error` 设为 `true`
- 如果返回的是复杂对象，推荐格式：`result: "{\"ok\":true,\"data\":...}"`

> **注意**: 前端不应依赖此命令的 Response 做渲染；它是一个纯回填命令。

---

### 4.13 `session.list` — 获取 Session 列表

**请求**:

```json
{
  "command": "session.list",
  "request_id": "r11",
  "session_id": "",
  "payload": {
    "page": 0,
    "limit": 20
  }
}
```

| payload 字段 | 类型 | 说明 |
|--------------|------|------|
| `page` | u32 | 页码，从 0 开始 |
| `limit` | u32 | 每页条数，最大 100 |
| `status` | string | 按状态过滤：`active` / `paused` / `closed` / `archived`。不传时默认不返回归档会话；查看归档需传 `archived` |

**成功响应**（已验证）:

```json
{
  "type": "response",
  "request_id": "r11",
  "success": true,
  "payload": {
    "page": 0,
    "limit": 20,
    "pages": 2,
    "total": 27,
    "sessions": [
      {
        "session_id": "my_session",
        "type": "chat",
        "title": "my_session",
        "status": "active",
        "message_count": 12,
        "estimated_tokens": 0,
        "provider_override": true,
        "system_prompt_override": false,
        "created_at": "2026-05-21T16:53:09.655111+00:00",
        "updated_at": "2026-05-21T16:53:09.655112+00:00",
        "summary": ""
      }
    ]
  }
}
```

---

### 4.14 `session.info` — 获取 Session 详情

**请求**:

```json
{
  "command": "session.info",
  "request_id": "r12",
  "session_id": "my_session",
  "payload": {}
}
```

**成功响应**（已验证）:

```json
{
  "type": "response",
  "request_id": "r12",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "session": {
      "session_id": "my_session",
      "status": "active",
      "title": "my_session",
      "type": "chat",
      "created_at": "2026-05-21T18:42:59.691446+00:00",
      "updated_at": "2026-05-21T18:47:52.808689+00:00"
    },
    "context": {
      "estimated_tokens": 0,
      "message_count": 0,
      "seed_count": 0,
      "usage_percent": 0,
      "window_tokens": 128000
    },
    "provider_override": true,
    "system_prompt_override": true,
    "tool_count": 1
  }
}
```

---

### 4.15 `session.get` — 获取 Session 简要统计

**请求**:

```json
{
  "command": "session.get",
  "request_id": "r13",
  "session_id": "my_session",
  "payload": {}
}
```

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r13",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "message_count": 12,
    "estimated_tokens": 3200,
    "window_tokens": 128000,
    "usage_percent": 2
  }
}
```

---

### 4.16 `session.messages` — 分页读取全量消息

**请求**:

```json
{
  "command": "session.messages",
  "request_id": "r14",
  "session_id": "my_session",
  "payload": {
    "page": 0,
    "limit": 50,
    "order": "asc"
  }
}
```

| payload 字段 | 类型 | 说明 |
|--------------|------|------|
| `page` | u64 | 页码，从 0 开始 |
| `limit` | u64 | 每页条数，最大 200 |
| `order` | string | 排序方向，默认 `asc`；传 `desc` 时从最新消息开始返回，适合快速查询最后对话 |

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r14",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "page": 0,
    "limit": 50,
    "order": "asc",
    "total": 4,
    "pages": 1,
    "messages": [
      { "message_id": "msg_1", "session_id": "my_session", "run_id": "run_xxx", "role": "user", "kind": "normal", "text": "你好", "content": [{"type":"text","text":"你好"}], "created_at": "..." },
      { "message_id": "msg_2", "session_id": "my_session", "run_id": "run_xxx", "role": "assistant", "kind": "normal", "text": "你好！", "content": [{"type":"text","text":"你好！"}], "created_at": "..." }
    ]
  }
}
```

> 每条消息包含 `run_id`（同一轮模型运行的聚合边界）、`text`（纯文本摘要，跳过 ToolUse 块）和 `content`（完整内容块数组，含 tool_use / tool_result 等结构化数据）。前端渲染工具调用时应使用 `content` 字段。
>
> 如果只想快速拿最新消息，用 `payload.order = "desc"`、`page = 0`、`limit = 1~50`，无需先读取第一页等待 `total` 后再计算最后一页。

**role 取值**:

| role | 说明 |
|------|------|
| `user` | 用户消息 |
| `assistant` | AI 回复 |
| `system` | 系统消息 |
| `tool` | 工具调用结果（由 `tool.execute.result` 回传后写入） |

**kind 取值**:

| kind | 说明 |
|------|------|
| `normal` | 普通对话消息 |
| `tool_result` | 工具执行结果 |
| `compaction_summary` | 上下文压缩摘要 |
| `context_seed` | 注入的上下文 Seed |
| `system_note` | 系统内部备注 |

> **前端注意**: 加载聊天历史时，`user` 和 `assistant` 消息中的 `content` 数组可能包含 `tool_use` 和 `tool_result` 块，应结构化渲染而非只显示 `text`。`system` 和 `tool` role 的消息通常不作为普通对话展示。

---

### 4.17 `session.close` / `session.archive` / `session.unarchive` / `session.delete` — 会话生命周期管理

#### `session.close` — 关闭会话

**作用**: 卸载当前内存运行态并将 session 状态写为 `closed`，但完整保留 `.aicore/sessions/<session_id>` 历史目录。下次使用同一 `session_id` 时仍可从磁盘恢复。

```json
{
  "command": "session.close",
  "request_id": "r15_close",
  "session_id": "my_session",
  "payload": {}
}
```

成功响应：

```json
{
  "type": "response",
  "request_id": "r15_close",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "closed": true,
    "unloaded": true,
    "note": "session history preserved in storage"
  }
}
```

#### `session.archive` — 归档会话

**作用**: 只把 `session.json.status` 写为 `archived`，不移动目录、不删除历史。默认 `session.list` 不返回归档会话；查看归档需调用 `session.list` 并传 `status: "archived"`。

```json
{
  "command": "session.archive",
  "request_id": "r15_archive",
  "session_id": "my_session",
  "payload": {}
}
```

#### `session.unarchive` — 恢复归档

**作用**: 将归档会话状态恢复为 `closed`，重新出现在默认会话列表中，但不会自动启动运行。

```json
{
  "command": "session.unarchive",
  "request_id": "r15_unarchive",
  "session_id": "my_session",
  "payload": {}
}
```

#### `session.delete` — 永久删除会话

**作用**: 删除 `.aicore/sessions/<session_id>` 整个持久化目录，并移除内存索引。该操作不可恢复，必须显式传 `payload.permanent=true`。

```json
{
  "command": "session.delete",
  "request_id": "r15_delete",
  "session_id": "my_session",
  "payload": { "permanent": true }
}
```

成功响应：

```json
{
  "type": "response",
  "request_id": "r15_delete",
  "success": true,
  "payload": { "session_id": "my_session", "deleted": true, "permanent": true }
}
```

---

### 4.18 `session.fork` — 分叉 Session

**作用**: 将源 session 的全部数据（消息、上下文、seeds、runs、工具配置）复制到一个新 session。原 session 完全不受影响，新 session 可独立继续对话。

**请求**:

```json
{
  "command": "session.fork",
  "request_id": "r_fork_1",
  "session_id": "",
  "payload": {
    "source_session_id": "sess_original",
    "new_session_id": "sess_branch"
  }
}
```

**字段说明**:

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `source_session_id` | string | ✅ | 被分叉的源 session ID |
| `new_session_id` | string | ✅ | 新 session ID（必须不存在） |

**复制的数据**:

| 数据 | 处理方式 |
|------|---------|
| session.json | 复制，session_id 替换为新 ID，时间戳更新，状态重置为 Active |
| messages.jsonl | 全部复制，session_id 替换，保留原 message_id（可追溯） |
| context_state.json | 复制，生成新 context_id，session_id 替换 |
| seeds.jsonl | 全部复制，生成新 seed_id，session_id 替换 |
| runs.jsonl | 全部复制，session_id 替换 |
| events.jsonl | **不复制**（新 session 从零开始记录事件） |
| provider_config | 内存中自动复制到新 session |
| system_prompt | 随 session metadata 自动复制 |
| tools 注册快照 | 随 session metadata 自动复制 |

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r_fork_1",
  "success": true,
  "payload": {
    "source_session_id": "sess_original",
    "new_session_id": "sess_branch",
    "session": { "session_id": "sess_branch", "title": "...", "status": "Active", "..." : "..." },
    "forked": true
  }
}
```

**错误场景**:

| 错误 | 原因 |
|------|------|
| `source_session_id and new_session_id are required` | 字段为空 |
| `source session 'xxx' not found` | 源 session 不存在 |
| `destination session 'xxx' already exists` | 目标 session 已存在（不能覆盖） |

**使用场景**:

- 对话到一半想尝试不同方向，fork 一个分支继续聊，不影响原对话
- 压缩前先 fork 一份完整历史作为备份
- 多人协作：从同一个 session fork 出不同人的工作分支

---

### 4.19 `system.stats` — 系统统计

**请求**:

```json
{
  "command": "system.stats",
  "request_id": "r17",
  "session_id": "",
  "payload": {}
}
```

**成功响应**（已验证）:

```json
{
  "type": "response",
  "request_id": "r17",
  "success": true,
  "payload": {
    "default_provider": {
      "base_url": "https://api.deepseek.com",
      "context_window_tokens": 128000,
      "model": "deepseek-chat",
      "protocol": "openai"
    },
    "session_count": 27,
    "system_prompt_length": 37,
    "tool_count": 120
  }
}
```

---

### 4.20 `runtime.sessions` — 查询运行中的 Session

**请求**:

```json
{
  "command": "runtime.sessions",
  "request_id": "r18",
  "session_id": "",
  "payload": {}
}
```

**成功响应**:

> 前端调试台会把该命令当作静默状态查询处理：用于刷新“运行中会话/Run”面板，不建议每次都展示在原始消息流里；正常聊天过程已有 `run.started` / `run.completed` / `run.failed` 事件表达状态变化。

```json
  "success": true,
  "payload": {
    "running_session_count": 2,
    "running_run_count": 3,
    "sessions": ["session_a", "session_b"],
    "runs": [
      { "session_id": "session_a", "run_id": "run_xxx", "duration_ms": 1832, "status": "running" },
      { "session_id": "session_b", "run_id": "run_yyy", "duration_ms": 91234, "status": "cancelling" }
    ]
  }
}
```

---

### 4.21 `context.preview` — 预览上下文视图

**请求**:

```json
{
  "command": "context.preview",
  "request_id": "r19",
  "session_id": "my_session",
  "payload": {}
}
```

**成功响应**（已验证）:

```json
{
  "type": "response",
  "request_id": "r19",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "active_context": {
      "context_id": "ctx_fd528350-...",
      "created_at": "2026-05-21T18:47:52.809507Z",
      "mode": "full",
      "rules": { "base_seed_ids": [], "exclude_ranges": [] },
      "session_id": "my_session"
    },
    "counts": {
      "active_messages": 0,
      "all_messages": 0,
      "model_input_messages": 0,
      "seeds": 0
    },
    "messages": [],
    "seeds": [],
    "preview": "# Context Preview\n\n## Messages\n\n",
    "stats": {
      "estimated_tokens": 0,
      "message_count": 0,
      "usage_percent": 0,
      "window_tokens": 128000
    }
  }
}
```

---

### 4.22 `context.trim.set` — 设置主裁剪策略

主裁剪策略同一时间只有一个 `mode` 生效；清空主裁剪、从指定消息后开始、只保留最近 N 条，都统一通过本命令表达。

**请求**:

```json
{
  "command": "context.trim.set",
  "request_id": "r20",
  "session_id": "my_session",
  "payload": {
    "mode": "checkpoint",
    "trigger_max_context_messages": 300,
    "retain_recent_turns": 20
  }
}
```

| mode | payload 字段 | 说明 |
|------|--------------|------|
| `none` | - | 关闭主裁剪策略，恢复完整消息视图（不删除历史） |
| `keep_recent_messages` | `keep_messages` | 硬限制只保留最近 N 条上下文消息 |
| `include_after` | `message_id` | 只纳入指定消息之后的历史 |
| `checkpoint` | `trigger_max_context_messages`, `retain_recent_turns` | 按上下文消息数触发，按最近用户对话轮数保留 |

**额外事件**: `context.updated`（`payload.action = "trim.set"`；自动阶梯裁剪真正触发时为 `trim.checkpoint_applied`）

---

### 4.23 `context.exclude` — 排除消息区间

**请求**:

```json
{
  "command": "context.exclude",
  "request_id": "r21",
  "session_id": "my_session",
  "payload": {
    "start_message_id": "msg_a",
    "end_message_id": "msg_b"
  }
}
```

| payload 字段 | 类型 | 必填 | 说明 |
|--------------|------|------|------|
| `start_message_id` | string | 是 | 起始消息 ID |
| `end_message_id` | string | 否 | 结束消息 ID（缺省等于起点） |

**额外事件**: `context.updated`（`payload.action = "exclude"`）

---

### 4.24 `context.seed.add` — 新增上下文 Seed

Seed 是 Session 级动态前置上下文块，不属于普通消息历史；构建模型输入时位于普通 messages 之前，且不受主裁剪策略影响。

**请求**:

```json
{
  "command": "context.seed.add",
  "request_id": "r24",
  "session_id": "my_session",
  "payload": {
    "content": "用户偏好中文简洁回答",
    "kind": "user_preference",
    "enabled": true,
    "priority": 10
  }
}
```

| payload 字段 | 类型 | 说明 |
|--------------|------|------|
| `content` | string | Seed 文本 |
| `kind` | string | 类型：`system_memory` / `user_preference` / `world_state` / `agent_state` / `compaction_summary` |
| `enabled` | bool | 是否启用，默认 `true` |
| `priority` | i32 | 优先级，默认 `0`；数值越小越靠前注入 |

**额外事件**: `context.seed.added`

---

### 4.25 `context.seed.set` — 按类型覆盖写入 Seed

适合写入“同类型只保留一份”的动态上下文，例如压缩摘要、用户偏好、世界状态。执行时会先删除当前 Session 中同 kind 的旧 seeds，再写入新 seed。

> 注意：`context.seed.set` 只更新 seed，不会修改 `active_context`，不会移动 `context.trim.set` 的裁剪边界。压缩摘要写入后，如果业务端确认要隐藏旧历史，需要再显式调用 `context.trim.set`。
>
> 内核会把 enabled seed 作为模型 `system` 前置上下文提交；对 Claude 协议适配器来说，seed 会合并进顶层 `system` 字段，不会进入 `messages[]`，因为 Claude Messages API 的 `messages[]` 只允许 `user` / `assistant`。

**请求**:

```json
{
  "command": "context.seed.set",
  "request_id": "r25",
  "session_id": "my_session",
  "payload": {
    "kind": "compaction_summary",
    "content": "这是压缩后的历史摘要...",
    "enabled": true,
    "priority": 100
  }
}
```

**额外事件**: `context.seed.updated`

---

### 4.26 `context.seed.delete` — 删除指定 Seed

**请求**:

```json
{
  "command": "context.seed.delete",
  "request_id": "r26",
  "session_id": "my_session",
  "payload": {
    "seed_id": "seed_xxx"
  }
}
```

| payload 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `seed_id` | string | ✅ | 要删除的 seed ID |

**额外事件**: `context.seed.deleted`

---

### 4.27 `context.seed.clear` — 清空 Seeds

按 kind 清空；不传 kind 时清空当前 Session 的全部 seeds。

**请求**:

```json
{
  "command": "context.seed.clear",
  "request_id": "r27",
  "session_id": "my_session",
  "payload": {
    "kind": "compaction_summary"
  }
}
```

| payload 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `kind` | string | 可选 | 不传则清空全部 seeds |

**额外事件**: `context.seed.cleared`

---

### 4.28 `events.pull` — 断线补拉事件

**请求**:

```json
{
  "command": "events.pull",
  "request_id": "r26",
  "session_id": "my_session",
  "payload": {
    "since_seq": 12
  }
}
```

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r26",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "since_seq": 12,
    "current_seq": 20,
    "count": 8,
    "events": []
  }
}
```

---

### 4.29 `events.subscribe` — 订阅实时事件

**请求**:

```json
{
  "command": "events.subscribe",
  "request_id": "r27",
  "session_id": "my_session",
  "payload": {
    "since_seq": 12
  }
}
```

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r27",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "subscribed": true,
    "since_seq": 12,
    "current_seq": 20,
    "replayed": 8
  }
}
```

> **当前状态**: 协议入口已建立，`since_seq` 补发能力可用，但完整的常驻订阅转发机制尚未完全闭环。`session.send` 会临时订阅并转发本次 run 的相关事件。

---

### 4.30 `run.cancel` — 中断运行

**请求**:

```json
{
  "command": "run.cancel",
  "request_id": "r28",
  "session_id": "my_session",
  "payload": {
    "run_id": "run_xxx"
  }
}
```

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r28",
  "success": true,
  "payload": {
    "status": "cancelling",
    "session_id": "my_session",
    "run_id": "run_xxx",
    "cancelled": true
  }
}
```

> **注意**: 此 Response 只表示"已收到取消请求"。真正完成通知以 `run.cancelled` 事件和 `session.send` 最终响应（`status: "cancelled"`）为准。

---

## 5. Event 事件详解

### 5.1 事件总表

| 事件名 | 说明 | 已验证 |
|--------|------|--------|
| `run.started` | 推理开始，含 provider/model 信息 | ✅ |
| `model.delta` | 流式文本增量 | ✅ |
| `model.completed` | 模型输出完成 | ✅ |
| `run.completed` | 运行结束（含统计） | ✅ |
| `run.retrying` | Core 遇到可恢复供应商错误，正在内部重试 | ✅ |
| `run.cancelled` | 运行被中断 | - |
| `tool_chain.diagnosed` | 工具链诊断 | ✅ |
| `tool.call.request` | 请求业务端/能力执行器执行工具 | - |
| `tool.call.result` | 工具执行结果已回填 | - |
| `tool.registered` | 工具注册完成 | - |
| `run.failed` | 推理失败（模型/供应商/运行时错误） | ✅ |
| `session.created` | Session 创建 | - |
| `session.closed` | Session 关闭 | - |
| `session.archived` | Session 归档 | - |
| `session.unarchived` | Session 恢复归档 | - |
| `session.deleted` | Session 永久删除 | - |
| `context.threshold.reached` | 上下文 token 达阈值 | - |
| `context.updated` | 上下文规则更新 | - |
| `context.seed.added` | 新增 seed | - |
| `context.seed.updated` | 覆盖写入 seed | - |
| `context.seed.deleted` | 删除 seed | - |
| `context.seed.cleared` | 清空 seed | - |

### 5.2 关键事件详解

#### `model.delta` — 流式文本增量

```json
{
  "type": "event",
  "event_type": "model.delta",
  "payload": {
    "delta": "你好",
    "event_type": "text"
  }
}
```

- `payload.event_type = "text"`: 正常可展示文本
- `payload.event_type = "thinking"`: 供应商思维过程增量（仅在供应商支持时出现）

#### `model.completed` — 模型输出完成

```json
{
  "type": "event",
  "event_type": "model.completed",
  "payload": {
    "content": "我是DeepSeek，一个由深度求索公司创造的免费AI助手！"
  }
}
```

#### `run.completed` — 运行结束

```json
{
  "type": "event",
  "event_type": "run.completed",
  "payload": {
    "status": "completed",
    "duration_ms": 3325,
    "input_tokens": 0,
    "output_tokens": 0,
    "total_tokens": 0,
    "tool_calls_made": 0
  }
}
```

#### `tool.call.request` — 请求业务端/能力执行器执行工具

```json
{
  "type": "event",
  "event_type": "tool.call.request",
  "session_id": "my_session",
  "run_id": "run_xxx",
  "trace_id": "trace_xxx",
  "timestamp": "2026-05-22T12:00:00Z",
  "payload": {
    "call_id": "call_xxx",
    "tool_name": "web_search",
    "input": { "query": "AgentKernel" },
    "timeout_ms": 5000
  },
  "event_seq": 12
}
```

#### 标准字段说明

| 字段 | 说明 |
|------|------|
| `session_id` | 所属会话 |
| `run_id` | 所属运行流程 |
| `trace_id` | 链路追踪 ID，预留 |
| `payload.call_id` | 本次工具调用唯一 ID，用于结果回填 |
| `payload.tool_name` | 工具名 |
| `payload.input` | 工具输入参数 |
| `payload.timeout_ms` | 允许等待的超时时间；`0` 表示无限等待 |

客户端收到后应：
1. 本地或转发执行对应工具
2. 通过 `tool.execute.result` 命令回传结果

#### `tool.call.result` — 工具执行结果已回填

```json
{
  "type": "event",
  "event_type": "tool.call.result",
  "session_id": "my_session",
  "run_id": "run_xxx",
  "payload": {
    "tool_name": "web_search",
    "call_id": "call_xxx",
    "result": "2026-05-22 12:00:00",
    "is_error": false
  }
}
```

#### 标准回填规则

- `call_id` 必须与 `tool.call.request` 中一致
- `result` 建议为字符串；如需返回复杂结构，请先序列化为字符串
- `is_error = true` 表示执行失败
- 该事件用于事件流、trace、debug、replay，不是再次要求业务端执行

#### `run.retrying` — Core 内部重试供应商请求

```json
{
  "type": "event",
  "event_type": "run.retrying",
  "session_id": "my_session",
  "run_id": "run_xxx",
  "payload": {
    "source": "provider",
    "stage": "model.stream",
    "attempt": 1,
    "next_attempt": 2,
    "max_retries": 10,
    "max_attempts": 11,
    "delay_ms": 734,
    "error": "error sending request for url (...) ",
    "retryable": true
  }
}
```

- 默认最多重试 10 次，可用环境变量 `AGENTKERNEL_PROVIDER_MAX_RETRIES` 覆盖；总尝试次数 = `max_retries + 1`。
- 只对可恢复错误重试：超时、连接断开、408/409/429、5xx、529/overloaded、`x-should-retry: true` 等。
- 业务端收到 `run.retrying` 时应展示“正在重试/上游波动”，不要立即判定本轮失败；最终仍以 `run.completed` / `run.failed` 为准。

#### `run.failed` — 推理失败

```json
{
  "type": "event",
  "event_type": "run.failed",
  "session_id": "my_session",
  "run_id": "run_xxx",
  "payload": {
    "error": "claude API error (400): {...}",
    "source": "provider",
    "stage": "model.stream",
    "retryable": false
  }
}
```

- `run.failed` 只表示某次对话推理失败，必须带 `session_id` 和 `run_id`。
- 业务端用于判断推理失败时，应优先监听 `run.failed`，不要依赖泛化的 `error` 类型。
- `session.send` 的失败 Response 用于结束命令等待；`run.failed` 用于事件流/UI 状态更新。两者可能描述同一次失败，但职责不同。

---

## 6. 典型接入流程

### 6.1 最小对话流程

```
1. 连接 ws://localhost:9991/ws
2. 收到 Hello Response（含 commands 列表）
3. 可选：provider.update 设置模型
4. 可选：system_prompt.set 设置系统提示词
5. 发送 session.send（带 message）
6. 监听事件流：
   - run.started → 开始
   - model.delta × N → 流式文本
   - model.completed → 完整文本
   - run.failed → 推理失败（带 session_id/run_id/error）
7. 收到 session.send Response → 本轮结束
```

### 6.2 带工具调用的流程

```
1. 连接并注册工具（tool.register）
2. 发送 session.send
3. 监听事件流：
   - run.started
   - tool_chain.diagnosed
   - model.delta（模型可能先输出文字）
   - tool.call.request → 收到工具调用请求
4. 业务端执行工具，或转发给外部能力套件执行
5. 业务端发送 tool.execute.result（带 call_id 和结果）
6. 继续监听：
   - tool.call.result
   - model.delta × N
   - model.completed
   - run.completed
7. 收到 session.send Response
```

### 6.3 外部能力套件桥接流程

适用于：业务端不想重复实现浏览器、文件、爬虫、数据库等能力，而是把 AgentKernel 下发的工具请求转发给独立能力套件。

```text
AgentKernel
  -> tool.call.request
业务端 Tool Bridge
  -> 按 tool_name/input 转发给外部能力套件
外部能力套件
  -> 返回标准 Tool Result
业务端 Tool Bridge
  -> tool.execute.result 回填 AgentKernel
AgentKernel
  -> tool.call.result + 继续模型推理
```

推荐业务端转发给外部能力套件时使用的中间协议：

```json
{
  "call_id": "call_xxx",
  "tool_name": "browser_open",
  "input": { "url": "https://example.com" },
  "context": {
    "session_id": "my_session",
    "run_id": "run_xxx",
    "trace_id": "trace_xxx",
    "timeout_ms": 10000
  }
}
```

外部能力套件返回：

```json
{
  "call_id": "call_xxx",
  "ok": true,
  "result": "页面已打开",
  "error": null,
  "metadata": {
    "duration_ms": 1200
  }
}
```

业务端再转换为 AgentKernel 标准回填：

```json
{
  "command": "tool.execute.result",
  "session_id": "my_session",
  "payload": {
    "call_id": "call_xxx",
    "result": "页面已打开",
    "is_error": false
  }
}
```

> **边界原则**: 外部能力套件不要直接依赖 AgentKernel 内部实现。它只需要理解 `call_id / tool_name / input / context`，然后返回 `call_id / ok / result / error`。

### 6.4 断线重连恢复

```
1. 本地记录最后的 event_seq
2. 重连后发送 events.pull（带 since_seq）
3. 收到缺失的事件列表
4. 继续正常监听
```

---

## 7. 注意事项与已知差异

### 7.1 字段命名易错点

| 易错点 | 正确做法 |
|--------|----------|
| `session_id` 放在 `payload` 内 | 放在消息**顶层** |
| 用 `id` 作为请求标识 | 用 `request_id` |
| `session.send` 用 `content` | 用 `message` |
| `system_prompt.set` 用 `prompt` | 用 `system_prompt` |

### 7.2 Session 自动创建

- `session.send` 发送到不存在的 session_id 时，会自动创建 session
- 无需预先调用 session 创建命令（不存在 `session.create` 命令）

### 7.3 Claude 协议工具名限制

- 工具名必须匹配 `^[a-zA-Z0-9_-]+$`
- 不能包含 `.`、空格等特殊字符
- OpenAI 协议无此限制

### 7.4 `tool.execute.result` 无标准响应

- 此命令被消费后不会返回 `send_ok`
- 前端不应依赖此命令的响应做渲染
- `tool.call.result` 才是工具闭环完成后的标准事件结果

### 7.5 Provider 配置持久化

- `provider.update` 会将配置持久化到 session metadata 文件
- 文件位置：`.aicore/sessions/<session_id>/session.json` 的 `metadata.provider_config`

### 7.6 事件流语义区分

- `event`：Core 主动推送的运行时事件（如 `run.started`、`model.delta`）
- `stream`：底层信号（如 `ping`）
- `response`：命令执行结果
- 三者不应混淆使用

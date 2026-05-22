# AgentKernel WebSocket API 文档

> **版本**: v1.0.0  
> **最后更新**: 2026-05-22  
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

### 1.3 连接示例

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
      "session.message.insert",
      "tool.register",
      "tool.unregister",
      "provider.update",
      "provider.get",
      "run.cancel",
      "runtime.sessions",
      "session.get",
      "session.info",
      "session.delete",
      "session.clear",
      "session.fork",
      "session.messages",
      "session.list",
      "system.stats",
      "context.preview",
      "context.compaction.apply",
      "system_prompt.get",
      "system_prompt.set",
      "tool.list",
      "tool.get",
      "tool.execute.result"
    ],
    "connection_id": "d748bd30-c1fa-4e08-8312-f3dbbe5f3f02",
    "server_version": "1.0.0"
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

```json
{
  "type": "response",
  "request_id": "r1",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "run_id": "run_xxx",
    "status": "completed",
    "content": "我是DeepSeek，一个由深度求索公司创造的免费AI助手！",
    "usage": { "input_tokens": 123, "output_tokens": 45 },
    "traces": 1,
    "trace_details": [...],
    "tool_calls_made": 0
  }
}
```

**执行过程中的事件流**（已验证）:

```
→ Command: session.send
← Event: run.started           (provider/model 信息)
← Event: tool_chain.diagnosed  (工具链诊断)
← Event: model.delta           (流式文本增量，多次)
← Event: model.delta
← Event: model.delta
  ...
← Event: model.completed       (完整文本)
← Event: run.completed         (运行统计)
← Response: session.send       (最终结果)
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

### 4.2 `session.message.insert` — 插入消息（不触发推理）

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

### 4.3 `provider.update` — 更新 Session 级 Provider 配置

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

### 4.4 `provider.get` — 读取 Provider 配置

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

### 4.5 `system_prompt.set` — 设置系统提示词

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

### 4.6 `system_prompt.get` — 读取系统提示词

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

### 4.7 `tool.register` — 注册工具

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
| `schema` | object | 否 | JSON Schema 定义输入参数 |
| `client_id` | string | 否 | 注册来源标识 |
| `permissions` | string[] | 否 | 权限标签 |
| `timeout_ms` | u64 | 否 | 超时毫秒数；`0` 表示无限等待 |
| `tags` | string[] | 否 | 分类标签 |

> **Claude 协议限制**: 工具名必须匹配 `^[a-zA-Z0-9_-]+$`，不能包含 `.` 等特殊字符。

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

### 4.8 `tool.unregister` — 注销工具

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

### 4.9 `tool.list` — 获取工具列表

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
          "input_schema": {}
        }
      }
    ]
  }
}
```

---

### 4.10 `tool.get` — 获取单个工具详情

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
      "input_schema": { "type": "object", "properties": {}, "required": [] }
    }
  }
}
```

---

### 4.11 `tool.execute.result` — 回传工具执行结果

**作用**: 客户端收到 `tool.call.request` 事件后，执行工具并通过此命令回传结果。

**请求**:

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
| `call_id` | string | **是** | 对应 `tool.call.request` 事件中的 `call_id` |
| `result` | string | **是** | 执行结果文本 |
| `is_error` | bool | 否 | 是否为错误结果，默认 `false` |

> **注意**: 此命令不会返回标准 Response。服务端消费后直接唤醒内部等待通道。

---

### 4.12 `session.list` — 获取 Session 列表

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
| `status` | string | 按状态过滤：`active` / `paused` / `closed` |

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

### 4.13 `session.info` — 获取 Session 详情

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

### 4.14 `session.get` — 获取 Session 简要统计

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

### 4.15 `session.messages` — 分页读取全量消息

**请求**:

```json
{
  "command": "session.messages",
  "request_id": "r14",
  "session_id": "my_session",
  "payload": {
    "page": 0,
    "limit": 50
  }
}
```

| payload 字段 | 类型 | 说明 |
|--------------|------|------|
| `page` | u64 | 页码，从 0 开始 |
| `limit` | u64 | 每页条数，最大 200 |

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
    "total": 4,
    "pages": 1,
    "messages": [
      { "message_id": "msg_1", "role": "user", "kind": "normal", "text": "你好", "content": [{"type":"text","text":"你好"}], "created_at": "..." },
      { "message_id": "msg_2", "role": "assistant", "kind": "normal", "text": "你好！", "content": [{"type":"text","text":"你好！"}], "created_at": "..." }
    ]
  }
}
```

> 每条消息包含 `text`（纯文本摘要，跳过 ToolUse 块）和 `content`（完整内容块数组，含 tool_use / tool_result 等结构化数据）。前端渲染工具调用时应使用 `content` 字段。

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

### 4.16 `session.delete` — 删除 Session

**请求**:

```json
{
  "command": "session.delete",
  "request_id": "r15",
  "session_id": "my_session",
  "payload": {}
}
```

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r15",
  "success": true,
  "payload": { "session_id": "my_session", "deleted": true }
}
```

---

### 4.17 `session.clear` — 清空上下文视图

**作用**: 清空当前 Active Context，不删除消息历史。

**请求**:

```json
{
  "command": "session.clear",
  "request_id": "r16",
  "session_id": "my_session",
  "payload": {}
}
```

**成功响应**:

```json
{
  "type": "response",
  "request_id": "r16",
  "success": true,
  "payload": {
    "session_id": "my_session",
    "cleared": true,
    "before": { "message_count": 12, "estimated_tokens": 3200 },
    "note": "messages preserved in storage, context view cleared"
  }
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

```json
{
  "type": "response",
  "request_id": "r18",
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

### 4.22 `context.reset` — 重置上下文规则

**请求**:

```json
{
  "command": "context.reset",
  "request_id": "r20",
  "session_id": "my_session",
  "payload": {}
}
```

**成功响应**: 返回新的 `active_context`（mode: `"full"`）。

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

### 4.24 `context.include_after` — 从某消息后纳入上下文

**请求**:

```json
{
  "command": "context.include_after",
  "request_id": "r22",
  "session_id": "my_session",
  "payload": {
    "message_id": "msg_xxx"
  }
}
```

**额外事件**: `context.updated`（`payload.action = "include_after"`）

---

### 4.25 `context.keep_recent` — 只保留最近 N 条

**请求**:

```json
{
  "command": "context.keep_recent",
  "request_id": "r23",
  "session_id": "my_session",
  "payload": {
    "keep": 20
  }
}
```

`keep` 传 `null` 取消该规则。

**额外事件**: `context.updated`（`payload.action = "keep_recent"`）

---

### 4.26 `context.seed.add` — 注入上下文 Seed

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
| `enabled` | bool | 是否启用 |
| `priority` | i32 | 优先级 |

**额外事件**: `context.seed.added`

> **注意**: 通过 `seed.add` 手动注入的种子，如果未被任何 ContextState 的 `base_seed_ids` 引用，在压缩后的新上下文中**不会自动注入模型输入**。只有 `compaction.apply` 创建的 summary seed 会被自动加入 `base_seed_ids`。

---

### 4.27 `context.compaction.apply` — 应用压缩摘要

**作用**: 创建压缩摘要 Seed，替换旧压缩上下文，切换 Active Context。

**行为说明**:
- 自动清除该 session 旧的 `CompactionSummary` 类型 seeds（多次压缩不会累积）
- 创建新 ContextState（mode=`compacted`），`base_seed_ids` 指向新 summary seed
- `include_after_message_id`：传入后，模型只看到该消息之后的新消息 + summary seed；**不传则全量历史仍会提交**，压缩无实际效果
- 下次 `session.send` 时，`build_model_input` 只注入 `base_seed_ids` 引用的 seeds，非引用的 seeds 不进入模型上下文

**请求**:

```json
{
  "command": "context.compaction.apply",
  "request_id": "r25",
  "session_id": "my_session",
  "payload": {
    "summary": "这是压缩后的历史摘要...",
    "include_after_message_id": "msg_xxx"
  }
}
```

**payload 字段**:

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `summary` | string | ✅ | 压缩摘要内容 |
| `include_after_message_id` | string | 可选 | 仅保留该消息之后的新消息；不传则旧消息全部保留在上下文中 |

**额外事件**: `context.compaction.applied`

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
| `run.cancelled` | 运行被中断 | - |
| `tool_chain.diagnosed` | 工具链诊断 | ✅ |
| `tool.call.request` | 请求客户端执行工具 | - |
| `tool.call.result` | 工具执行结果 | - |
| `tool.registered` | 工具注册完成 | - |
| `error` | 运行期错误 | ✅ |
| `session.created` | Session 创建 | - |
| `session.closed` | Session 关闭 | - |
| `context.threshold.reached` | 上下文 token 达阈值 | - |
| `context.compaction.applied` | 压缩摘要已应用 | - |
| `context.reset` | 上下文重置 | - |
| `context.updated` | 上下文规则更新 | - |
| `context.seed.added` | 新增 seed | - |

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

#### `tool.call.request` — 请求客户端执行工具

```json
{
  "type": "event",
  "event_type": "tool.call.request",
  "payload": {
    "tool_name": "web_search",
    "call_id": "call_xxx",
    "input": { "query": "AgentKernel" },
    "timeout_ms": 0
  }
}
```

客户端收到后应：
1. 本地执行对应工具
2. 通过 `tool.execute.result` 命令回传结果

#### `error` — 运行期错误

```json
{
  "type": "event",
  "event_type": "error",
  "payload": {
    "error": "claude API error (400): {...}",
    "source": "provider",
    "stage": "model.stream",
    "retryable": false
  }
}
```

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
   - run.completed → 统计
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
4. 客户端执行工具
5. 发送 tool.execute.result（带 call_id 和结果）
6. 继续监听：
   - tool.call.result
   - model.delta × N
   - model.completed
   - run.completed
7. 收到 session.send Response
```

### 6.3 断线重连恢复

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

### 7.5 Provider 配置持久化

- `provider.update` 会将配置持久化到 session metadata 文件
- 文件位置：`.aicore/sessions/<session_id>/session.json` 的 `metadata.provider_config`

### 7.6 事件流语义区分

- `event`：Core 主动推送的运行时事件（如 `run.started`、`model.delta`）
- `stream`：底层信号（如 `ping`）
- `response`：命令执行结果
- 三者不应混淆使用

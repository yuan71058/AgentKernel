# AgentKernel Tool Protocol 标准

> 用途：定义 AgentKernel 下发工具请求、业务端/外部能力套件执行、结果回填的统一协议。  
> 适用链路：`AgentKernel -> 业务端 Tool Bridge -> 外部能力套件 -> 业务端 -> AgentKernel`

---

## 1. 核心边界

AgentKernel 不直接执行能力，只负责：

1. 根据模型输出生成 `tool.call.request`
2. 等待业务端回传 `tool.execute.result`
3. 将工具结果写回上下文并继续模型推理
4. 发出 `tool.call.result` 事件用于调试、追踪、回放

业务端负责：

1. 接收 `tool.call.request`
2. 本地执行工具，或转发给外部能力套件
3. 将执行结果转换为 `tool.execute.result` 回填 AgentKernel

外部能力套件不需要理解 AgentKernel 内部状态，只需要理解标准 Tool Request / Tool Result。

---

## 2. AgentKernel -> 业务端：`tool.call.request`

AgentKernel 通过 WebSocket Event 下发工具调用请求。

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
    "tool_name": "browser_open",
    "input": {
      "url": "https://example.com"
    },
    "timeout_ms": 10000
  },
  "event_seq": 12
}
```

### 字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `type` | string | 是 | 固定为 `event` |
| `event_type` | string | 是 | 固定为 `tool.call.request` |
| `session_id` | string | 是 | 本次工具调用所属会话 |
| `run_id` | string | 是 | 本次工具调用所属推理流程 |
| `trace_id` | string | 否 | 链路追踪 ID，预留 |
| `payload.call_id` | string | 是 | 工具调用唯一 ID，结果回填时必须原样带回 |
| `payload.tool_name` | string | 是 | 工具名称 |
| `payload.input` | object | 是 | 工具输入参数，结构由 `tool.register.schema` 决定 |
| `payload.timeout_ms` | number | 否 | 超时时间；`0` 表示无限等待 |
| `event_seq` | number | 否 | session 内事件序号，用于断线补拉 |

---

## 3. 业务端 -> 外部能力套件：标准 Tool Request

业务端如果不自己执行工具，可以把请求转发给独立能力套件。推荐使用下面的中间格式：

```json
{
  "call_id": "call_xxx",
  "tool_name": "browser_open",
  "input": {
    "url": "https://example.com"
  },
  "context": {
    "session_id": "my_session",
    "run_id": "run_xxx",
    "trace_id": "trace_xxx",
    "timeout_ms": 10000
  }
}
```

### 字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `call_id` | string | 是 | 原始 `tool.call.request.payload.call_id` |
| `tool_name` | string | 是 | 工具名称 |
| `input` | object | 是 | 工具输入参数 |
| `context.session_id` | string | 是 | 原始 session_id |
| `context.run_id` | string | 是 | 原始 run_id |
| `context.trace_id` | string | 否 | 原始 trace_id |
| `context.timeout_ms` | number | 否 | 原始 timeout_ms |

> 外部能力套件只依赖这层协议，不直接依赖 AgentKernel WebSocket Event Envelope。

---

## 4. 外部能力套件 -> 业务端：标准 Tool Result

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

### 字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `call_id` | string | 是 | 必须与请求中的 `call_id` 一致 |
| `ok` | bool | 是 | 是否执行成功 |
| `result` | string | 是 | 成功结果；复杂对象请序列化为 JSON 字符串 |
| `error` | string/null | 否 | 失败原因；`ok=false` 时建议必填 |
| `metadata` | object | 否 | 执行耗时、截图路径、调试信息等，不直接回填模型时可保留在业务端 |

---

## 5. 业务端 -> AgentKernel：`tool.execute.result`

业务端把外部能力套件结果转换成 AgentKernel 标准回填命令。

成功：

```json
{
  "command": "tool.execute.result",
  "request_id": "tool_result_call_xxx",
  "session_id": "my_session",
  "payload": {
    "call_id": "call_xxx",
    "result": "页面已打开",
    "is_error": false
  }
}
```

失败：

```json
{
  "command": "tool.execute.result",
  "request_id": "tool_result_call_xxx",
  "session_id": "my_session",
  "payload": {
    "call_id": "call_xxx",
    "result": "浏览器打开失败：连接超时",
    "is_error": true
  }
}
```

### 字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `command` | string | 是 | 固定为 `tool.execute.result` |
| `request_id` | string | 否 | 建议填，便于业务端日志关联；当前不会收到标准 Response |
| `session_id` | string | 是 | 原始 session_id |
| `payload.call_id` | string | 是 | 原始 call_id |
| `payload.result` | string | 是 | 回填给模型的结果文本 |
| `payload.is_error` | bool | 否 | 是否错误结果 |

> 当前 `tool.execute.result` 不返回标准 Response。它被服务端消费后会唤醒内部等待通道。

---

## 6. AgentKernel -> 业务端：`tool.call.result`

AgentKernel 消费 `tool.execute.result` 后，会发出工具结果事件，供事件流、trace、debug、replay 使用。

```json
{
  "type": "event",
  "event_type": "tool.call.result",
  "session_id": "my_session",
  "run_id": "run_xxx",
  "payload": {
    "tool_name": "browser_open",
    "call_id": "call_xxx",
    "result": "页面已打开",
    "is_error": false
  }
}
```

注意：`tool.call.result` 不是新的执行请求，不要再次转发给能力套件。

---

## 7. 幂等与错误处理规范

1. `call_id` 是幂等键：同一个 `call_id` 只能最终确认一次。
2. 能力套件收到重复 `call_id` 时，应返回第一次执行结果，或返回明确的重复调用错误。
3. 业务端超时后，应回填 `is_error=true`，不要让 AgentKernel 无限等待。
4. 外部能力套件异常时，业务端应转换为：

```json
{
  "call_id": "call_xxx",
  "result": "能力套件执行失败：具体错误信息",
  "is_error": true
}
```

5. 复杂结果请序列化为字符串，例如：

```json
{
  "call_id": "call_xxx",
  "result": "{\"ok\":true,\"items\":[1,2,3]}",
  "is_error": false
}
```

---

## 8. 推荐链路

```text
1. 业务端启动并连接 AgentKernel WS
2. 业务端注册工具：tool.register
3. 用户发送消息：session.send
4. AgentKernel 下发：tool.call.request
5. 业务端根据 tool_name 路由：
   - 本地工具：直接执行
   - 外部能力：转发给能力套件
6. 能力返回标准 Tool Result
7. 业务端回填：tool.execute.result
8. AgentKernel 发出：tool.call.result
9. AgentKernel 继续模型推理并输出最终回答
```

---

## 9. 设计原则

- AgentKernel 保持轻量，不内置具体能力。
- 业务端是 Tool Bridge，负责路由、鉴权、超时、重试、结果转换。
- 外部能力套件只做能力执行，不感知 AgentKernel 内部上下文。
- 所有跨进程能力调用都必须带 `call_id`，便于幂等、追踪和回放。

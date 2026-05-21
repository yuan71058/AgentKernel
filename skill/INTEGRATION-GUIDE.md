# AgentKernel 接入指南 — 命令搭配与使用技巧

> 本文档只讲"什么时候该调什么、怎么组合"，不重复参数和返回结构。
> 详细字段说明请查阅 [API.md](./API.md)。

---

## 一、连接建立后

连接 `ws://.../ws` 后，服务端自动推送 Hello Response。

**建议立刻做**:
```
system.stats          ← 了解全局状态（session 数量、默认 provider、工具数量）
session.list          ← 拿到所有 session 列表，渲染下拉选择器
```

不需要等 Hello 返回再发，Hello 是自动推的，收到后直接发上面两条即可。

---

## 二、首次使用（无历史 Session）

如果 `session.list` 返回为空，直接让用户开始聊天：

```
session.send          ← session_id 随意取，不存在会自动创建
```

Session 会在首次 `session.send` 时自动创建，无需调用创建命令（不存在 `session.create`）。

---

## 三、切换到某个 Session（核心场景）

用户在下拉选择器中选中一个 session 后，**按顺序**做以下操作：

```
① provider.get            ← 获取该 session 的 provider 配置，回填配置表单
② system_prompt.get       ← 获取系统提示词，回填编辑区
③ tool.list               ← 获取已注册工具列表，渲染工具面板
④ context.preview         ← 获取上下文视图、统计信息、seed 列表
⑤ session.messages        ← 加载聊天历史（page=0, limit=50），渲染聊天区域
```

> **为什么要按这个顺序？**
> - ①②③ 是配置类，先拿配置可以让 UI 立刻呈现当前状态
> - ④ 是上下文概览，用于"上下文"面板
> - ⑤ 是历史消息，用于"聊天"面板
> - 它们互相无依赖，可以并发发送，但建议等响应后按序回填 UI，避免闪烁

如果还需要"全量消息"面板（分页浏览所有历史）：
```
session.messages          ← page=0, limit=200，滚动到底后再翻页
```

如果需要查看 session 详情（运行状态、provider override 状态等）：
```
session.info              ← 比 session.get 更完整
```

---

## 四、首次配置 Session（Agent 场景）

如果你在做 Agent 应用，首次使用某个 session 前需要配置好运行环境：

```
① provider.update         ← 设置模型（protocol / base_url / api_key / model）
② system_prompt.set       ← 设置系统提示词
③ tool.register × N       ← 注册需要的工具（可多次调用）
④ session.send            ← 发送第一条消息，开始推理
```

> **顺序很重要**: 先配 provider，再配 prompt，再注册工具，最后发消息。
> `provider.update` 和 `system_prompt.set` 都会自动创建 session（如果不存在）。

---

## 五、发送消息与接收回复

```
session.send              ← 发送用户消息
```

**同时监听事件流**（这些是服务端主动推送的，不需要你请求）：

| 阶段 | 收到的事件 | 你该做什么 |
|------|-----------|-----------|
| 开始 | `run.started` | UI 显示"思考中"，记录 `run_id` |
| 流式输出 | `model.delta` × N | 逐字拼接到聊天区（`payload.delta`） |
| 思维过程 | `model.delta`（`payload.event_type="thinking"`）| 可选展示到"思考"区域 |
| 模型完成 | `model.completed` | `payload.content` 是完整文本，可用于兜底 |
| 运行结束 | `run.completed` | 显示耗时、token 统计 |
| 最终响应 | `session.send` Response | `payload.content` 是最终文本 |

> **兜底逻辑**: 如果 `model.completed` 到了但聊天区还没内容，用它的 `payload.content` 兜底显示。
> 这是因为极少数情况下 `model.delta` 可能没有触发（如超短回复）。

---

## 六、带工具调用的对话

当模型决定调用工具时，事件流会插入工具调用环节：

```
session.send
  ↓
run.started
  ↓
tool_chain.diagnosed       ← 工具链诊断（调试用）
  ↓
model.delta × N            ← 模型可能先输出文字
  ↓
tool.call.request          ← ⚠️ 模型要调用工具了
  ↓
                           ← 此时你需要：
                           ←   1. 从 payload 中取 tool_name、call_id、input
                           ←   2. 本地执行工具
                           ←   3. 发送 tool.execute.result（带 call_id、result）
  ↓
tool.call.result           ← 工具结果已写入
  ↓
model.delta × N            ← 模型继续推理（可能还会调工具）
  ↓
model.completed
  ↓
run.completed
```

**关键**: 收到 `tool.call.request` 后，必须通过 `tool.execute.result` 回传结果，否则模型会一直等待。

> **工具名注意**: Claude 协议下工具名只能含 `[a-zA-Z0-9_-]`，不能有 `.`。

---

## 七、中断推理

用户点击"停止生成"时：

```
run.cancel                ← 发送取消请求（需要 run_id）
```

**后续会收到**:
1. `run.cancel` Response → `payload.status = "cancelling"`
2. `run.cancelled` 事件 → 确认中断完成
3. `session.send` Response → `payload.status = "cancelled"`，可能带 `partial_preserved` 和 `content`

> `run.cancel` 的 Response 只表示"收到请求"，不表示已中断完成。
> UI 应进入"cancelling"状态，等 `run.cancelled` 事件才算结束。

---

## 八、上下文管理

### 8.1 查看当前上下文

```
context.preview           ← 获取完整上下文视图（规则、统计、消息、seed）
```

### 8.2 手动排除污染历史

如果某段历史消息干扰了模型回复：

```
context.exclude           ← 排除指定消息区间（start_message_id ~ end_message_id）
```

排除后再发 `session.send`，模型就看不到被排除的内容了。

### 8.3 截断上下文

```
context.keep_recent       ← 只保留最近 N 条消息（keep=null 取消）
context.include_after     ← 从某条消息之后开始纳入
```

### 8.4 重置上下文

```
context.reset             ← 恢复到默认的 full 模式，清除所有排除/截断规则
```

### 8.5 注入记忆/偏好

```
context.seed.add          ← 向上下文注入一段记忆（如用户偏好、世界状态）
```

### 8.6 压缩上下文（高级）

当 token 用量过高时：

```
① session.messages        ← 读取全部历史
② （外部 AI 总结历史）     ← 生成压缩摘要
③ context.compaction.apply ← 注入摘要 + 切换到压缩模式
```

> Core 不负责压缩逻辑，只负责接受压缩结果。压缩是外部工作流。

---

## 九、断线重连

```
① 本地记录最后看到的 event_seq
② 重连后发 events.pull（since_seq = 上次的 seq）
③ 合并补拉回来的事件
④ 按"三、切换到某个 Session"的流程恢复 UI 状态
```

---

## 十、常用操作速查

| 场景 | 命令组合 |
|------|---------|
| 连接后初始化 | `system.stats` + `session.list` |
| 切换 session | `provider.get` → `system_prompt.get` → `tool.list` → `context.preview` → `session.messages` |
| 开始新对话 | `session.send`（session 不存在会自动创建） |
| 配置新 agent | `provider.update` → `system_prompt.set` → `tool.register` × N → `session.send` |
| 停止生成 | `run.cancel` → 等 `run.cancelled` |
| 查看工具列表 | `tool.list` |
| 切换模型 | `provider.update` → `session.send` |
| 清空上下文 | `context.reset`（不清历史）或 `session.clear` |
| 删除 session | `session.delete` |
| 排除历史消息 | `context.exclude` → `session.send` |
| 查看运行中的任务 | `runtime.sessions` |
| 断线恢复 | `events.pull` → 恢复 UI |

---

## 十一、常见误区

1. **不要假设 session 已存在** — 大部分带 `session_id` 的命令会自动创建 session，但 `provider.get`、`system_prompt.get` 等查询命令在 session 不存在时返回空/默认值，不会报错也不会创建。

2. **不要忽略 `tool.call.request`** — 收到后必须回传 `tool.execute.result`，否则推理会卡住。

3. **不要把 `event` 当 `response`** — `event` 是服务端主动推的运行时信号，`response` 是命令执行结果，两者处理逻辑不同。

4. **不要重复发 `session.send`** — 一个 session 同时只有一个活跃 run。上一个 run 未完成时再发 `session.send` 会导致并发冲突。先用 `run.cancel` 中断上一轮。

5. **`session.messages.full` 不是独立命令** — 这只是前端内部的追踪标签，实际发送的还是 `session.messages`。

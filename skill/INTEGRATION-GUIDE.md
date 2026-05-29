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

**排查问题时**：优先看独立通讯日志 `.aicore/logs/comm.jsonl`。它记录业务端 ↔ AgentKernel 的 WS 原始命令、响应、事件，滚动保留历史文件 `comm.1.jsonl`、`comm.2.jsonl` 等，比只看 session 消息更完整。

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
③ tool.list               ← 获取已注册工具轻量列表，渲染工具面板；单个工具详情再用 tool.get
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

如果只需要快速查看最新消息（例如判断 AI 最后一条回复）：
```
session.messages          ← page=0, limit=20, order="desc"
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
session.retry             ← 上一轮失败/中断后基于现有历史续跑，不新增 user message
```

**同时监听事件流**（这些是服务端主动推送的，不需要你请求）：

| 阶段 | 收到的事件 | 你该做什么 |
|------|-----------|-----------|
| 开始 | `run.started` | UI 显示"思考中"，记录 `run_id` |
| 流式输出 | `model.delta` × N | 逐字拼接到聊天区（`payload.delta`） |
| 思维过程 | `model.delta`（`payload.event_type="thinking"`）| 可选展示到"思考"区域 |
| 模型完成 | `model.completed` | `payload.content` 是完整文本，可用于兜底 |
| 推理失败 | `run.failed` | UI 标记失败，按 `session_id/run_id` 归属到对应会话 |
| 最终响应 | `session.send` / `session.retry` Response | `payload.content` 是最终文本 |

> **兜底逻辑**: 如果 `model.completed` 到了但聊天区还没内容，用它的 `payload.content` 兜底显示。
> 这是因为极少数情况下 `model.delta` 可能没有触发（如超短回复）。
>
> **重试逻辑**: 如果上一轮失败时最后一条有效消息不是最终 `assistant` 输出，可以发 `session.retry`。它不会新增用户消息；若工具结果已经写入历史，会带着这些 tool result 继续请求模型。

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

`context.preview` 的 `payload.messages` 可直接用于前端的“当前模型可见消息列表”。消息类型不要只看顶层 `role`，必须结合 `content[].type` 判断：

| 层级 | 字段 | 含义 |
|------|------|------|
| Message | `role` | `user` / `assistant` / `system` / `tool`；当前工具结果通常以 `role=user` + `content[].type=tool_result` 形式落盘 |
| Message | `kind` | `normal` / `tool_result` / `compaction_summary` / `context_seed` / `system_note`，用于区分消息来源/用途 |
| Content Block | `type` | `text` / `tool_use` / `tool_result` / `image` / `audio`，这是渲染消息块的核心依据 |
| Tool Use | `id` / `name` / `input` | 模型请求执行的工具调用 |
| Tool Result | `tool_use_id` / `content` / `is_error` | 业务端回填的工具结果，必须用 `tool_use_id` 关联前面的 `tool_use.id` |

**工具链聚合显示规则（前端必须做）**：

1. 以 `run_id` 作为第一层边界，只聚合同一个 `run_id` 内的工具链；不要跨 run 聚合，避免把其他轮对话混进来。
2. 在同一个 run 内，按消息顺序扫描：遇到 `assistant` 消息中的 `content[].type="tool_use"`，创建一个工具链组；一条 assistant message 里可能有多个 `tool_use`，都属于同一组。
3. 后续连续出现的 `tool_result` 块，用 `tool_result.tool_use_id == tool_use.id` 精确挂回对应工具。不要只靠相邻消息、工具名或顺序匹配。
4. 如果后面又出现新的 `assistant` + `tool_use`，且 `run_id` 相同，可以作为同一 run 的下一段工具链继续合并到该 run 组；如果 `run_id` 不同，必须新建组。
5. 工具状态建议按以下规则展示：
   - `pending` / 执行中：已有 `tool_use`，但还没有匹配到 `tool_result`。
   - `done` / 已完成：匹配到 `tool_result` 且 `is_error != true`。
   - `error` / 执行失败：匹配到 `tool_result` 且 `is_error == true`。
   - `orphan_result` / 孤立结果：有 `tool_result`，但找不到对应 `tool_use`；应单独警告展示，不要强行合并。
6. 聚合后的聊天 UI 推荐只显示一张“工具链卡片”：标题显示工具数量和整体状态；展开后逐个显示工具名、参数、结果、耗时/错误。原始消息仍保留在“全部历史/调试面板”。

> 注意：`session.messages` 和 `context.preview` 的消息结构基本一致；区别是前者看历史分页，后者看“当前模型实际可见上下文”。前端聊天区、上下文面板、历史面板都应复用同一套工具链聚合逻辑。

### 8.2 手动排除污染历史

如果某段历史消息干扰了模型回复：

```
context.exclude           ← 排除指定消息区间（start_message_id ~ end_message_id）
```

排除后再发 `session.send`，模型就看不到被排除的内容了。

### 8.3 主裁剪策略

主裁剪统一使用一个命令，同一时间只有一个 `mode` 生效：

```
context.trim.set
```

常用 payload：

```json
{ "mode": "none" }
```

```json
{ "mode": "keep_recent_messages", "keep_messages": 50 }
```

```json
{ "mode": "include_after", "message_id": "msg_xxx" }
```

```json
{
  "mode": "checkpoint",
  "trigger_max_context_messages": 300,
  "retain_recent_turns": 20
}
```

`checkpoint` 的语义是：按上下文消息数触发，按用户对话轮次保留；只在下一次 `session.send` 的用户消息边界应用，不在 AI 工具链中途裁剪，避免切断 `tool_use/tool_result`。

### 8.4 Seed 管理

```
context.seed.add   ← 新增 seed
context.seed.set   ← 按类型覆盖写入 seed
context.seed.delete ← 删除指定 seed
context.seed.clear ← 清空某类 / 全部 seeds
```

Seed 是独立于消息历史的动态前置上下文块，默认不受消息裁剪规则影响。`context.seed.set` 只写入 seed，不会自动移动裁剪边界；需要隐藏旧历史时，业务端必须再显式调用 `context.trim.set`。

协议适配时，Core 会把 seed 作为模型前置上下文处理。Claude 协议下 seed 会合并进顶层 `system`，不会进入 `messages[]`，避免触发 Claude Messages API 的 `messages[].role` 只允许 `user` / `assistant` 的限制。

### 8.6 消息裁剪


当 token 用量过高时：

```
① session.messages                      ← 读取全部历史
② （外部 AI 总结历史）                   ← 生成摘要
③ context.seed.set(kind=compaction_summary) ← 覆盖写入摘要 seed
④ context.trim.set(mode=include_after)    ← 从最后一条旧消息之后继续
```

> Core 不负责压缩逻辑，只负责 seed 管理和消息视图裁剪。压缩是外部工作流。

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
| 失败续跑 | `session.retry`（不新增用户消息，复用已落盘 tool result） |
| 查看工具列表 | `tool.list` |
| 切换模型 | `provider.update` → `session.send` |
| 清空主裁剪策略 | `context.trim.set` + `{ "mode": "none" }`（不清历史） |
| 关闭会话 | `session.close`（卸载运行态，保留历史） |
| 归档会话 | `session.archive`（默认列表隐藏，历史保留） |
| 查看归档 | `session.list` + `status: "archived"` |
| 永久删除 session | `session.delete` + `permanent: true` |
| 排除历史消息 | `context.exclude` → `session.send` |
| 插入上下文消息 | `session.message.insert`（不触发推理，下次 `session.send` 自动进入上下文） |
| 查看运行中的任务 | `runtime.sessions` |
| 断线恢复 | `events.pull` → 恢复 UI |

---

## 十一、常见误区

1. **不要假设 session 已存在** — 大部分带 `session_id` 的命令会自动创建 session，但 `provider.get`、`system_prompt.get` 等查询命令在 session 不存在时返回空/默认值，不会报错也不会创建。

2. **不要忽略 `tool.call.request`** — 收到后必须回传 `tool.execute.result`，否则推理会卡住。

3. **不要把 `event` 当 `response`** — `event` 是服务端主动推的运行时信号，`response` 是命令执行结果，两者处理逻辑不同。

4. **不要重复发 `session.send`** — 一个 session 同时只有一个活跃 run。上一个 run 未完成时再发 `session.send` 会导致并发冲突。先用 `run.cancel` 中断上一轮。

5. **`session.messages.full` 不是独立命令** — 这只是前端内部的追踪标签，实际发送的还是 `session.messages`。

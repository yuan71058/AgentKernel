<div align="center">
  <h1>🚀 AgentKernel</h1>
  <p><b>一个轻量、可嵌入、WebSocket 驱动的 AI Runtime Kernel</b></p>
  <p>
    <a href="https://github.com/cih1996/AgentKernel/stargazers"><img src="https://img.shields.io/github/stars/cih1996/AgentKernel?style=flat-square&color=blue" alt="Stars"></a>
    <a href="https://github.com/cih1996/AgentKernel/blob/main/LICENSE"><img src="https://img.shields.io/github/license/cih1996/AgentKernel?style=flat-square" alt="License"></a>
    <img src="https://img.shields.io/badge/language-Rust-orange.svg?style=flat-square" alt="Language">
  </p>
</div>

<br/>

> 💡 **让你的网页、脚本、服务端项目不再反复从 0 写 Agent Runtime。**

## 🤔 为什么做 AgentKernel？

真正麻烦的不是“调用一次 LLM”，而是构建长期运行的 Agent 时反复遇到的基建问题：Session 管理、上下文持久化、工具动态注册、流式事件统一等。

AgentKernel 的目标就是**把 AI Runtime 做成一个独立 Kernel**。未来无论是开发 Python、Go、Node.js 还是 Web 项目，业务端只需连接 WebSocket，即可跨语言复用同一套核心能力。

## 🎯 核心定位

AgentKernel 不是聊天 UI，也不是业务 Agent，而是 **Agent Runtime Kernel**。

**核心原则：Kernel 只管运行时，业务端只管编排。**

| ⚙️ AgentKernel (运行时核心) | 🧠 业务端 (应用编排) |
| :--- | :--- |
| **模型交互**：模型调用、并发调度 | **工具实现**：具体工具执行逻辑与权限 |
| **状态管理**：Session 管理、持久化存储 | **业务逻辑**：业务提示词、记忆系统提取 |
| **上下文**：Context 构建、主动暴露阈值事件 | **压缩策略**：MCP 编排、智能上下文压缩 |
| **通信协议**：WebSocket IPC、事件流分发 | **前端交互**：最终产品 UI 展示 |

## 🏗️ 架构与原理

AgentKernel 采用 **WebSocket** 作为核心双向通信协议，实现状态与控制的完全解耦：

```text
[ 业务端 Client ]  --- Command (执行指令: 发送消息/注册工具) --->  [ AgentKernel ]
[ 业务端 Client ]  <--- Event (运行状态: 模型输出/工具请求) ---  [ AgentKernel ]
```

### ✨ 核心特性

1. **🔌 工具能力动态热插拔**：无需修改 Kernel 源码。业务端通过 WS 动态注册工具定义，接收 `tool.call.request` 后在本地执行并回传结果。
2. **📚 全量历史与可控视图**：Message Log 永久保留，但提供 Active Context View。Kernel 只暴露阈值事件，不硬编码压缩策略，交由业务端自由裁量。
3. **⚡ 事件流即一等公民**：运行过程中主动推送 `model.delta`、`tool.call.request` 等状态，彻底告别轮询查询，方便调试与分布式部署。
4. **🪶 保持极致轻量**：不内置重型的记忆系统、规则库或技能市场。坚守边界，只做跨平台复用的 Runtime。

## ⚖️ 适用场景对比

| ✅ 适合 AgentKernel 的场景 | ❌ 不适合的场景 |
| :--- | :--- |
| - 给现有业务系统/Web接入 AI Runtime<br>- 开发跨语言自动化脚本系统<br>- 构建多 Agent 编排平台底层<br>- 打造类似 ComfyUI 的 Agent 运行节点 | - 只需要简单调用一次 LLM 接口<br>- 需要开箱即用的完整 Coding Agent (如 Cursor/Aider)<br>- 寻找现成的聊天 UI 产品 |

## 🚀 快速开始

只需三步，即可在本地启动 AgentKernel 调试环境：

```bash
git clone https://github.com/cih1996/AgentKernel.git
cd AgentKernel
cargo run
```

启动成功后，直接在浏览器中打开：[http://localhost:9991/](http://localhost:9991/) 即可进入 Web 调试控制台。

> 💡 WebSocket 接口地址默认为：`ws://localhost:9991/ws`

## 📦 存储结构与 API

<details>
<summary><b>📂 查看存储结构</b></summary>

当前优先使用文件式持久化，方便调试与查看全量日志（后续将引入 SQLite 作为主存储）：
```text
.aicore/
└── sessions/
    └── <session_id>/
        ├── session.json
        ├── messages.jsonl
        ├── events.jsonl
        └── ...
```
</details>

<details>
<summary><b>🔌 查看 WebSocket 协议示例</b></summary>

**发送消息**
```json
{
  "command": "session.send",
  "session_id": "debug",
  "payload": { "message": "获取当前时间" }
}
```

**注册工具**
```json
{
  "command": "tool.register",
  "session_id": "debug",
  "payload": { "tool_name": "get_time", "schema": { "type": "object" } }
}
```

**接收调用与回传结果**
```json
// Kernel -> Client (Event)
{
  "type": "event",
  "event_type": "tool.call.request",
  "payload": { "tool_name": "get_time", "call_id": "xxx" }
}

// Client -> Kernel (Command)
{
  "command": "tool.execute.result",
  "payload": { "call_id": "xxx", "result": "2026-05-18 08:30:00" }
}
```
</details>

## 📸 调试控制台截图

<table>
  <tr>
    <td align="center"><img src="assets/1.png" alt="Runtime Console" width="280"><br>Runtime Console</td>
    <td align="center"><img src="assets/2.png" alt="Session Management" width="280"><br>Session Management</td>
    <td align="center"><img src="assets/3.png" alt="Tool Runtime" width="280"><br>Tool Runtime</td>
  </tr>
  <tr>
    <td align="center"><img src="assets/4.png" alt="Event Stream" width="280"><br>Event Stream</td>
    <td align="center"><img src="assets/5.png" alt="Raw Messages" width="280"><br>Raw Messages</td>
    <td align="center"><img src="assets/6.png" alt="Config and Prompt" width="280"><br>Config and Prompt</td>
  </tr>
</table>

## 🗺️ 路线图 & 社区

- [ ] 完整的 Context 操作与 Compaction workflow
- [ ] Tool call ACK / 幂等状态查询
- [ ] SQLite 主存储与多 client 权限边界
- [ ] SDK 示例 (JS / Python / Go)

**License:** MIT  
**社区交流:** QQ群 `250892941`

[![Star History Chart](https://api.star-history.com/svg?repos=cih1996/AgentKernel&type=Date)](https://www.star-history.com/#cih1996/AgentKernel&Date)

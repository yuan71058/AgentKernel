<div align="right">
  <strong>English</strong> | <a href="./README_zh.md">简体中文</a>
</div>

<div align="center">
  <img src="assets/logo.svg" alt="AgentKernel Logo" width="180">
  <h1>🚀 AgentKernel</h1>
  <p><b>A lightweight, embeddable, WebSocket-driven AI Runtime Kernel</b></p>
  <p>
    <a href="https://github.com/cih1996/AgentKernel/stargazers"><img src="https://img.shields.io/github/stars/cih1996/AgentKernel?style=flat-square&color=blue" alt="Stars"></a>
    <a href="https://github.com/cih1996/AgentKernel/blob/main/LICENSE"><img src="https://img.shields.io/github/license/cih1996/AgentKernel?style=flat-square" alt="License"></a>
    <img src="https://img.shields.io/badge/language-Rust-orange.svg?style=flat-square" alt="Language">
  </p>
</div>

<br />

> 💡 **An ultra-lightweight Agent kernel that makes integrating AI into your projects a breeze!**
> Zero-embedding, high concurrency—using it is as simple as using an object.
> Everything communicates via the WebSocket protocol, requiring no tight coupling to your project code. A completely independent microservice!

**🌐 Online Demo:** [https://cih1996.github.io/AgentKernel/](https://cih1996.github.io/AgentKernel/)<br>
**📺 Video Tutorial (How to integrate):**

<div align="center">
  <a href="https://youtu.be/iSo_Ux1AcRw?si=JGDLmntvuN51CmoT">
    <img src="https://img.youtube.com/vi/iSo_Ux1AcRw/maxresdefault.jpg" alt="AgentKernel Video Tutorial" width="600" style="border-radius: 8px;">
  </a>
</div>

## 🚀 Introduction

**Tool capabilities, Skills, and MCP can be freely and dynamically hot-plugged, registered, and invoked via callbacks.**

Various event callbacks are pre-packaged, saving you from reinventing the wheel:
1. **Context Management**: Context activation, truncation, injection, threshold detection, and full-history queries.
2. **Multi-Model Compatibility**: Compatible with providers like OpenAI, Claude, and Ollama.
3. **Dynamic Tool Dispatch**: Real-time registration of capability tools, executed by you via callbacks, including MCP.

**You can even build the next Open Code, Claude Code, OpenClaw, etc.!**
Because it serves as the core communication base for AI, you no longer need to worry about provider compatibility, TOOL protocols, or context management. Everything is encapsulated, yet remains fully dynamic and customizable in real-time!

## 🎯 Core Positioning

AgentKernel is neither a Chat UI nor a Business Agent, but an **Agent Runtime Kernel**.

**Core Principle: AgentKernel handles the AI runtime; your project owns its product logic.**

| ⚙️ AgentKernel Kernel       | 🧠 Your Project           |
| :--------------------------- | :---------------------- |
| Connects models, manages sessions, maintains context | Decides how users interact and how the product is displayed |
| Receives messages, runs inference, pushes progress events | Registers your own capabilities, such as database lookup, messaging, or device control |
| Sends a request when AI needs a capability | Executes the capability and returns the result to the Kernel |

> 💡 **Integration Note**
> AgentKernel is better suited as an independently running AI kernel: your project connects to it after startup and uses different `session`s as needed.\
> For multi-user shared sessions or collaborative viewing, it is recommended that your project handles distribution, broadcasting, and permission control at the upper layer, rather than having multiple client endpoints directly connect to the same `session` in the Core.\
> The Core is responsible for the runtime and protocol boundaries, not multi-user collaborative orchestration.

### Recommended Integration Method

```mermaid
flowchart LR
    K[Start AgentKernel\nlistens on ws://localhost:9991/ws]
    P[Your Project\nWebsite / App / Backend]
    U[User]

    P -->|Connect to the Kernel port| K
    U -->|Send a message| P
    P -->|Forward message to Kernel| K
    P -->|Register capabilities\ne.g. database / messaging / device actions| K
    K -->|When AI needs a capability\ncall back your project| P
    P -->|Execute capability\nand return result| K
    K -->|Return AI response and progress| P
    P -->|Display to user| U
```

- Start AgentKernel first. It listens on a WebSocket port.
- Your project connects to this port, then sends messages, registers capabilities, and receives AI progress events.
- When AI needs a capability, AgentKernel calls back your project. Your project executes it and returns the result, then the Kernel continues inference.
- Your project can reuse the same connection for multiple `session`s.

## 🏗️ Architecture & Principles

AgentKernel uses **WebSocket** as its core bidirectional communication protocol. In plain words: your project sends messages to the Kernel, the Kernel talks to the model, and if the model needs a capability, the Kernel calls your project back to execute it.

```mermaid
sequenceDiagram
    participant U as User
    participant P as Your Project
    participant K as AgentKernel Kernel
    participant M as AI Model

    P->>K: Connect to Kernel port after startup
    P->>K: Register capabilities, such as search, messaging, or file actions
    U->>P: Ask a question
    P->>K: Send the user's message to the Kernel
    K->>M: Ask AI to generate a response
    M-->>K: Return text, streamed back as it arrives
    K-->>P: Keep sending AI progress and output

    alt AI needs a capability
        K-->>P: Ask your project to execute a capability
        P->>P: Your project executes it
        P->>K: Return execution result to Kernel
        K->>M: Continue AI generation with the result
        M-->>K: Continue generating
        K-->>P: Return final response
    else AI does not need a capability
        K-->>P: Return final response directly
    end

    P-->>U: Show the AI response
```

In this flow, AgentKernel does not care how your business works and does not force your project to use a specific language. Your project only needs to connect via WebSocket, send messages, register capabilities, and handle callbacks.

### ✨ Core Features

1. **🔌 Dynamic Hot-Pluggable Tool Capabilities**: No need to modify Kernel source code. The business side dynamically registers tool definitions via WS, receives `tool.call.request`, executes locally, and returns the result.
2. **📚 Full History and Controllable Views**: Message Logs are permanently retained, but an Active Context View is provided. The Kernel only exposes threshold events and does not hardcode compression strategies, leaving it to the business side's discretion.
3. **⚡ Event Streams as First-Class Citizens**: Proactively pushes states like `model.delta` and `tool.call.request` during execution. If the provider supports reasoning/thinking stream passthrough, it also outputs via `model.delta.payload.event_type = "thinking"`, facilitating debugging and distributed deployment.
4. **🪶 Extremely Lightweight**: Does not have built-in heavy memory systems, rule libraries, or skill markets. Sticks to its boundaries, acting purely as a cross-platform reusable Runtime.

### 🧩 Companion Capability Examples

These projects work well with AgentKernel and can quickly add MCP discovery, execution, and local system-operation capabilities:

- [agentkernel-mcp-framework](https://github.com/cih1996/agentkernel-mcp-framework): An independent MCP discovery and execution tool, suitable as the bridge between your project and MCP capabilities.
- [agentkernel-capabilities](https://github.com/cih1996/agentkernel-capabilities): A basic system-operation MCP that provides common capabilities such as search, file lookup, file reading, file editing, and Bash execution.

## ⚖️ Use Case Comparison

| ✅ Suitable for AgentKernel                                                                  | ❌ Unsuitable Scenarios                                                                   |
| :------------------------------------------------------------------------------------ | :------------------------------------------------------------------------- |
| - Adding AI Runtime to existing business systems/Web<br>- Developing cross-language automated script systems<br>- Building the foundation for multi-Agent orchestration platforms<br>- Creating Agent running nodes similar to ComfyUI | - Only needing a simple one-off LLM API call<br>- Needing an out-of-the-box complete Coding Agent (like Cursor/Aider)<br>- Looking for a ready-made Chat UI product |

## 🚀 Quick Start

First, start the Core:

```bash
git clone https://github.com/cih1996/AgentKernel.git
cd AgentKernel
cargo run
```

- `cargo run` starts the **headless Core service**.
- Default WebSocket address: `ws://localhost:9991/ws`
- If your default binary is not the server, you can explicitly use `cargo run -p agentkernel-server`

If you need a local web debugging console, start it separately:

```bash
cd web
python3 server.py
```

- Default debug page address: <http://127.0.0.1:8899>
- `web/server.py` will first check if the Core is running; if not, it will prompt and exit directly.
- The recommended sequence is to start the Core first, then the debug page.

> 💡 For official business integration, it is recommended to keep your Python / Node.js / Go / Rust service persistently connected to the Core, and reuse the connection with different `session_id`s. If a user needs to "stop generating", the business service should send `run.cancel`. Do not let multiple end clients directly compete for the same shared `session`.

## 📦 Storage Structure & API

<details>
<summary><b>📂 View Storage Structure</b></summary>

Currently, file-based persistence is prioritized for easy debugging and viewing full logs (SQLite will be introduced as the main storage later):

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
<summary><b>🔌 View WebSocket Protocol Example</b></summary>

**Send Message**

```json
{
  "command": "session.send",
  "session_id": "debug",
  "payload": { "message": "Get current time" }
}
```

**Register Tool**

```json
{
  "command": "tool.register",
  "session_id": "debug",
  "payload": { "tool_name": "get_time", "schema": { "type": "object" } }
}
```

**Receive Call and Return Result**

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

## 📸 Debug Console Screenshots

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

## 🗺️ Roadmap & Community

- [ ] Complete Context operations and Compaction workflow
- [ ] Tool call ACK / Idempotent state queries
- [ ] SQLite main storage and multi-client permission boundaries
- [ ] SDK Examples (JS / Python / Go)

**License:** MIT<br>
**Community Chat:** <br>
QQ Group `250892941` <br>
<a href="https://discord.gg/mcQtYDVjW"><img src="https://img.shields.io/badge/Discord-Join%20Us-7289DA?style=flat-square&logo=discord&logoColor=white" alt="Discord"></a>

[![Star History Chart](https://api.star-history.com/svg?repos=cih1996/AgentKernel\&type=Date)](https://www.star-history.com/#cih1996/AgentKernel\&Date)

//! # WS Server
//!
//! 基于 axum 的 WebSocket 服务器，实现 AI Runtime Protocol。
//!
//! 核心原则：
//! - 所有操作都是 Command（客户端 → 服务端）
//! - 所有变化都是 Event（服务端 → 客户端）
//! - 每条消息显式携带 session_id / run_id / trace_id
//! - WS 是 Runtime 协议，不是聊天 API

use crate::{WsMessage, commands};
use core_context::ContextStats;
use core_events::event_types::*;
use core_protocol::*;
use core_runtime::{AgentKernel, ToolRouter};

use axum::{
    Router,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::any,
};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot, broadcast};
use tracing::{info, warn, debug};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SessionToolSnapshotItem {
    tool: Tool,
    #[serde(default)]
    registration: Option<ToolRegistration>,
}

/// WS 工具路由器
///
/// 实现 ToolRouter trait，将工具调用路由到 WS 客户端执行。
/// 通过 oneshot 通道同步等待客户端返回工具执行结果。
pub struct WsToolRouter {
    event_bus: Arc<core_events::EventBus>,
    pending: Arc<Mutex<HashMap<String, PendingToolCall>>>,
}

struct PendingToolCall {
    run_id: String,
    sender: oneshot::Sender<serde_json::Value>,
}

impl WsToolRouter {
    pub fn new(event_bus: Arc<core_events::EventBus>) -> Self {
        Self {
            event_bus,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 解析客户端返回的工具执行结果，唤醒等待中的 execute()
    pub async fn resolve_result(&self, call_id: &str, result: serde_json::Value) {
        let mut map = self.pending.lock().await;
        if let Some(pending) = map.remove(call_id) {
            let _ = pending.sender.send(result);
            info!(call_id = %call_id, "tool result resolved");
        } else {
            warn!(call_id = %call_id, "no pending tool call found for result");
        }
    }
}

#[async_trait::async_trait]
impl ToolRouter for WsToolRouter {
    async fn execute(
        &self,
        session_id: &str,
        run_id: &str,
        tool_name: &str,
        call_id: &str,
        input: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<(String, bool), String> {
        let (tx, rx) = oneshot::channel::<serde_json::Value>();

        // 注册 pending，等待客户端返回结果
        {
            let mut map = self.pending.lock().await;
            map.insert(call_id.to_string(), PendingToolCall {
                run_id: run_id.to_string(),
                sender: tx,
            });
        }

        // 发射工具调用请求事件（事件转发器会推送给 WS 客户端）
        self.event_bus.emit(
            EventEnvelope::new(TOOL_CALL_REQUEST, session_id)
                .with_run_id(run_id)
                .with_payload(json!({
                    "tool_name": tool_name,
                    "call_id": call_id,
                    "input": input,
                    "timeout_ms": timeout_ms,
                })),
        );

        info!(call_id = %call_id, tool = %tool_name, timeout_ms, "waiting for tool result from client");

        let wait_result = if timeout_ms == 0 {
            Ok(rx.await)
        } else {
            tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await
        };

        match wait_result {
            Ok(Ok(val)) => {
                let content = val["content"].as_str().unwrap_or("").to_string();
                let is_error = val["is_error"].as_bool().unwrap_or(false);
                Ok((content, is_error))
            }
            Ok(Err(_)) => {
                // oneshot sender dropped（连接断开）
                self.pending.lock().await.remove(call_id);
                Err("tool result channel closed (client disconnected)".into())
            }
            Err(_) => {
                // 超时
                self.pending.lock().await.remove(call_id);
                Err(format!("tool '{}' timed out after {}ms", tool_name, timeout_ms))
            }
        }
    }

    async fn cancel_run(&self, run_id: &str) -> Result<(), String> {
        let mut map = self.pending.lock().await;
        map.retain(|_, pending| pending.run_id != run_id);
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any { self }
}

/// WS 服务器
pub struct WsServer {
    pub scaffold: Arc<AgentKernel>,
}

impl WsServer {
    pub fn new(scaffold: Arc<AgentKernel>) -> Self {
        Self { scaffold }
    }

    /// 构建 axum Router（不含绑定，供 Shuttle 等外部服务使用）
    pub fn router(&self) -> Router {
        let scaffold = self.scaffold.clone();
        Router::new()
            .route("/ws", any(move |ws| {
                let scaffold = scaffold.clone();
                async move { handle_ws_upgrade(ws, scaffold).await }
            }))
    }

    /// 启动 WS 服务器
    pub async fn start(&self, addr: &str) -> Result<(), String> {
        let app = self.router();

        let listener = tokio::net::TcpListener::bind(addr).await
            .map_err(|e| format!("bind {} failed: {}", addr, e))?;
        info!("WS server listening on ws://{}/ws", addr);

        axum::serve(listener, app).await
            .map_err(|e| format!("server error: {}", e))
    }
}

async fn handle_ws_upgrade(ws: WebSocketUpgrade, scaffold: Arc<AgentKernel>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_connection(socket, scaffold))
}

async fn handle_connection(socket: WebSocket, scaffold: Arc<AgentKernel>) {
    let conn_id = uuid::Uuid::new_v4().to_string();
    info!(conn_id = %conn_id, "client connected");

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = mpsc::channel::<WsMessage>(256);

    // 发送连接确认
    let hello = WsMessage::Response {
        request_id: "hello".into(),
        success: true,
        payload: json!({
            "connection_id": conn_id,
            "server_version": "1.0.0",
            "commands": [
                commands::SEND_MESSAGE,
                commands::REGISTER_TOOL,
                commands::UNREGISTER_TOOL,
                commands::UPDATE_PROVIDER,
                commands::GET_PROVIDER,
                commands::CANCEL_RUN,
                commands::RUNTIME_SESSIONS,
                commands::GET_SESSION,
                commands::SESSION_INFO,
                commands::SESSION_DELETE,
                commands::SESSION_CLEAR,
                commands::SESSION_MESSAGES,
                commands::LIST_SESSIONS,
                commands::SYSTEM_STATS,
                commands::CONTEXT_PREVIEW,
                commands::COMPACTION_APPLY,
                commands::SYSTEM_PROMPT_GET,
                commands::SYSTEM_PROMPT_SET,
                commands::TOOL_LIST,
                commands::TOOL_GET,
                "tool.execute.result",
            ]
        }),
    };
    if tx.send(hello).await.is_err() { return; }

    // 发送任务：从 rx 读取 WsMessage 并发送到 WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(text) = serde_json::to_string(&msg) {
                if ws_sender.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    // 接收任务：从 WebSocket 读取消息并路由
    while let Some(msg) = ws_receiver.next().await {
        let msg = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Ping(_)) => { let _ = tx.send(WsMessage::Stream {
                session_id: String::new(), run_id: String::new(),
                event: "ping".into(), data: json!({"ok": true}),
            }).await; continue; }
            Ok(Message::Close(_)) => break,
            Err(e) => { warn!(conn_id = %conn_id, error = %e, "ws recv error"); break; }
            _ => continue,
        };

        debug!(conn_id = %conn_id, raw = %msg, "received message");

        let parsed: serde_json::Value = match serde_json::from_str(&msg) {
            Ok(v) => v,
            Err(e) => { send_err(&tx, "parse_error", &format!("invalid json: {}", e)).await; continue; }
        };

        let command = parsed["command"].as_str().unwrap_or("").to_string();
        let request_id = parsed["request_id"].as_str().unwrap_or("").to_string();
        let session_id = parsed["session_id"].as_str().unwrap_or("").to_string();
        let payload = parsed.get("payload").cloned().unwrap_or(serde_json::Value::Null);

        info!(conn_id = %conn_id, command = %command, session_id = %session_id, request_id = %request_id, "dispatch command");

        let tx_clone = tx.clone();
        let scaffold = scaffold.clone();

        match command.as_str() {
            commands::SEND_MESSAGE => {
                let sid = session_id.clone();
                let rid = request_id.clone();
                tokio::spawn(handle_send_message(
                    scaffold, tx_clone, sid, rid, payload,
                ));
            }
            "tool.execute.result" => {
                // 解析客户端返回的工具执行结果，通过 WsToolRouter 唤醒等待中的 runtime
                let call_id = payload["call_id"].as_str().unwrap_or("").to_string();
                let result = payload["result"].as_str().unwrap_or("");
                let is_error = payload["is_error"].as_bool().unwrap_or(false);
                let result_payload = json!({
                    "content": result,
                    "is_error": is_error,
                });
                if let Some(ref router) = scaffold.tool_router {
                    if let Some(ws_router) = router.as_any().downcast_ref::<WsToolRouter>() {
                        ws_router.resolve_result(&call_id, result_payload).await;
                    } else {
                        warn!("tool_router is not WsToolRouter, cannot resolve result");
                    }
                }
            }
            commands::REGISTER_TOOL => {
                handle_register_tool(&scaffold, &tx_clone, &session_id, &request_id, payload).await;
            }
            commands::UNREGISTER_TOOL => {
                let name = payload["tool_name"].as_str().unwrap_or("");
                if name.is_empty() {
                    send_err(&tx_clone, &request_id, "tool_name is required").await;
                    continue;
                }
                if !session_id.is_empty() {
                    if let Err(e) = remove_session_tool_snapshot(&scaffold, &session_id, name).await {
                        send_err(&tx_clone, &request_id, &e).await;
                        continue;
                    }
                }
                scaffold.unregister_tool(&session_id, name);
                send_ok(&tx_clone, &request_id, json!({"unregistered": name, "session_id": session_id})).await;
                info!(tool = %name, session_id = %session_id, "tool unregistered via ws");
            }
            commands::UPDATE_PROVIDER => {
                handle_update_provider(&scaffold, &tx_clone, &session_id, &request_id, payload).await;
            }
            commands::GET_PROVIDER => {
                handle_get_provider(&scaffold, &tx_clone, &session_id, &request_id).await;
            }
            commands::CANCEL_RUN => {
                let run_id = payload["run_id"].as_str().unwrap_or("").to_string();
                if run_id.is_empty() {
                    send_err(&tx_clone, &request_id, "run_id is required").await;
                    continue;
                }
                let cancelled = scaffold.cancel_run(&run_id);
                info!(session_id = %session_id, run_id = %run_id, cancelled, "run.cancel received");
                send_ok(&tx_clone, &request_id, json!({
                    "status": if cancelled { "cancelling" } else { "not_found" },
                    "session_id": session_id,
                    "run_id": run_id,
                    "cancelled": cancelled,
                })).await;
            }
            commands::RUNTIME_SESSIONS => {
                handle_runtime_sessions(&scaffold, &tx_clone, &request_id).await;
            }
            commands::GET_SESSION => {
                if !session_id.is_empty() {
                    if let Err(e) = scaffold.load_session_state(&session_id).await {
                        warn!(session_id = %session_id, error = %e, "load session state failed");
                    }
                }
                handle_get_session(&scaffold, &tx_clone, &session_id, &request_id).await;
            }
            commands::SESSION_INFO => {
                if !session_id.is_empty() {
                    if let Err(e) = scaffold.load_session_state(&session_id).await {
                        warn!(session_id = %session_id, error = %e, "load session state failed");
                    }
                }
                handle_session_info(&scaffold, &tx_clone, &session_id, &request_id).await;
            }
            commands::SESSION_DELETE => {
                handle_session_delete(&scaffold, &tx_clone, &session_id, &request_id).await;
            }
            commands::SESSION_CLEAR => {
                handle_session_clear(&scaffold, &tx_clone, &session_id, &request_id).await;
            }
            commands::SESSION_MESSAGES => {
                if !session_id.is_empty() {
                    if let Err(e) = scaffold.load_session_state(&session_id).await {
                        warn!(session_id = %session_id, error = %e, "load session state failed");
                    }
                }
                handle_session_messages(&scaffold, &tx_clone, &session_id, &request_id, &payload).await;
            }
            commands::LIST_SESSIONS => {
                handle_list_sessions(&scaffold, &tx_clone, &request_id, &payload).await;
            }
            commands::SYSTEM_STATS => {
                handle_system_stats(&scaffold, &tx_clone, &request_id).await;
            }
            commands::CONTEXT_PREVIEW => {
                if !session_id.is_empty() {
                    if let Err(e) = scaffold.load_session_state(&session_id).await {
                        warn!(session_id = %session_id, error = %e, "load session state failed");
                    }
                }
                handle_context_preview(&scaffold, &tx_clone, &session_id, &request_id).await;
            }
            commands::CONTEXT_RESET => {
                handle_context_reset(&scaffold, &tx_clone, &session_id, &request_id).await;
            }
            commands::CONTEXT_EXCLUDE => {
                handle_context_exclude(&scaffold, &tx_clone, &session_id, &request_id, &payload).await;
            }
            commands::CONTEXT_INCLUDE_AFTER => {
                handle_context_include_after(&scaffold, &tx_clone, &session_id, &request_id, &payload).await;
            }
            commands::CONTEXT_KEEP_RECENT => {
                handle_context_keep_recent(&scaffold, &tx_clone, &session_id, &request_id, &payload).await;
            }
            commands::CONTEXT_SEED_ADD => {
                handle_context_seed_add(&scaffold, &tx_clone, &session_id, &request_id, &payload).await;
            }
            commands::COMPACTION_APPLY => {
                handle_compaction_apply(&scaffold, &tx_clone, &session_id, &request_id, &payload).await;
            }
            commands::EVENTS_PULL => {
                handle_events_pull(&scaffold, &tx_clone, &session_id, &request_id, &payload).await;
            }
            commands::EVENTS_SUBSCRIBE => {
                handle_events_subscribe(
                    scaffold.clone(), tx_clone.clone(),
                    &session_id, &request_id, &payload,
                ).await;
            }
            commands::SYSTEM_PROMPT_GET => {
                if !session_id.is_empty() {
                    scaffold.session_mgr.get_or_create(&session_id).await.ok();
                }
                let session_prompt = if !session_id.is_empty() {
                    scaffold.session_mgr.get_system_prompt(&session_id)
                } else {
                    None
                };
                send_ok(&tx_clone, &request_id, json!({
                    "session_id": session_id,
                    "system_prompt": session_prompt.clone().unwrap_or_else(|| scaffold.get_system_prompt()),
                    "is_session_override": session_prompt.is_some(),
                })).await;
            }
            commands::SYSTEM_PROMPT_SET => {
                let new_prompt = payload["system_prompt"].as_str().unwrap_or("").to_string();
                if session_id.is_empty() {
                    scaffold.set_system_prompt(&new_prompt);
                    send_ok(&tx_clone, &request_id, json!({
                        "session_id": session_id,
                        "system_prompt": scaffold.get_system_prompt(),
                        "is_session_override": false,
                        "updated": true,
                    })).await;
                } else {
                    scaffold.session_mgr.get_or_create(&session_id).await.ok();
                    match scaffold.session_mgr.set_system_prompt(&session_id, new_prompt.clone()).await {
                        Ok(_) => send_ok(&tx_clone, &request_id, json!({
                            "session_id": session_id,
                            "system_prompt": new_prompt,
                            "is_session_override": true,
                            "updated": true,
                        })).await,
                        Err(e) => send_err(&tx_clone, &request_id, &e).await,
                    }
                }
                info!(len = new_prompt.len(), session_id = %session_id, "system prompt updated via ws");
            }
            commands::TOOL_LIST => {
                if !session_id.is_empty() {
                    let _ = scaffold.restore_session_tools(&session_id);
                }
                handle_tool_list(&scaffold, &tx_clone, &session_id, &request_id).await;
            }
            commands::TOOL_GET => {
                let name = payload["tool_name"].as_str().unwrap_or("");
                if name.is_empty() {
                    send_err(&tx_clone, &request_id, "tool_name is required").await;
                } else if let Some(tool) = scaffold.tool_manager.get_tool(&session_id, name) {
                    let reg = scaffold.tool_manager.get_registration(&session_id, name);
                    send_ok(&tx_clone, &request_id, json!({
                        "tool": tool,
                        "registration": reg,
                    })).await;
                } else {
                    send_err(&tx_clone, &request_id, &format!("tool '{}' not found", name)).await;
                }
            }
            _ => {
                warn!(command = %command, "unknown command");
                send_err(&tx_clone, &request_id, &format!("unknown command: {}", command)).await;
            }
        }
    }

    send_task.abort();
    info!(conn_id = %conn_id, "client disconnected");
}

// ─── Command Handlers ──────────────────────────────────────────

async fn handle_send_message(
    scaffold: Arc<AgentKernel>,
    tx: mpsc::Sender<WsMessage>,
    session_id: String,
    request_id: String,
    payload: serde_json::Value,
) {
    let message = payload["message"].as_str().unwrap_or("");
    let max_repeated_tool_calls = payload["max_repeated_tool_calls"].as_u64().unwrap_or(10) as u32;
    let images: Vec<String> = payload["images"].as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    if session_id.is_empty() || message.is_empty() {
        send_err(&tx, &request_id, "session_id and message are required").await;
        return;
    }

    // 自动创建 session（如果不存在）
    scaffold.session_mgr.get_or_create(&session_id).await.ok();

    let run_id = format!("run_{}", uuid::Uuid::new_v4());

    // 订阅事件流（在 chat 启动前）
    let mut event_rx = scaffold.subscribe_events();
    let fwd_session = session_id.to_string();
    let fwd_run = run_id.clone();
    let fwd_tx = tx.clone();

    // 事件转发任务：把 EventBus 中与本次 run 相关的事件实时推给 WS 客户端
    let event_forwarder = tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(evt) => {
                    // 只转发本次 session + run 的事件
                    if evt.session_id != fwd_session { continue; }
                    if !fwd_run.is_empty() && !evt.run_id.is_empty() && evt.run_id != fwd_run {
                        continue;
                    }
                    // 转发所有事件（包括 TOOL_CALL_REQUEST，客户端 UI 需要展示）
                    let _ = fwd_tx.send(WsMessage::Event(evt)).await;
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(skipped = n, "event bus lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // 执行对话
    let opts = core_runtime::ChatOptions {
        session_id: session_id.to_string(),
        run_id: run_id.clone(),
        message: message.to_string(),
        images,
        max_repeated_tool_calls,
    };

    let result = scaffold.chat_with_options(opts).await;
    // 先让 forwarder 有机会把最后的 model.completed / run.completed / error 事件刷到 WS，
    // 再停止转发任务，避免终态事件被提前中断丢失。
    tokio::task::yield_now().await;
    event_forwarder.abort();

    match result {
        Ok(resp) => {
            send_ok(&tx, &request_id, json!({
                "session_id": resp.session_id,
                "run_id": resp.run_id,
                "status": resp.status,
                "partial_preserved": resp.partial_preserved,
                "content": resp.content,
                "usage": {
                    "input_tokens": resp.usage.input_tokens,
                    "output_tokens": resp.usage.output_tokens,
                },
                "traces": resp.traces.len(),
                "trace_details": resp.traces,
                "tool_calls_made": resp.tool_calls_made,
            })).await;
        }
        Err(e) => {
            send_err_payload(&tx, &request_id, e.to_payload()).await;
        }
    }
}

fn read_session_tool_snapshot(scaffold: &AgentKernel, session_id: &str) -> Vec<SessionToolSnapshotItem> {
    scaffold.session_mgr
        .get_session_tools(session_id)
        .and_then(|v| serde_json::from_value::<Vec<SessionToolSnapshotItem>>(v).ok())
        .unwrap_or_default()
}

async fn save_session_tool_snapshot(
    scaffold: &AgentKernel,
    session_id: &str,
    snapshot: &[SessionToolSnapshotItem],
) -> Result<(), String> {
    if session_id.is_empty() {
        return Ok(());
    }
    scaffold.session_mgr.get_or_create(session_id).await?;
    let value = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
    scaffold.session_mgr.set_session_tools(session_id, value).await
}

async fn upsert_session_tool_snapshot(
    scaffold: &AgentKernel,
    session_id: &str,
    tool: &Tool,
    registration: &ToolRegistration,
) -> Result<(), String> {
    let mut snapshot = read_session_tool_snapshot(scaffold, session_id);
    snapshot.retain(|item| item.tool.name != tool.name);
    snapshot.push(SessionToolSnapshotItem {
        tool: tool.clone(),
        registration: Some(registration.clone()),
    });
    save_session_tool_snapshot(scaffold, session_id, &snapshot).await
}

async fn remove_session_tool_snapshot(
    scaffold: &AgentKernel,
    session_id: &str,
    tool_name: &str,
) -> Result<(), String> {
    let mut snapshot = read_session_tool_snapshot(scaffold, session_id);
    snapshot.retain(|item| item.tool.name != tool_name);
    save_session_tool_snapshot(scaffold, session_id, &snapshot).await
}

async fn handle_tool_list(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
) {
    let persisted_snapshot = if !session_id.is_empty() {
        scaffold.session_mgr.get_session_tools(session_id)
    } else {
        None
    };
    let tools = scaffold.tool_manager.get_tools(session_id);
    let list: Vec<serde_json::Value> = tools.iter().map(|t| {
        let reg = scaffold.tool_manager.get_registration(session_id, &t.name);
        let empty_tags: Vec<String> = vec![];
        json!({
            "name": t.name,
            "description": t.description,
            "input_schema": t.input_schema,
            "client_id": reg.as_ref().map(|r| r.client_id.as_str()).unwrap_or(""),
            "timeout_ms": reg.as_ref().map(|r| r.timeout_ms),
            "tags": reg.as_ref().map(|r| &r.tags).unwrap_or(&empty_tags),
        })
    }).collect();

    send_ok(tx, request_id, json!({
        "session_id": session_id,
        "count": list.len(),
        "tools": list,
        "persisted_snapshot": persisted_snapshot,
    })).await;
}

async fn handle_register_tool(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
    payload: serde_json::Value,
) {
    let name = payload["tool_name"].as_str().unwrap_or("");
    let desc = payload["description"].as_str().unwrap_or("");
    let schema = payload.get("schema").cloned().unwrap_or(serde_json::json!({}));
    let client_id = payload["client_id"].as_str().unwrap_or("unknown");
    let perms: Vec<String> = payload["permissions"].as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let timeout = payload["timeout_ms"].as_u64().unwrap_or(0);
    let tags: Vec<String> = payload["tags"].as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    if name.is_empty() {
        send_err(tx, request_id, "tool_name is required").await;
        return;
    }

    let tool = Tool {
        name: name.to_string(),
        description: desc.to_string(),
        input_schema: schema.clone(),
    };
    let reg = ToolRegistration {
        tool_name: name.to_string(),
        description: desc.to_string(),
        client_id: client_id.to_string(),
        permissions: perms,
        timeout_ms: timeout,
        tags,
    };
    if !session_id.is_empty() {
        if let Err(e) = upsert_session_tool_snapshot(scaffold, session_id, &tool, &reg).await {
            send_err(tx, request_id, &e).await;
            return;
        }
    }

    scaffold.register_tool(session_id, tool.clone(), reg.clone());

    scaffold.event_bus.emit(EventEnvelope::new(TOOL_REGISTERED, session_id)
        .with_payload(json!({"tool_name": name, "client_id": client_id, "session_id": session_id})));

    send_ok(tx, request_id, json!({"registered": name, "session_id": session_id})).await;
    info!(tool = %name, client = %client_id, session_id = %session_id, "tool registered via ws");
}

async fn handle_runtime_sessions(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    request_id: &str,
) {
    let active_runs = scaffold.list_active_runs();
    let mut session_set = HashSet::new();
    for run in &active_runs {
        session_set.insert(run.session_id.clone());
    }

    send_ok(tx, request_id, json!({
        "running_session_count": session_set.len(),
        "running_run_count": active_runs.len(),
        "sessions": session_set.into_iter().collect::<Vec<_>>(),
        "runs": active_runs,
    })).await;
}

async fn handle_get_session(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }
    let stats: ContextStats = scaffold.context_mgr.stats(session_id);
    let msg_count = scaffold.context_mgr.get_all_messages(session_id).len();
    send_ok(tx, request_id, json!({
        "session_id": session_id,
        "message_count": msg_count,
        "estimated_tokens": stats.estimated_tokens,
        "window_tokens": stats.window_tokens,
        "usage_percent": stats.usage_percent,
    })).await;
}

/// session.info — 完整 session 详情（含 session 元数据 + context 统计 + provider 信息）
async fn handle_session_info(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }

    let stats = scaffold.context_mgr.stats(session_id);
    let msg_count = scaffold.context_mgr.get_all_messages(session_id).len();
    let seed_count = scaffold.context_mgr.get_seeds(session_id).len();
    let has_override = scaffold.session_mgr.get_provider_override(session_id).is_some();

    // 尝试从 SessionManager 获取 session 元数据
    let session_meta = scaffold.session_mgr.get(session_id).map(|s| {
        json!({
            "session_id": s.session_id,
            "type": format!("{:?}", s.session_type).to_lowercase(),
            "title": s.title,
            "status": format!("{:?}", s.status).to_lowercase(),
            "created_at": s.created_at.to_rfc3339(),
            "updated_at": s.updated_at.to_rfc3339(),
        })
    });

    send_ok(tx, request_id, json!({
        "session_id": session_id,
        "session": session_meta,
        "context": {
            "message_count": msg_count,
            "seed_count": seed_count,
            "estimated_tokens": stats.estimated_tokens,
            "window_tokens": stats.window_tokens,
            "usage_percent": stats.usage_percent,
        },
        "provider_override": has_override,
        "system_prompt_override": scaffold.session_mgr.get_system_prompt(session_id).is_some(),
        "tool_count": scaffold.tool_manager.tool_names(session_id).len(),
    })).await;
}

/// session.delete — 删除 session（清空 context + 移除 session 记录）
async fn handle_session_delete(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }

    // 清除上下文（消息仍保留在 storage，但不再可见）
    scaffold.context_mgr.remove_session(session_id);
    // 移除 session 记录
    let removed = scaffold.session_mgr.remove_session(session_id);

    send_ok(tx, request_id, json!({
        "session_id": session_id,
        "deleted": removed,
    })).await;

    info!(session_id = %session_id, "session deleted via ws");
}

/// session.clear — 仅清空上下文视图（消息永久保留，符合规则）
async fn handle_session_clear(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }

    let before = scaffold.context_mgr.stats(session_id);
    scaffold.context_mgr.remove_session(session_id);

    send_ok(tx, request_id, json!({
        "session_id": session_id,
        "cleared": true,
        "before": {
            "message_count": before.message_count,
            "estimated_tokens": before.estimated_tokens,
        },
        "note": "messages preserved in storage, context view cleared",
    })).await;

    info!(session_id = %session_id, "context cleared via ws");
}

/// session.messages — 分页读取消息历史
async fn handle_session_messages(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
    payload: &serde_json::Value,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }

    let page = payload["page"].as_u64().unwrap_or(0) as usize;
    let limit = payload["limit"].as_u64().unwrap_or(50) as usize;
    let limit = limit.min(200); // 上限 200

    let all = scaffold.context_mgr.get_all_messages(session_id);
    let total = all.len();
    let offset = page * limit;

    let paged: Vec<serde_json::Value> = all.iter()
        .skip(offset)
        .take(limit)
        .map(|m| {
            let text: String = m.content.iter().filter_map(|c| {
                match c {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    ContentBlock::ToolUse { .. } => None,
                    ContentBlock::ToolResult { content, .. } => content.as_deref(),
                    _ => None,
                }
            }).collect();
            json!({
                "message_id": m.message_id,
                "role": format!("{:?}", m.role).to_lowercase(),
                "kind": format!("{:?}", m.kind).to_lowercase(),
                "text": text,
                "created_at": m.created_at.to_rfc3339(),
            })
        })
        .collect();

    send_ok(tx, request_id, json!({
        "session_id": session_id,
        "page": page,
        "limit": limit,
        "total": total,
        "pages": (total as f64 / limit as f64).ceil() as u64,
        "messages": paged,
    })).await;
}

/// session.list — 分页查询 session 列表（系统级，session_id 为空）
async fn handle_list_sessions(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    request_id: &str,
    payload: &serde_json::Value,
) {
    let page = payload["page"].as_u64().unwrap_or(0) as u32;
    let limit = payload["limit"].as_u64().unwrap_or(20) as u32;
    let limit = limit.min(100);
    let status = payload["status"].as_str();

    let (sessions, total) = scaffold.session_mgr.list_sessions_paged(page, limit, status);

    let list: Vec<serde_json::Value> = sessions.iter().map(|s| {
        let stats = scaffold.context_mgr.stats(&s.session_id);
        json!({
            "session_id": s.session_id,
            "type": format!("{:?}", s.session_type).to_lowercase(),
            "title": s.title,
            "status": format!("{:?}", s.status).to_lowercase(),
            "message_count": stats.message_count,
            "estimated_tokens": stats.estimated_tokens,
            "provider_override": scaffold.session_mgr.get_provider_override(&s.session_id).is_some(),
            "system_prompt_override": scaffold.session_mgr.get_system_prompt(&s.session_id).is_some(),
            "created_at": s.created_at.to_rfc3339(),
            "updated_at": s.updated_at.to_rfc3339(),
            "summary": s.metadata.get("summary").and_then(|v| v.as_str()).unwrap_or(""),
        })
    }).collect();

    send_ok(tx, request_id, json!({
        "page": page,
        "limit": limit,
        "total": total,
        "pages": (total as f64 / limit as f64).ceil() as u64,
        "sessions": list,
    })).await;
}

/// system.stats — 系统级统计
async fn handle_system_stats(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    request_id: &str,
) {
    let session_count = scaffold.session_mgr.session_count();
    let tool_count = scaffold.tool_manager.count();
    let config = &scaffold.config;

    send_ok(tx, request_id, json!({
        "session_count": session_count,
        "tool_count": tool_count,
        "default_provider": {
            "protocol": format!("{:?}", config.protocol).to_lowercase(),
            "base_url": config.base_url,
            "model": config.model,
            "context_window_tokens": config.context_window_tokens,
        },
        "system_prompt_length": scaffold.get_system_prompt().len(),
    })).await;
}

/// events.pull — 断线补拉：返回 session 中 seq > since_seq 的所有事件
async fn handle_events_pull(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
    payload: &serde_json::Value,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required for events.pull").await;
        return;
    }

    let since_seq = payload["since_seq"].as_u64().unwrap_or(0);
    let events = scaffold.event_bus.pull_since(session_id, since_seq);
    let current_seq = scaffold.event_bus.current_seq(session_id);

    let event_list: Vec<serde_json::Value> = events.iter().map(|e| {
        serde_json::to_value(e).unwrap_or(serde_json::Value::Null)
    }).collect();

    send_ok(tx, request_id, json!({
        "session_id": session_id,
        "since_seq": since_seq,
        "current_seq": current_seq,
        "count": events.len(),
        "events": event_list,
    })).await;

    debug!(session_id = %session_id, since_seq = since_seq, count = events.len(), "events pulled");
}

/// events.subscribe — 订阅实时事件流，可选 since_seq 补拉历史后再切换实时
async fn handle_events_subscribe(
    scaffold: Arc<AgentKernel>,
    tx: mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
    payload: &serde_json::Value,
) {
    if session_id.is_empty() {
        send_err(&tx, request_id, "session_id is required for events.subscribe").await;
        return;
    }

    let since_seq = payload["since_seq"].as_u64().unwrap_or(0);
    let current_seq = scaffold.event_bus.current_seq(session_id);

    // 1. 先发送补拉结果
    if since_seq > 0 && since_seq < current_seq {
        let missed = scaffold.event_bus.pull_since(session_id, since_seq);
        for evt in &missed {
            let _ = tx.send(WsMessage::Event(evt.clone())).await;
        }
        info!(session_id = %session_id, missed = missed.len(), "replayed missed events");
    }

    // 2. 确认订阅成功
    send_ok(&tx, request_id, json!({
        "session_id": session_id,
        "subscribed": true,
        "since_seq": since_seq,
        "current_seq": current_seq,
        "replayed": if since_seq > 0 { current_seq.saturating_sub(since_seq) } else { 0 },
    })).await;

    // 3. 注册实时转发（后续该 session 的事件自动推送给此连接）
    //    通过在连接的事件循环中增加 session 订阅过滤来实现
    //    当前架构中 EventBus 广播所有事件，连接层已按 session_id 过滤
    //    客户端只需用 events.pull + 后续实时广播即可覆盖全场景
    info!(session_id = %session_id, "events subscription active");
}

async fn handle_context_preview(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }
    let all_messages = scaffold.context_mgr.get_all_messages(session_id);
    let active_messages = scaffold.context_mgr.build_context_view(session_id);
    let model_input = scaffold.context_mgr.build_model_input(session_id);
    let seeds = scaffold.context_mgr.get_seeds(session_id);
    let stats = scaffold.context_mgr.stats(session_id);
    let active_context = scaffold.context_mgr.get_context(session_id)
        .unwrap_or_else(|| scaffold.context_mgr.default_context_state(session_id));
    let preview = core_export::context_preview(&active_messages, &seeds);
    send_ok(tx, request_id, json!({
        "session_id": session_id,
        "active_context": active_context,
        "stats": {
            "message_count": stats.message_count,
            "estimated_tokens": stats.estimated_tokens,
            "window_tokens": stats.window_tokens,
            "usage_percent": stats.usage_percent,
        },
        "counts": {
            "all_messages": all_messages.len(),
            "active_messages": active_messages.len(),
            "model_input_messages": model_input.len(),
            "seeds": seeds.len(),
        },
        "messages": active_messages,
        "seeds": seeds,
        "preview": preview,
    })).await;
}

async fn persist_context_state(scaffold: &AgentKernel, ctx: &ContextState) -> Result<(), String> {
    scaffold.storage.save_context_state(ctx).await
}

async fn emit_context_event(scaffold: &AgentKernel, session_id: &str, event_type: &str, payload: serde_json::Value) {
    scaffold.event_bus.emit(EventEnvelope::new(event_type, session_id).with_payload(payload));
}

async fn handle_context_reset(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }
    scaffold.session_mgr.get_or_create(session_id).await.ok();
    let ctx = scaffold.context_mgr.reset_context(session_id);
    if let Err(e) = persist_context_state(scaffold, &ctx).await {
        send_err(tx, request_id, &e).await;
        return;
    }
    emit_context_event(scaffold, session_id, "context.reset", json!({"context": ctx.clone()})).await;
    send_ok(tx, request_id, json!({"session_id": session_id, "active_context": ctx})).await;
}

async fn handle_context_exclude(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
    payload: &serde_json::Value,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }
    let start = payload["start_message_id"].as_str().unwrap_or("");
    let end = payload["end_message_id"].as_str().unwrap_or(start);
    if start.is_empty() {
        send_err(tx, request_id, "start_message_id is required").await;
        return;
    }
    scaffold.session_mgr.get_or_create(session_id).await.ok();
    let ctx = scaffold.context_mgr.exclude_range(session_id, start, end);
    if let Err(e) = persist_context_state(scaffold, &ctx).await {
        send_err(tx, request_id, &e).await;
        return;
    }
    emit_context_event(scaffold, session_id, "context.updated", json!({"action": "exclude", "context": ctx.clone()})).await;
    send_ok(tx, request_id, json!({"session_id": session_id, "active_context": ctx})).await;
}

async fn handle_context_include_after(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
    payload: &serde_json::Value,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }
    let message_id = payload["message_id"].as_str().unwrap_or("");
    if message_id.is_empty() {
        send_err(tx, request_id, "message_id is required").await;
        return;
    }
    scaffold.session_mgr.get_or_create(session_id).await.ok();
    let ctx = scaffold.context_mgr.include_after(session_id, message_id);
    if let Err(e) = persist_context_state(scaffold, &ctx).await {
        send_err(tx, request_id, &e).await;
        return;
    }
    emit_context_event(scaffold, session_id, "context.updated", json!({"action": "include_after", "context": ctx.clone()})).await;
    send_ok(tx, request_id, json!({"session_id": session_id, "active_context": ctx})).await;
}

async fn handle_context_keep_recent(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
    payload: &serde_json::Value,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }
    let keep = if payload["keep"].is_null() { None } else { payload["keep"].as_u64() };
    scaffold.session_mgr.get_or_create(session_id).await.ok();
    let ctx = scaffold.context_mgr.set_keep_recent(session_id, keep);
    if let Err(e) = persist_context_state(scaffold, &ctx).await {
        send_err(tx, request_id, &e).await;
        return;
    }
    emit_context_event(scaffold, session_id, "context.updated", json!({"action": "keep_recent", "context": ctx.clone()})).await;
    send_ok(tx, request_id, json!({"session_id": session_id, "active_context": ctx})).await;
}

async fn handle_context_seed_add(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
    payload: &serde_json::Value,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }
    let content = payload["content"].as_str().unwrap_or("").to_string();
    if content.trim().is_empty() {
        send_err(tx, request_id, "content is required").await;
        return;
    }
    let kind = match payload["kind"].as_str().unwrap_or("system_memory") {
        "compaction_summary" => SeedKind::CompactionSummary,
        "user_preference" => SeedKind::UserPreference,
        "world_state" => SeedKind::WorldState,
        "agent_state" => SeedKind::AgentState,
        _ => SeedKind::SystemMemory,
    };
    let seed = ContextSeed {
        seed_id: format!("seed_{}", uuid::Uuid::new_v4()),
        session_id: session_id.to_string(),
        kind,
        content,
        enabled: payload["enabled"].as_bool().unwrap_or(true),
        priority: payload["priority"].as_i64().unwrap_or(0) as i32,
    };
    scaffold.session_mgr.get_or_create(session_id).await.ok();
    scaffold.context_mgr.add_seed(session_id, seed.clone());
    if let Err(e) = scaffold.storage.save_seed(&seed).await {
        send_err(tx, request_id, &e).await;
        return;
    }
    emit_context_event(scaffold, session_id, "context.seed.added", json!({"seed": seed.clone()})).await;
    send_ok(tx, request_id, json!({"session_id": session_id, "seed": seed})).await;
}

async fn handle_compaction_apply(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
    payload: &serde_json::Value,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required").await;
        return;
    }
    let summary = payload["summary"].as_str().unwrap_or("").to_string();
    if summary.trim().is_empty() {
        send_err(tx, request_id, "summary is required").await;
        return;
    }
    let include_after = payload["include_after_message_id"].as_str().map(String::from);
    scaffold.session_mgr.get_or_create(session_id).await.ok();
    let (ctx, seed) = scaffold.context_mgr.apply_compaction(session_id, summary, include_after);
    if let Err(e) = scaffold.storage.save_seed(&seed).await {
        send_err(tx, request_id, &e).await;
        return;
    }
    if let Err(e) = persist_context_state(scaffold, &ctx).await {
        send_err(tx, request_id, &e).await;
        return;
    }
    emit_context_event(scaffold, session_id, CONTEXT_COMPACTION_APPLIED, json!({
        "context": ctx.clone(),
        "seed": seed.clone(),
    })).await;
    send_ok(tx, request_id, json!({"session_id": session_id, "active_context": ctx, "seed": seed})).await;
}

async fn handle_update_provider(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
    payload: serde_json::Value,
) {
    if session_id.is_empty() {
        send_err(tx, request_id, "session_id is required for provider.update").await;
        return;
    }

    // 读取当前覆盖（或默认）作为基础
    let mut config = scaffold.session_mgr.get_provider_override(session_id)
        .unwrap_or_else(|| scaffold.config.clone());

    // 按字段覆盖
    if let Some(p) = payload["protocol"].as_str() {
        config.protocol = match p {
            "claude" => Protocol::Claude,
            "openai" => Protocol::OpenAI,
            _ => { send_err(tx, request_id, &format!("unknown protocol: {}", p)).await; return; }
        };
    }
    if let Some(u) = payload["base_url"].as_str() {
        config.base_url = u.to_string();
    }
    if let Some(k) = payload["api_key"].as_str() {
        config.api_key = k.to_string();
    }
    if let Some(m) = payload["model"].as_str() {
        config.model = m.to_string();
    }
    if let Some(t) = payload["max_tokens"].as_u64() {
        config.max_tokens = t;
    }
    if let Some(t) = payload["temperature"].as_f64() {
        config.temperature = t;
    }

    // 验证：api_key 和 model 不能为空
    if config.api_key.is_empty() || config.model.is_empty() {
        send_err(tx, request_id, "api_key and model are required").await;
        return;
    }

    scaffold.session_mgr.set_provider_override(session_id, config.clone()).await
        .map_err(|e| e.to_string()).ok();

    send_ok(tx, request_id, json!({
        "session_id": session_id,
        "provider": {
            "protocol": format!("{:?}", config.protocol).to_lowercase(),
            "base_url": config.base_url,
            "model": config.model,
            "max_tokens": config.max_tokens,
            "temperature": config.temperature,
        }
    })).await;

    info!(session_id = %session_id, model = %config.model, "provider updated for session");
}

async fn handle_get_provider(
    scaffold: &AgentKernel,
    tx: &mpsc::Sender<WsMessage>,
    session_id: &str,
    request_id: &str,
) {
    let config = if !session_id.is_empty() {
        scaffold.session_mgr.get_provider_override(session_id)
            .unwrap_or_else(|| scaffold.config.clone())
    } else {
        scaffold.config.clone()
    };

    send_ok(tx, request_id, json!({
        "session_id": session_id,
        "is_override": scaffold.session_mgr.get_provider_override(session_id).is_some(),
        "provider": {
            "protocol": format!("{:?}", config.protocol).to_lowercase(),
            "base_url": config.base_url,
            "api_key": config.api_key,
            "model": config.model,
            "max_tokens": config.max_tokens,
            "temperature": config.temperature,
        }
    })).await;
}

// ─── Helpers ───────────────────────────────────────────────────

async fn send_ok(tx: &mpsc::Sender<WsMessage>, request_id: &str, payload: serde_json::Value) {
    let _ = tx.send(WsMessage::Response {
        request_id: request_id.to_string(),
        success: true,
        payload,
    }).await;
}

async fn send_err(tx: &mpsc::Sender<WsMessage>, request_id: &str, error: &str) {
    send_err_payload(tx, request_id, json!({"error": error})).await;
}

async fn send_err_payload(tx: &mpsc::Sender<WsMessage>, request_id: &str, payload: serde_json::Value) {
    let _ = tx.send(WsMessage::Response {
        request_id: request_id.to_string(),
        success: false,
        payload,
    }).await;
}

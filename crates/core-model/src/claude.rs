//! Claude 协议适配器

use async_trait::async_trait;
use core_protocol::*;
use crate::{normalize_for_claude, prepare_model_input, CallTrace, ProviderAdapter};
use std::time::Instant;

pub struct ClaudeAdapter;

#[async_trait]
impl ProviderAdapter for ClaudeAdapter {
    fn protocol(&self) -> Protocol { Protocol::Claude }
    fn name(&self) -> &str { "claude" }

    async fn send_message(
        &self,
        config: &ProviderConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Response, CallTrace), String> {
        let start = Instant::now();
        let url = build_url(&config.base_url);
        let prepared = prepare_model_input(messages);
        let mut trace = CallTrace {
            api_url: url.clone(),
            protocol: "claude".to_string(),
            model: config.model.clone(),
            stream: false,
            tool_chain_report: Some(prepared.tool_chain_report.clone()),
            ..Default::default()
        };

        // Claude 不支持音频输入
        for msg in &prepared.sanitized_messages {
            for block in &msg.content {
                if let ContentBlock::Audio { .. } = block {
                    return Err("Claude API 不支持音频输入（Audio）。请使用支持音频的供应商（如本地 llama-server）或先将音频转为文字。".to_string());
                }
            }
        }

        let normalized = normalize_for_claude(&prepared);

        let mut body = serde_json::json!({
            "model": config.model,
            "max_tokens": config.max_tokens,
            "stream": false,
            "system": [{"type": "text", "text": system}],
            "messages": serialize_messages(&normalized),
        });
        if !tools.is_empty() && config.tools_mode == ToolsMode::Standard {
            body["tools"] = serde_json::json!(serialize_tools(tools));
        }
        if config.temperature > 0.0 {
            body["temperature"] = serde_json::json!(config.temperature);
        }

        let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;
        trace.request_body = body_str.clone();
        trace.request_headers = build_headers(config);

        let client = reqwest::Client::new();
        let resp = client.post(&url)
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(body_str)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        trace.response_code = resp.status().as_u16();
        let resp_body = resp.text().await.map_err(|e| e.to_string())?;
        trace.response_body = resp_body.clone();
        trace.duration_ms = start.elapsed().as_millis() as u64;

        if trace.response_code != 200 {
            trace.error = Some(resp_body.clone());
            return Err(format!("claude API error ({}): {}", trace.response_code, resp_body));
        }

        let parsed: serde_json::Value = serde_json::from_str(&resp_body).map_err(|e| e.to_string())?;
        let response = parse_response(&parsed)?;
        trace.finish_reason = format!("{:?}", response.stop_reason);
        trace.input_tokens = response.usage.input_tokens;
        trace.output_tokens = response.usage.output_tokens;
        Ok((response, trace))
    }

    async fn stream_message(
        &self,
        config: &ProviderConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        handler: Box<dyn Fn(StreamEvent) + Send + Sync>,
    ) -> Result<(Response, CallTrace), String> {
        let start = Instant::now();
        let url = build_url(&config.base_url);
        let prepared = prepare_model_input(messages);
        let mut trace = CallTrace {
            api_url: url.clone(),
            protocol: "claude".to_string(),
            model: config.model.clone(),
            stream: true,
            tool_chain_report: Some(prepared.tool_chain_report.clone()),
            ..Default::default()
        };

        // Claude 不支持音频输入
        for msg in &prepared.sanitized_messages {
            for block in &msg.content {
                if let ContentBlock::Audio { .. } = block {
                    return Err("Claude API 不支持音频输入（Audio）。请使用支持音频的供应商（如本地 llama-server）或先将音频转为文字。".to_string());
                }
            }
        }

        let normalized = normalize_for_claude(&prepared);
        let mut body = serde_json::json!({
            "model": config.model,
            "max_tokens": config.max_tokens,
            "stream": true,
            "system": [{"type": "text", "text": system}],
            "messages": serialize_messages(&normalized),
        });
        if !tools.is_empty() && config.tools_mode == ToolsMode::Standard {
            body["tools"] = serde_json::json!(serialize_tools(tools));
        }
        if config.temperature > 0.0 {
            body["temperature"] = serde_json::json!(config.temperature);
        }

        let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;
        trace.request_body = body_str.clone();

        let client = reqwest::Client::new();
        let resp = client.post(&url)
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .body(body_str)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        trace.response_code = resp.status().as_u16();
        if trace.response_code != 200 {
            let err_body = resp.text().await.map_err(|e| e.to_string())?;
            trace.error = Some(err_body.clone());
            trace.duration_ms = start.elapsed().as_millis() as u64;
            return Err(format!("claude API error ({}): {}", trace.response_code, err_body));
        }

        // SSE 流式解析
        use futures::StreamExt;
        let mut text_parts = Vec::new();
        let mut thinking_parts = Vec::new();
        let mut tool_inputs: std::collections::HashMap<usize, Vec<String>> = std::collections::HashMap::new();
        let mut tool_names: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
        let mut tool_ids: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
        let mut stop_reason = String::new();
        let mut response_id = String::new();

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer = buffer[line_end + 1..].to_string();

                if !line.starts_with("data: ") {
                    continue;
                }
                let data = &line[6..];
                if data == "[DONE]" {
                    break;
                }

                let event: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let etype = event["type"].as_str().unwrap_or("");
                match etype {
                    "message_start" => {
                        response_id = event["message"]["id"].as_str().unwrap_or("").to_string();
                    }
                    "content_block_start" => {
                        let idx = event["index"].as_u64().unwrap_or(0) as usize;
                        if event["content_block"]["type"].as_str() == Some("tool_use") {
                            tool_names.insert(idx, event["content_block"]["name"].as_str().unwrap_or("").to_string());
                            tool_ids.insert(idx, event["content_block"]["id"].as_str().unwrap_or("").to_string());
                            tool_inputs.insert(idx, Vec::new());
                        } else if event["content_block"]["type"].as_str() == Some("thinking") {
                            thinking_parts.push(event["content_block"]["thinking"].as_str().unwrap_or("").to_string());
                        }
                    }
                    "content_block_delta" => {
                        let idx = event["index"].as_u64().unwrap_or(0) as usize;
                        if event["delta"]["type"].as_str() == Some("text_delta") {
                            let text = event["delta"]["text"].as_str().unwrap_or("");
                            text_parts.push(text.to_string());
                            handler(StreamEvent {
                                event: StreamEventType::Text,
                                delta: text.to_string(),
                                full_text: text_parts.join(""),
                                session_id: String::new(),
                                run_id: String::new(),
                            });
                        } else if event["delta"]["type"].as_str() == Some("thinking_delta") {
                            let thinking = event["delta"]["thinking"].as_str().unwrap_or("");
                            thinking_parts.push(thinking.to_string());
                            handler(StreamEvent {
                                event: StreamEventType::Thinking,
                                delta: thinking.to_string(),
                                full_text: thinking_parts.join(""),
                                session_id: String::new(),
                                run_id: String::new(),
                            });
                        } else if event["delta"]["type"].as_str() == Some("input_json_delta") {
                            if let Some(buf) = tool_inputs.get_mut(&idx) {
                                buf.push(event["delta"]["partial_json"].as_str().unwrap_or("").to_string());
                            }
                        }
                    }
                    "message_delta" => {
                        stop_reason = event["delta"]["stop_reason"].as_str().unwrap_or("").to_string();
                    }
                    _ => {}
                }
            }
        }

        // 组装响应
        let mut content = Vec::new();
        let full_text = text_parts.join("");
        if !full_text.is_empty() {
            content.push(ContentBlock::text(&full_text));
        }
        let full_thinking = thinking_parts.join("");
        if !full_thinking.is_empty() {
            content.push(ContentBlock::Thinking {
                thinking: full_thinking,
                signature: None,
            });
        }

        for (idx, buf) in &tool_inputs {
            let raw: String = buf.join("");
            let input: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
            let name = tool_names.get(idx).cloned().unwrap_or_default();
            let id = tool_ids.get(idx).cloned().unwrap_or_else(|| format!("toolu_{}", idx));
            content.push(ContentBlock::tool_use(&id, &name, input));
        }

        let stop = match stop_reason.as_str() {
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
            "end_turn" => StopReason::EndTurn,
            _ => StopReason::Unknown,
        };

        trace.duration_ms = start.elapsed().as_millis() as u64;
        trace.finish_reason = stop_reason.clone();

        Ok((Response {
            id: response_id,
            model: config.model.clone(),
            role: Role::Assistant,
            content,
            stop_reason: stop,
            usage: Usage::default(),
        }, trace))
    }
}

fn build_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1/messages") {
        base.to_string()
    } else {
        format!("{}/v1/messages", base)
    }
}

fn build_headers(config: &ProviderConfig) -> std::collections::HashMap<String, String> {
    let mut h = std::collections::HashMap::new();
    h.insert("content-type".into(), "application/json".into());
    h.insert("anthropic-version".into(), "2023-06-01".into());
    h.insert("accept".into(), "text/event-stream".into());
    let key_display = if config.api_key.len() > 8 {
        format!("{}...", &config.api_key[..8])
    } else {
        "***".to_string()
    };
    h.insert("x-api-key".into(), key_display);
    h
}

fn serialize_messages(messages: &[Message]) -> serde_json::Value {
    let msgs: Vec<serde_json::Value> = messages.iter().map(|m| {
        let content: Vec<serde_json::Value> = m.content.iter().map(|c| {
            serde_json::to_value(c).unwrap_or(serde_json::Value::Null)
        }).collect();
        serde_json::json!({
            "role": format!("{:?}", m.role).to_lowercase(),
            "content": content,
        })
    }).collect();
    serde_json::Value::Array(msgs)
}

fn serialize_tools(tools: &[Tool]) -> Vec<serde_json::Value> {
    tools.iter().map(|t| {
        serde_json::json!({
            "name": t.name,
            "description": t.description,
            "input_schema": t.schema_for_protocol(&Protocol::Claude),
        })
    }).collect()
}

fn parse_response(val: &serde_json::Value) -> Result<Response, String> {
    let id = val["id"].as_str().unwrap_or("").to_string();
    let model = val["model"].as_str().unwrap_or("").to_string();
    let stop = match val["stop_reason"].as_str().unwrap_or("") {
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "end_turn" => StopReason::EndTurn,
        _ => StopReason::Unknown,
    };
    let mut content = Vec::new();
    if let Some(arr) = val["content"].as_array() {
        for item in arr {
            let cb: ContentBlock = serde_json::from_value(item.clone())
                .map_err(|e| format!("parse content block: {}", e))?;
            content.push(cb);
        }
    }
    let usage = Usage {
        input_tokens: val["usage"]["input_tokens"].as_u64().unwrap_or(0),
        output_tokens: val["usage"]["output_tokens"].as_u64().unwrap_or(0),
        ..Default::default()
    };
    Ok(Response { id, model, role: Role::Assistant, content, stop_reason: stop, usage })
}

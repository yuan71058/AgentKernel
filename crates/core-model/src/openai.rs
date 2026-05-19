//! OpenAI 兼容协议适配器

use async_trait::async_trait;
use core_protocol::*;
use crate::{convert_for_openai, CallTrace, ProviderAdapter};
use std::time::Instant;

pub struct OpenAIAdapter;

#[async_trait]
impl ProviderAdapter for OpenAIAdapter {
    fn protocol(&self) -> Protocol { Protocol::OpenAI }
    fn name(&self) -> &str { "openai" }

    async fn send_message(
        &self,
        config: &ProviderConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Response, CallTrace), String> {
        let start = Instant::now();
        let url = build_url(&config.base_url);
        let mut trace = CallTrace {
            api_url: url.clone(),
            protocol: "openai".to_string(),
            model: config.model.clone(),
            stream: false,
            ..Default::default()
        };

        let mut msgs = Vec::new();
        if !system.is_empty() {
            msgs.push(serde_json::json!({"role": "system", "content": system}));
        }
        msgs.extend(convert_for_openai(messages));

        let mut body = serde_json::json!({
            "model": config.model,
            "max_tokens": config.max_tokens,
            "stream": false,
            "messages": msgs,
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
            .header("Authorization", format!("Bearer {}", config.api_key))
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
            return Err(format!("openai API error ({}): {}", trace.response_code, resp_body));
        }

        let parsed: serde_json::Value = serde_json::from_str(&resp_body).map_err(|e| e.to_string())?;
        let response = parse_response(&parsed)?;
        trace.finish_reason = response.stop_reason.to_string();
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
        let mut trace = CallTrace {
            api_url: url.clone(),
            protocol: "openai".to_string(),
            model: config.model.clone(),
            stream: true,
            ..Default::default()
        };

        let mut msgs = Vec::new();
        if !system.is_empty() {
            msgs.push(serde_json::json!({"role": "system", "content": system}));
        }
        msgs.extend(convert_for_openai(messages));

        let mut body = serde_json::json!({
            "model": config.model,
            "max_tokens": config.max_tokens,
            "stream": true,
            "messages": msgs,
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
            .header("Authorization", format!("Bearer {}", config.api_key))
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
            return Err(format!("openai API error ({}): {}", trace.response_code, err_body));
        }

        use futures::StreamExt;
        let mut text_parts = Vec::new();
        let mut thinking_parts = Vec::new();
        let mut tool_calls: std::collections::HashMap<usize, (String, String, String)> = std::collections::HashMap::new();
        let mut stop_reason = String::new();
        let mut buffer = String::new();

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer = buffer[line_end + 1..].to_string();

                if !line.starts_with("data: ") { continue; }
                let data = &line[6..];
                if data == "[DONE]" { break; }

                let event: serde_json::Value = match serde_json::from_str(data) {
                    Ok(v) => v, Err(_) => continue,
                };

                if let Some(choices) = event["choices"].as_array() {
                    if let Some(choice) = choices.first() {
                        if let Some(sr) = choice["finish_reason"].as_str() {
                            stop_reason = sr.to_string();
                        }
                        if let Some(delta) = choice.get("delta") {
                            if let Some(reasoning) = delta["reasoning_content"].as_str().or_else(|| delta["reasoning"].as_str()) {
                                thinking_parts.push(reasoning.to_string());
                                handler(StreamEvent {
                                    event: StreamEventType::Thinking,
                                    delta: reasoning.to_string(),
                                    full_text: thinking_parts.join(""),
                                    session_id: String::new(),
                                    run_id: String::new(),
                                });
                            }
                            if let Some(content) = delta["content"].as_str() {
                                text_parts.push(content.to_string());
                                handler(StreamEvent {
                                    event: StreamEventType::Text,
                                    delta: content.to_string(),
                                    full_text: text_parts.join(""),
                                    session_id: String::new(),
                                    run_id: String::new(),
                                });
                            }
                            if let Some(tcs) = delta["tool_calls"].as_array() {
                                for tc in tcs {
                                    let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                                    let id = tc["id"].as_str().unwrap_or("").to_string();
                                    let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                                    let args = tc["function"]["arguments"].as_str().unwrap_or("").to_string();
                                    let entry = tool_calls.entry(idx).or_insert_with(Default::default);
                                    if !id.is_empty() { entry.0 = id; }
                                    if !name.is_empty() { entry.1 = name; }
                                    entry.2.push_str(&args);
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut content = Vec::new();
        let full_text = text_parts.join("");
        if !full_text.is_empty() {
            let full_reasoning = thinking_parts.join("");
            content.push(ContentBlock::Text {
                text: full_text,
                reasoning_content: if full_reasoning.is_empty() { None } else { Some(full_reasoning) },
            });
        }
        for (_, (id, name, args)) in &tool_calls {
            let input: serde_json::Value = serde_json::from_str(args).unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
            content.push(ContentBlock::tool_use(id, name, input));
        }

        let stop = match stop_reason.as_str() {
            "tool_calls" | "tool_use" => StopReason::ToolUse,
            "length" => StopReason::MaxTokens,
            "stop" | "content_filter" => StopReason::EndTurn,
            _ => StopReason::Unknown,
        };

        trace.duration_ms = start.elapsed().as_millis() as u64;
        trace.finish_reason = stop_reason;

        Ok((Response {
            id: String::new(),
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
    if base.ends_with("/v1/chat/completions") {
        base.to_string()
    } else {
        format!("{}/v1/chat/completions", base)
    }
}

fn serialize_tools(tools: &[Tool]) -> Vec<serde_json::Value> {
    tools.iter().map(|t| {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": t.name,
                "description": t.description,
                "parameters": t.input_schema,
            }
        })
    }).collect()
}

fn parse_response(val: &serde_json::Value) -> Result<Response, String> {
    let id = val["id"].as_str().unwrap_or("").to_string();
    let model = val["model"].as_str().unwrap_or("").to_string();
    let mut content = Vec::new();

    if let Some(choices) = val["choices"].as_array() {
        if let Some(choice) = choices.first() {
            if let Some(text) = choice["message"]["content"].as_str() {
                if !text.is_empty() {
                    content.push(ContentBlock::text(text));
                }
            }
            if let Some(tcs) = choice["message"]["tool_calls"].as_array() {
                for tc in tcs {
                    let id = tc["id"].as_str().unwrap_or("").to_string();
                    let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                    let args: serde_json::Value = serde_json::from_str(
                        tc["function"]["arguments"].as_str().unwrap_or("{}")
                    ).unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                    content.push(ContentBlock::tool_use(&id, &name, args));
                }
            }
        }
    }

    let usage = Usage {
        input_tokens: val["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
        output_tokens: val["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        ..Default::default()
    };

    Ok(Response {
        id, model,
        role: Role::Assistant,
        content,
        stop_reason: StopReason::EndTurn,
        usage,
    })
}

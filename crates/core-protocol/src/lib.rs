//! # Core Protocol
//!
//! AI Runtime Core 的统一协议层。
//! 定义所有 crate 共享的类型：Message、ContentBlock、Tool、Response、Event Envelope、Session、Run。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ═══════════════════════════════════════════════════════════════
//  Message & ContentBlock
// ═══════════════════════════════════════════════════════════════

/// 消息角色
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
}

/// 消息内容块类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
    },
    Image {
        source: ImageSource,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    Audio {
        source: AudioSource,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSource {
    pub data: String,
    pub format: String,
}

impl ContentBlock {
    pub fn text(t: &str) -> Self {
        ContentBlock::Text { text: t.to_string(), reasoning_content: None }
    }
    pub fn tool_use(id: &str, name: &str, input: serde_json::Value) -> Self {
        ContentBlock::ToolUse { id: id.to_string(), name: name.to_string(), input }
    }
    pub fn tool_result(tool_use_id: &str, content: &str, is_error: bool) -> Self {
        ContentBlock::ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: Some(content.to_string()),
            is_error: Some(is_error),
        }
    }
    pub fn image(media_type: &str, data: &str) -> Self {
        ContentBlock::Image {
            source: ImageSource {
                source_type: "base64".to_string(),
                media_type: media_type.to_string(),
                data: data.to_string(),
            },
        }
    }
    pub fn audio(format: &str, data: &str) -> Self {
        ContentBlock::Audio {
            source: AudioSource {
                data: data.to_string(),
                format: format.to_string(),
            },
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Message（永久保留，不删除）
// ═══════════════════════════════════════════════════════════════

/// 消息 Kind
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Normal,
    ToolResult,
    CompactionSummary,
    ContextSeed,
    SystemNote,
}

/// 内部统一消息（永久保存）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub message_id: String,
    pub session_id: String,
    pub run_id: String,
    pub role: Role,
    pub kind: MessageKind,
    pub content: Vec<ContentBlock>,
    pub token_estimate: u64,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Message {
    pub fn new(session_id: &str, role: Role, content: Vec<ContentBlock>) -> Self {
        Self {
            message_id: format!("msg_{}", Uuid::new_v4()),
            session_id: session_id.to_string(),
            run_id: String::new(),
            role,
            kind: MessageKind::Normal,
            content,
            token_estimate: 0,
            created_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Tool Definition
// ═══════════════════════════════════════════════════════════════

/// 工具定义（协议层）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// 原始注册 schema，用于展示、持久化和跨 provider 重新编译。
    pub input_schema: serde_json::Value,
    /// provider-specific 编译结果，key 使用协议小写名（claude/openai）。
    #[serde(default)]
    pub compiled_schemas: HashMap<String, serde_json::Value>,
}

impl Tool {
    pub fn with_compiled_schemas(mut self) -> Result<Self, String> {
        self.compiled_schemas = compile_tool_schemas_for_providers(&self.input_schema)?;
        Ok(self)
    }

    pub fn schema_for_protocol(&self, protocol: &Protocol) -> serde_json::Value {
        let key = protocol.schema_key();
        self.compiled_schemas
            .get(key)
            .cloned()
            .unwrap_or_else(|| self.input_schema.clone())
    }
}

impl Protocol {
    pub fn schema_key(&self) -> &'static str {
        match self {
            Protocol::Claude => "claude",
            Protocol::OpenAI => "openai",
        }
    }
}

pub fn compile_tool_schemas_for_providers(schema: &serde_json::Value) -> Result<HashMap<String, serde_json::Value>, String> {
    let mut compiled = HashMap::new();
    compiled.insert("openai".to_string(), compile_openai_tool_schema(schema)?);
    compiled.insert("claude".to_string(), compile_claude_tool_schema(schema)?);
    Ok(compiled)
}

fn compile_openai_tool_schema(schema: &serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(obj) = schema.as_object() else {
        return Err("tool schema must be a JSON object".to_string());
    };
    if obj.get("type").and_then(|v| v.as_str()) == Some("object") || obj.contains_key("properties") {
        return Ok(normalize_object_schema(schema));
    }
    Err("tool schema must be an object schema or contain properties".to_string())
}

fn compile_claude_tool_schema(schema: &serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(obj) = schema.as_object() else {
        return Err("Claude tool input_schema must be a JSON object".to_string());
    };

    if obj.get("type").and_then(|v| v.as_str()) == Some("object") || obj.contains_key("properties") {
        let mut normalized = normalize_object_schema(schema);
        if let Some(map) = normalized.as_object_mut() {
            for key in ["oneOf", "anyOf", "allOf", "enum", "not"] {
                map.remove(key);
            }
        }
        return Ok(normalized);
    }

    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(arr) = obj.get(key).and_then(|v| v.as_array()) {
            return merge_object_variants_for_claude(key, arr);
        }
    }

    Err("Claude tool input_schema top-level must be object and cannot be adapted from this schema".to_string())
}

fn normalize_object_schema(schema: &serde_json::Value) -> serde_json::Value {
    let mut map = schema.as_object().cloned().unwrap_or_default();
    map.insert("type".to_string(), serde_json::json!("object"));
    if !map.get("properties").map(|v| v.is_object()).unwrap_or(false) {
        map.insert("properties".to_string(), serde_json::json!({}));
    }
    if !map.get("required").map(|v| v.is_array()).unwrap_or(false) {
        map.insert("required".to_string(), serde_json::json!([]));
    }
    serde_json::Value::Object(map)
}

fn merge_object_variants_for_claude(key: &str, variants: &[serde_json::Value]) -> Result<serde_json::Value, String> {
    if variants.is_empty() {
        return Err(format!("Claude tool input_schema top-level {key} has no variants to adapt"));
    }

    let mut properties = serde_json::Map::new();
    let mut required = std::collections::BTreeSet::new();
    let mut description_parts = Vec::new();

    for variant in variants {
        let normalized = normalize_object_schema(variant);
        let Some(obj) = normalized.as_object() else { continue; };
        if obj.get("type").and_then(|v| v.as_str()) != Some("object") {
            return Err(format!("Claude tool input_schema top-level {key} contains a non-object variant"));
        }
        if let Some(desc) = obj.get("description").and_then(|v| v.as_str()) {
            description_parts.push(desc.to_string());
        }
        if let Some(props) = obj.get("properties").and_then(|v| v.as_object()) {
            for (name, prop) in props {
                properties.entry(name.clone()).or_insert_with(|| prop.clone());
            }
        }
        if let Some(reqs) = obj.get("required").and_then(|v| v.as_array()) {
            for req in reqs.iter().filter_map(|v| v.as_str()) {
                required.insert(req.to_string());
            }
        }
    }

    let mut out = serde_json::Map::new();
    out.insert("type".to_string(), serde_json::json!("object"));
    if !description_parts.is_empty() {
        out.insert("description".to_string(), serde_json::json!(format!(
            "Adapted from top-level {key}. Variants: {}",
            description_parts.join(" | ")
        )));
    }
    out.insert("properties".to_string(), serde_json::Value::Object(properties));
    out.insert("required".to_string(), serde_json::json!(required.into_iter().collect::<Vec<_>>()));
    out.insert("additionalProperties".to_string(), serde_json::json!(true));
    Ok(serde_json::Value::Object(out))
}

/// 工具注册元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRegistration {
    pub tool_name: String,
    pub description: String,
    pub client_id: String,
    pub permissions: Vec<String>,
    pub timeout_ms: u64,
    pub tags: Vec<String>,
}

// ═══════════════════════════════════════════════════════════════
//  Response & Usage
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

/// 供应商响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub model: String,
    pub role: Role,
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for StopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

// ═══════════════════════════════════════════════════════════════
//  StreamEvent
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    pub event: StreamEventType,
    pub delta: String,
    pub full_text: String,
    pub session_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamEventType {
    Text,
    Thinking,
    ToolUse,
    ToolResult,
    Error,
    Done,
}

// ═══════════════════════════════════════════════════════════════
//  Event Envelope（统一事件结构）
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub id: String,
    pub event_type: String,
    pub session_id: String,
    pub run_id: String,
    pub trace_id: String,
    pub timestamp: DateTime<Utc>,
    pub payload: serde_json::Value,
    /// 事件序列号（每个 session 递增，用于断线补拉）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_seq: Option<u64>,
}

impl EventEnvelope {
    pub fn new(event_type: &str, session_id: &str) -> Self {
        Self {
            id: format!("evt_{}", Uuid::new_v4()),
            event_type: event_type.to_string(),
            session_id: session_id.to_string(),
            run_id: String::new(),
            trace_id: String::new(),
            timestamp: Utc::now(),
            payload: serde_json::Value::Null,
            event_seq: None,
        }
    }

    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = payload;
        self
    }

    pub fn with_run_id(mut self, run_id: &str) -> Self {
        self.run_id = run_id.to_string();
        self
    }

    pub fn with_trace_id(mut self, trace_id: &str) -> Self {
        self.trace_id = trace_id.to_string();
        self
    }
}

// ═══════════════════════════════════════════════════════════════
//  Session & Run
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub session_type: SessionType,
    pub title: String,
    pub active_context_id: String,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    Chat,
    Compact,
    System,
    ToolWorker,
    Evaluation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Paused,
    Closed,
    Archived,
}

/// Run：一次完整推理流程
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub run_id: String,
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

// ═══════════════════════════════════════════════════════════════
//  Context State
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextState {
    pub context_id: String,
    pub session_id: String,
    pub mode: ContextMode,
    pub rules: ContextRules,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    Full,
    Sliding,
    Compacted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrimMode {
    None,
    KeepRecentMessages,
    IncludeAfter,
    Checkpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimPolicy {
    pub mode: TrimMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_messages: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_max_context_messages: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retain_recent_turns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_after_message_id: Option<String>,
}

impl Default for TrimPolicy {
    fn default() -> Self {
        Self {
            mode: TrimMode::None,
            keep_messages: None,
            message_id: None,
            trigger_max_context_messages: None,
            retain_recent_turns: None,
            applied_after_message_id: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextRules {
    #[serde(default)]
    pub exclude_ranges: Vec<(String, String)>,
    #[serde(default)]
    pub trim: TrimPolicy,
    #[serde(default)]
    pub base_seed_ids: Vec<String>,
}

/// Context Seed：注入到上下文的记忆块
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSeed {
    pub seed_id: String,
    pub session_id: String,
    pub kind: SeedKind,
    pub content: String,
    pub enabled: bool,
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SeedKind {
    CompactionSummary,
    UserPreference,
    WorldState,
    AgentState,
    SystemMemory,
}

impl std::fmt::Display for SeedKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

// ═══════════════════════════════════════════════════════════════
//  Provider Config
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub protocol: Protocol,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub tools_mode: ToolsMode,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,
    #[serde(default)]
    pub temperature: f64,
    #[serde(default = "default_context_window")]
    pub context_window_tokens: u64,
    /// 是否支持图片输入（默认 false，仅文本）
    #[serde(default)]
    pub supports_image: bool,
    /// 是否支持音频输入（默认 false，仅文本）
    #[serde(default)]
    pub supports_audio: bool,
}

fn default_max_tokens() -> u64 { 4096 }
fn default_context_window() -> u64 { 128_000 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Claude,
    OpenAI,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolsMode {
    Standard,
    TextMatch,
}

impl Default for Protocol {
    fn default() -> Self { Protocol::OpenAI }
}
impl Default for ToolsMode {
    fn default() -> Self { ToolsMode::Standard }
}
impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            protocol: Protocol::OpenAI,
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            tools_mode: ToolsMode::Standard,
            max_tokens: 4096,
            temperature: 0.0,
            context_window_tokens: 128_000,
            supports_image: false,
            supports_audio: false,
        }
    }
}

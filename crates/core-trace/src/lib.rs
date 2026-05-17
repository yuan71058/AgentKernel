//! # Core Trace
//!
//! 运行追踪。记录每次推理的完整链路：输入、输出、tool calls、latency、token、errors。

use core_protocol::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceRecord {
    pub trace_id: String,
    pub session_id: String,
    pub run_id: String,
    pub step_type: TraceStepType,
    pub input_summary: String,
    pub output_summary: String,
    pub tool_calls: Vec<ToolCallTrace>,
    pub latency_ms: u64,
    pub tokens: Usage,
    pub error: Option<String>,
    pub context_build: Option<String>,
    pub prompt_list: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallTrace {
    pub tool_name: String,
    pub input: serde_json::Value,
    pub result: String,
    pub is_error: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceStepType {
    #[default]
    ModelCall,
    ToolCall,
    ContextBuild,
    Compression,
}

/// Trace 收集器
pub struct TraceCollector {
    records: std::sync::RwLock<Vec<TraceRecord>>,
}

impl TraceCollector {
    pub fn new() -> Self {
        Self { records: std::sync::RwLock::new(Vec::new()) }
    }

    pub fn record(&self, trace: TraceRecord) {
        self.records.write().unwrap().push(trace);
    }

    pub fn get_by_run(&self, run_id: &str) -> Vec<TraceRecord> {
        self.records.read().unwrap()
            .iter()
            .filter(|r| r.run_id == run_id)
            .cloned()
            .collect()
    }

    pub fn get_by_session(&self, session_id: &str) -> Vec<TraceRecord> {
        self.records.read().unwrap()
            .iter()
            .filter(|r| r.session_id == session_id)
            .cloned()
            .collect()
    }

    pub fn all(&self) -> Vec<TraceRecord> {
        self.records.read().unwrap().clone()
    }
}

impl Default for TraceCollector {
    fn default() -> Self { Self::new() }
}

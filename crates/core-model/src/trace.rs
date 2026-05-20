//! 轻量导出：将 CallTrace 序列化为 JSON/Markdown

use crate::CallTrace;

impl CallTrace {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn to_markdown(&self) -> String {
        let tool_chain_summary = if let Some(report) = &self.tool_chain_report {
            format!(
                "| Original Messages | {} |\n\
                 | Sanitized Messages | {} |\n\
                 | Complete Tool Calls | {} |\n\
                 | Dropped Incomplete Tool Calls | {} |\n\
                 | Dropped Orphan Tool Results | {} |\n",
                report.original_message_count,
                report.sanitized_message_count,
                if report.complete_tool_call_ids.is_empty() { "none".into() } else { report.complete_tool_call_ids.join(", ") },
                if report.dropped_incomplete_tool_call_ids.is_empty() { "none".into() } else { report.dropped_incomplete_tool_call_ids.join(", ") },
                if report.dropped_orphan_tool_result_ids.is_empty() { "none".into() } else { report.dropped_orphan_tool_result_ids.join(", ") },
            )
        } else {
            String::new()
        };
        format!(
            "# API Call Trace\n\n\
             | Field | Value |\n|---|---|\n\
             | Trace ID | {} |\n\
             | Protocol | {} |\n\
             | Model | {} |\n\
             | URL | {} |\n\
             | Method | {} |\n\
             | Response Code | {} |\n\
             | Duration | {}ms |\n\
             | Stream | {} |\n\
             | Finish Reason | {} |\n\
             | Input Tokens | {} |\n\
             | Output Tokens | {} |\n\
             | Error | {} |\n\
             {}",
            self.trace_id,
            self.protocol,
            self.model,
            self.api_url,
            if self.stream { "POST (SSE)" } else { "POST" },
            self.response_code,
            self.duration_ms,
            self.stream,
            self.finish_reason,
            self.input_tokens,
            self.output_tokens,
            self.error.as_deref().unwrap_or("none"),
            tool_chain_summary,
        )
    }
}

//! Project Everruns usage once per reasoning iteration, never from rollups.
//!
//! The same generation is represented in `llm.generation`, message completion,
//! reason completion, and turn/session totals. A recursive JSON walk bills it
//! repeatedly. Prefer the generation record (which also carries folded
//! compaction cost), with reason/message fallbacks for incomplete streams.
//! Everruns input/cache-read/cache-write buckets are disjoint.

use mira::Usage;
use serde_json::Value;

#[derive(Default)]
pub struct Summary {
    pub usage: Usage,
    pub model_calls: u64,
    pub usage_calls: u64,
    pub tool_requests: u64,
    pub cache_creation_tokens: u64,
    pub first_input_tokens: u64,
    pub peak_input_tokens: u64,
    pub first_tool_count: usize,
    pub peak_tool_count: usize,
    pub first_tool_bytes: usize,
    pub peak_tool_bytes: usize,
    pub actual_cost_usd: f64,
    pub actual_cost_calls: u64,
}

impl Summary {
    pub fn apply_to(self, transcript: &mut mira::Transcript) {
        for (name, value) in [
            ("llm.calls", self.model_calls as f64),
            ("llm.usage_calls", self.usage_calls as f64),
            (
                "llm.input_tokens_total",
                (self.usage.input_tokens
                    + self.usage.cache_read_tokens
                    + self.cache_creation_tokens) as f64,
            ),
            (
                "llm.usage_complete",
                f64::from(self.model_calls == self.usage_calls),
            ),
            (
                "llm.cache_creation_tokens",
                self.cache_creation_tokens as f64,
            ),
            ("llm.provider_reported_cost_usd", self.actual_cost_usd),
            ("llm.provider_cost_calls", self.actual_cost_calls as f64),
        ] {
            transcript.record_metric(name, value);
        }
        if self.usage_calls > 0 && self.model_calls == self.usage_calls {
            transcript.record_metric("llm.input_tokens_first", self.first_input_tokens as f64);
            transcript.record_metric("llm.input_tokens_peak", self.peak_input_tokens as f64);
        }
        if self.tool_requests > 0 && self.tool_requests == self.model_calls {
            for (name, value) in [
                ("llm.tools_first", self.first_tool_count as f64),
                ("llm.tools_peak", self.peak_tool_count as f64),
                ("llm.tool_bytes_first", self.first_tool_bytes as f64),
                ("llm.tool_bytes_peak", self.peak_tool_bytes as f64),
                ("llm.tool_requests_captured", self.tool_requests as f64),
            ] {
                transcript.record_metric(name, value);
            }
        }
        transcript.metadata.insert(
            "usage_convention".into(),
            Value::String("everruns-disjoint-v1".into()),
        );
        transcript.metadata.insert(
            "usage_projection".into(),
            Value::String("generation-iteration-v1".into()),
        );
        transcript.usage = self.usage;
    }

    fn add(&mut self, usage: &Value) {
        let input = number(usage, "input_tokens");
        let cached = number(usage, "cache_read_tokens");
        let created = number(usage, "cache_creation_tokens");
        let processed = input + cached + created;
        if self.usage_calls == 0 {
            self.first_input_tokens = processed;
        }
        self.usage_calls += 1;
        self.peak_input_tokens = self.peak_input_tokens.max(processed);
        self.usage.input_tokens += input;
        self.usage.output_tokens += number(usage, "output_tokens");
        self.usage.cache_read_tokens += cached;
        self.usage.reasoning_tokens += number(usage, "reasoning_tokens");
        self.cache_creation_tokens += created;
        self.usage.cost_usd += [
            "effective_cost_usd",
            "actual_cost_usd",
            "estimated_cost_usd",
            "cost_usd",
            "cost",
        ]
        .iter()
        .find_map(|key| usage.get(*key).and_then(Value::as_f64))
        .unwrap_or(0.0);
        if let Some(cost) = usage.get("actual_cost_usd").and_then(Value::as_f64) {
            self.actual_cost_usd += cost;
            self.actual_cost_calls += 1;
        }
    }
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

pub fn summarize_events(events: &[Value]) -> Summary {
    let mut summary = Summary::default();
    let mut pending: Option<(u8, &Value)> = None;
    for event in events {
        let kind = event["type"].as_str().unwrap_or_default();
        let data = &event["data"];
        if kind == "reason.started"
            && let Some((_, usage)) = pending.take()
        {
            summary.add(usage);
        }
        if kind == "llm.generation" {
            // These are the definitions actually offered on this request, not
            // the whole runtime registry. This is a schema-byte measure, not
            // provider-token estimation or a prompt-cache hit measurement.
            // `data.tools` contains display summaries, not parameter schemas.
            // Only a captured driver request can establish schema bytes. For
            // inline providers this measures the baseline `tools` array, not
            // cumulative definitions carried by later conversation items.
            if let Some(tools) = data
                .pointer("/request_body/tools")
                .and_then(Value::as_array)
            {
                let bytes = serde_json::to_vec(tools).map_or(0, |bytes| bytes.len());
                if summary.tool_requests == 0 {
                    summary.first_tool_count = tools.len();
                    summary.first_tool_bytes = bytes;
                }
                summary.peak_tool_count = summary.peak_tool_count.max(tools.len());
                summary.peak_tool_bytes = summary.peak_tool_bytes.max(bytes);
                summary.tool_requests += 1;
            }
            summary.model_calls += 1;
        }
        let candidate = match kind {
            "reason.completed" => data.get("usage").map(|usage| (2, usage)),
            "llm.generation" => data.pointer("/metadata/usage").map(|usage| (3, usage)),
            "output.message.completed" => data
                .pointer("/message/metadata/usage")
                .or_else(|| data.get("usage"))
                .map(|usage| (1, usage)),
            _ => None,
        };
        if let Some((priority, usage)) = candidate
            && usage.get("input_tokens").and_then(Value::as_u64).is_some()
            && pending.as_ref().is_none_or(|(old, _)| priority >= *old)
        {
            pending = Some((priority, usage));
        }
    }
    if let Some((_, usage)) = pending {
        summary.add(usage);
    }
    // A minimal exported stream may retain completion usage without generation
    // events. Do not confuse failed usage-less requests with no requests.
    summary.model_calls = summary.model_calls.max(summary.usage_calls);
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_one_generation_despite_five_event_representations() {
        let usage = json!({"input_tokens":100,"output_tokens":20,"cache_read_tokens":80,
            "actual_cost_usd":0.002,"estimated_cost_usd":0.004});
        let events = vec![
            json!({"type":"reason.started","data":{}}),
            json!({"type":"llm.generation","data":{"metadata":{"usage":usage},"request_body":{"tools":[{"name":"tool_search","parameters":{"type":"object"}}]}}}),
            json!({"type":"output.message.completed","data":{"message":{"metadata":{"usage":usage}}}}),
            json!({"type":"reason.completed","data":{"usage":usage}}),
            json!({"type":"turn.completed","data":{"usage":usage}}),
            json!({"type":"session.idled","data":{"usage":usage}}),
        ];
        let actual = summarize_events(&events);
        assert_eq!(actual.model_calls, 1);
        assert_eq!(actual.usage.input_tokens, 100);
        assert_eq!(actual.usage.output_tokens, 20);
        assert_eq!(actual.usage.cache_read_tokens, 80);
        assert_eq!(actual.first_input_tokens, 180);
        assert_eq!(actual.first_tool_count, 1);
        assert_eq!(actual.usage.cost_usd, 0.002);
    }

    #[test]
    fn handles_multiple_turns_and_an_interrupted_final_iteration() {
        let events = vec![
            json!({"type":"reason.started","data":{}}),
            json!({"type":"llm.generation","data":{"metadata":{"usage":{"input_tokens":10,"output_tokens":2}}}}),
            json!({"type":"reason.completed","data":{"usage":{"input_tokens":10,"output_tokens":2}}}),
            json!({"type":"turn.completed","data":{"usage":{"input_tokens":10}}}),
            json!({"type":"reason.started","data":{}}),
            json!({"type":"llm.generation","data":{"metadata":{"usage":{"input_tokens":20,"output_tokens":3}}}}),
            json!({"type":"output.message.completed","data":{"usage":{"input_tokens":20,"output_tokens":3}}}),
        ];
        let actual = summarize_events(&events);
        assert_eq!(actual.model_calls, 2);
        assert_eq!(actual.usage.input_tokens, 30);
        assert_eq!(actual.usage.output_tokens, 5);
        assert_eq!(actual.peak_input_tokens, 20);
    }

    #[test]
    fn cache_writes_and_effective_cost_are_not_lost_or_double_counted() {
        let actual = summarize_events(&[json!({"type":"reason.completed","data":{"usage":{
            "input_tokens":10,"output_tokens":1,"cache_read_tokens":20,"cache_creation_tokens":30,
            "effective_cost_usd":0.003,"actual_cost_usd":0.002,"estimated_cost_usd":0.004
        }}})]);
        assert_eq!(actual.first_input_tokens, 60);
        assert_eq!(actual.cache_creation_tokens, 30);
        assert_eq!(actual.usage.cost_usd, 0.003);
        assert_eq!(actual.actual_cost_usd, 0.002);
        assert_eq!(actual.actual_cost_calls, 1);
    }

    #[test]
    fn tool_result_json_and_session_rollups_are_not_model_usage() {
        let events = vec![
            json!({"type":"tool.completed","data":{"result":{"input_tokens":9999,"output_tokens":9999}}}),
            json!({"type":"session.idled","data":{"usage":{"input_tokens":9999,"output_tokens":9999}}}),
        ];
        let actual = summarize_events(&events);
        assert_eq!(actual.model_calls, 0);
        assert_eq!(actual.usage.input_tokens, 0);
    }

    #[test]
    fn generation_preserves_folded_compaction_cost() {
        let actual = summarize_events(&[
            json!({"type":"reason.started","data":{}}),
            json!({"type":"llm.generation","data":{"metadata":{"usage":{"input_tokens":10,"actual_cost_usd":0.002,"effective_cost_usd":0.003}}}}),
            json!({"type":"reason.completed","data":{"usage":{"input_tokens":10,"actual_cost_usd":0.002}}}),
        ]);
        assert_eq!(actual.usage.cost_usd, 0.003);
        assert_eq!(actual.usage.input_tokens, 10);
    }

    #[test]
    fn missing_usage_and_uncaptured_schemas_remain_observable() {
        let actual = summarize_events(&[json!({"type":"llm.generation","data":{
            "tools":[{"name":"only_a_display_summary"}],"metadata":{"success":false}
        }})]);
        assert_eq!(actual.model_calls, 1);
        assert_eq!(actual.usage_calls, 0);
        assert_eq!(actual.tool_requests, 0);
        let mut transcript = mira::Transcript::default();
        actual.apply_to(&mut transcript);
        assert!(!transcript.metrics.contains_key("llm.tool_bytes_first"));
    }
}

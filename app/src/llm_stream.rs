use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use crate::present::{self, SseEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmProvider {
    OpenAiChat,
    OpenAiResponses,
    Anthropic,
    Gemini,
}

impl LlmProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAiChat => "OpenAI Chat",
            Self::OpenAiResponses => "OpenAI Responses",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Gemini",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmStreamSummary {
    pub provider: LlmProvider,
    pub model: Option<String>,
    pub finish_reason: Option<String>,
    pub thinking: String,
    pub content: String,
    pub tool_calls: Vec<LlmToolCall>,
    pub usage: LlmUsage,
    pub chunk_count: usize,
    pub ttft_ms: Option<u64>,
    pub total_ms: Option<u64>,
}

impl LlmStreamSummary {
    pub fn tokens_per_sec(&self) -> Option<f64> {
        let out = self.usage.output_tokens?;
        let total = self.total_ms?;
        if out == 0 || total == 0 {
            return None;
        }
        let gen_ms = match self.ttft_ms {
            Some(ttft) if total > ttft => total - ttft,
            _ => total,
        };
        if gen_ms == 0 {
            return None;
        }
        Some(out as f64 / (gen_ms as f64 / 1000.0))
    }
}

#[derive(Default)]
struct Accumulator {
    provider: Option<LlmProvider>,
    model: Option<String>,
    finish_reason: Option<String>,
    thinking: String,
    content: String,
    tools: BTreeMap<usize, LlmToolCall>,
    tool_id_to_slot: BTreeMap<String, usize>,
    next_tool_slot: usize,
    usage: LlmUsage,
    chunk_count: usize,
}

impl Accumulator {
    fn tool_slot(&mut self, index: usize) -> &mut LlmToolCall {
        if index >= self.next_tool_slot {
            self.next_tool_slot = index + 1;
        }
        self.tools.entry(index).or_insert_with(|| LlmToolCall {
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
        })
    }

    fn slot_for_item_id(&mut self, item_id: &str) -> usize {
        if let Some(&slot) = self.tool_id_to_slot.get(item_id) {
            return slot;
        }
        let slot = self.next_tool_slot;
        self.next_tool_slot += 1;
        if !item_id.is_empty() {
            self.tool_id_to_slot.insert(item_id.to_string(), slot);
        }
        self.tools.entry(slot).or_insert_with(|| LlmToolCall {
            id: item_id.to_string(),
            name: String::new(),
            arguments: String::new(),
        });
        slot
    }

    fn record_openai_usage(&mut self, usage: &Value) {
        if let Some(v) = usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_u64)
        {
            self.usage.input_tokens = Some(v);
        }
        if let Some(v) = usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_u64)
        {
            self.usage.output_tokens = Some(v);
        }
        if let Some(v) = usage
            .get("completion_tokens_details")
            .or_else(|| usage.get("output_tokens_details"))
            .and_then(|d| d.get("reasoning_tokens"))
            .and_then(Value::as_u64)
        {
            self.usage.reasoning_tokens = Some(v);
        }
        if let Some(v) = usage.get("total_tokens").and_then(Value::as_u64) {
            self.usage.total_tokens = Some(v);
        }
    }
}

/// Parses an HTTP response body into an aggregated [`LlmStreamSummary`] when it is a
/// recognized LLM `text/event-stream`.
pub fn parse_llm_sse_body(
    response_headers: &[(String, String)],
    response_body: &[u8],
    ttfb: Option<Duration>,
    total: Option<Duration>,
) -> Option<LlmStreamSummary> {
    let kind = present::content_type(response_headers)?;
    if !kind.to_ascii_lowercase().contains("event-stream") {
        return None;
    }
    let text = std::str::from_utf8(response_body).ok()?;
    let events = present::sse_events(text);
    parse_llm_sse_events(&events, ttfb, total)
}

/// Aggregates parsed SSE events from OpenAI Chat, OpenAI Responses, Anthropic, or Gemini.
pub fn parse_llm_sse_events(
    events: &[SseEvent],
    ttfb: Option<Duration>,
    total: Option<Duration>,
) -> Option<LlmStreamSummary> {
    let mut acc = Accumulator::default();
    for ev in events {
        let data = ev.data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if consume_event(&mut acc, &ev.event, &value) {
            acc.chunk_count += 1;
        }
    }
    let provider = acc.provider?;
    if acc.chunk_count == 0 {
        return None;
    }
    Some(LlmStreamSummary {
        provider,
        model: acc.model,
        finish_reason: acc.finish_reason,
        thinking: acc.thinking,
        content: acc.content,
        tool_calls: acc.tools.into_values().collect(),
        usage: acc.usage,
        chunk_count: acc.chunk_count,
        ttft_ms: ttfb.map(|d| d.as_millis() as u64),
        total_ms: total.map(|d| d.as_millis() as u64),
    })
}

fn consume_event(acc: &mut Accumulator, event_name: &str, value: &Value) -> bool {
    if is_openai_chat(value) {
        consume_openai_chat(acc, value);
        return true;
    }
    let ev_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(event_name);
    if ev_type.starts_with("response.") {
        consume_openai_responses(acc, ev_type, value);
        return true;
    }
    if is_anthropic_event(ev_type) {
        consume_anthropic(acc, ev_type, value);
        return true;
    }
    if is_gemini_chunk(value) {
        consume_gemini(acc, value);
        return true;
    }
    false
}

fn is_openai_chat(value: &Value) -> bool {
    value
        .get("object")
        .and_then(Value::as_str)
        .is_some_and(|obj| obj == "chat.completion.chunk")
        || value
            .get("choices")
            .and_then(Value::as_array)
            .is_some_and(|arr| {
                arr.first()
                    .and_then(|c| c.get("delta"))
                    .is_some_and(Value::is_object)
            })
}

fn consume_openai_chat(acc: &mut Accumulator, value: &Value) {
    acc.provider.get_or_insert(LlmProvider::OpenAiChat);
    if let Some(model) = value.get("model").and_then(Value::as_str)
        && !model.is_empty()
    {
        acc.model = Some(model.to_string());
    }
    if let Some(usage) = value.get("usage").filter(|u| u.is_object()) {
        acc.record_openai_usage(usage);
    }
    let Some(choice) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
    else {
        return;
    };
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str)
        && !reason.is_empty()
    {
        acc.finish_reason = Some(reason.to_string());
    }
    let Some(delta) = choice.get("delta") else {
        return;
    };
    if let Some(text) = delta.get("content").and_then(Value::as_str) {
        acc.content.push_str(text);
    }
    if let Some(reason_text) = delta
        .get("reasoning_content")
        .or_else(|| delta.get("reasoning"))
        .and_then(Value::as_str)
    {
        acc.thinking.push_str(reason_text);
    }
    if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
        for tc in tcs {
            let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let slot = acc.tool_slot(idx);
            if let Some(id) = tc.get("id").and_then(Value::as_str)
                && !id.is_empty()
            {
                slot.id = id.to_string();
            }
            if let Some(func) = tc.get("function") {
                if let Some(name) = func.get("name").and_then(Value::as_str)
                    && !name.is_empty()
                {
                    slot.name = name.to_string();
                }
                if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                    slot.arguments.push_str(args);
                }
            }
        }
    }
}

fn consume_openai_responses(acc: &mut Accumulator, ev_type: &str, value: &Value) {
    acc.provider.get_or_insert(LlmProvider::OpenAiResponses);
    match ev_type {
        "response.output_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                acc.content.push_str(delta);
            }
        }
        "response.reasoning_summary_text.delta"
        | "response.reasoning_text.delta"
        | "response.reasoning.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                acc.thinking.push_str(delta);
            }
        }
        "response.output_item.added" => {
            let Some(item) = value.get("item") else {
                return;
            };
            if item.get("type").and_then(Value::as_str) != Some("function_call") {
                return;
            }
            let item_id = item.get("id").and_then(Value::as_str).unwrap_or("");
            let slot_idx = if !item_id.is_empty() {
                acc.slot_for_item_id(item_id)
            } else {
                value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize
            };
            let call_id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let name = item.get("name").and_then(Value::as_str).unwrap_or("");
            let slot = acc.tool_slot(slot_idx);
            if !call_id.is_empty() {
                slot.id = call_id.to_string();
            }
            if !name.is_empty() {
                slot.name = name.to_string();
            }
        }
        "response.function_call_arguments.delta" => {
            let item_id = value.get("item_id").and_then(Value::as_str).unwrap_or("");
            let slot_idx = if !item_id.is_empty() {
                acc.slot_for_item_id(item_id)
            } else {
                value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize
            };
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                acc.tool_slot(slot_idx).arguments.push_str(delta);
            }
        }
        "response.created" | "response.in_progress" | "response.completed" => {
            let resp = value.get("response").unwrap_or(value);
            if let Some(model) = resp.get("model").and_then(Value::as_str)
                && !model.is_empty()
            {
                acc.model = Some(model.to_string());
            }
            if ev_type == "response.completed" {
                if let Some(status) = resp.get("status").and_then(Value::as_str)
                    && !status.is_empty()
                {
                    acc.finish_reason = Some(status.to_string());
                }
                if let Some(usage) = resp.get("usage").filter(|u| u.is_object()) {
                    acc.record_openai_usage(usage);
                }
            }
        }
        _ => {}
    }
}

fn is_anthropic_event(ev_type: &str) -> bool {
    matches!(
        ev_type,
        "message_start"
            | "content_block_start"
            | "content_block_delta"
            | "content_block_stop"
            | "message_delta"
            | "message_stop"
    )
}

fn consume_anthropic(acc: &mut Accumulator, ev_type: &str, value: &Value) {
    acc.provider.get_or_insert(LlmProvider::Anthropic);
    match ev_type {
        "message_start" => {
            let msg = value.get("message").unwrap_or(value);
            if let Some(model) = msg.get("model").and_then(Value::as_str)
                && !model.is_empty()
            {
                acc.model = Some(model.to_string());
            }
            if let Some(inp) = msg
                .get("usage")
                .and_then(|u| u.get("input_tokens"))
                .and_then(Value::as_u64)
            {
                acc.usage.input_tokens = Some(inp);
            }
        }
        "content_block_start" => {
            let idx = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let Some(block) = value.get("content_block") else {
                return;
            };
            if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                let slot = acc.tool_slot(idx);
                if let Some(id) = block.get("id").and_then(Value::as_str) {
                    slot.id = id.to_string();
                }
                if let Some(name) = block.get("name").and_then(Value::as_str) {
                    slot.name = name.to_string();
                }
            }
        }
        "content_block_delta" => {
            let idx = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let Some(delta) = value.get("delta") else {
                return;
            };
            match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                "text_delta" => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        acc.content.push_str(text);
                    }
                }
                "thinking_delta" => {
                    if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                        acc.thinking.push_str(text);
                    }
                }
                "input_json_delta" => {
                    if let Some(part) = delta.get("partial_json").and_then(Value::as_str) {
                        acc.tool_slot(idx).arguments.push_str(part);
                    }
                }
                _ => {}
            }
        }
        "message_delta" => {
            if let Some(reason) = value
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(Value::as_str)
                && !reason.is_empty()
            {
                acc.finish_reason = Some(reason.to_string());
            }
            if let Some(out) = value
                .get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(Value::as_u64)
            {
                acc.usage.output_tokens = Some(out);
            }
        }
        _ => {}
    }
}

fn is_gemini_chunk(value: &Value) -> bool {
    value
        .get("candidates")
        .and_then(Value::as_array)
        .is_some_and(|arr| {
            arr.first()
                .and_then(|c| c.get("content"))
                .and_then(|ct| ct.get("parts"))
                .is_some_and(Value::is_array)
        })
}

fn consume_gemini(acc: &mut Accumulator, value: &Value) {
    acc.provider.get_or_insert(LlmProvider::Gemini);
    if let Some(model) = value.get("modelVersion").and_then(Value::as_str)
        && !model.is_empty()
    {
        acc.model = Some(model.to_string());
    }
    if let Some(usage) = value.get("usageMetadata") {
        if let Some(v) = usage.get("promptTokenCount").and_then(Value::as_u64) {
            acc.usage.input_tokens = Some(v);
        }
        if let Some(v) = usage.get("candidatesTokenCount").and_then(Value::as_u64) {
            acc.usage.output_tokens = Some(v);
        }
        if let Some(v) = usage.get("thoughtsTokenCount").and_then(Value::as_u64) {
            acc.usage.reasoning_tokens = Some(v);
        }
        if let Some(v) = usage.get("totalTokenCount").and_then(Value::as_u64) {
            acc.usage.total_tokens = Some(v);
        }
    }
    let Some(cand) = value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
    else {
        return;
    };
    if let Some(reason) = cand.get("finishReason").and_then(Value::as_str)
        && !reason.is_empty()
    {
        acc.finish_reason = Some(reason.to_string());
    }
    let Some(parts) = cand
        .get("content")
        .and_then(|ct| ct.get("parts"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for part in parts {
        if part.get("thought").and_then(Value::as_bool) == Some(true) {
            if let Some(t) = part.get("text").and_then(Value::as_str) {
                acc.thinking.push_str(t);
            }
        } else if let Some(t) = part.get("text").and_then(Value::as_str) {
            acc.content.push_str(t);
        }
        if let Some(fc) = part.get("functionCall").and_then(Value::as_object) {
            let slot_idx = acc.next_tool_slot;
            let name = fc
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let id = fc
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("call_{}", slot_idx + 1));
            let arguments = fc
                .get("args")
                .and_then(|args| serde_json::to_string(args).ok())
                .unwrap_or_default();
            let slot = acc.tool_slot(slot_idx);
            slot.id = id;
            slot.name = name;
            slot.arguments = arguments;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_chat_stream_with_reasoning_tool_calls_and_usage() {
        let sse = concat!(
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4.1\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"Check \"}}]}\n\n",
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4.1\",\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"weather\",\"content\":\"Sure\"}}]}\n\n",
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"!\",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"city\\\":\"}}]}}]}\n\n",
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"Tokyo\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":25,\"total_tokens\":37,\"completion_tokens_details\":{\"reasoning_tokens\":10}}}\n\n",
            "data: [DONE]\n\n"
        );
        let headers = vec![("Content-Type".into(), "text/event-stream".into())];
        let summary = parse_llm_sse_body(
            &headers,
            sse.as_bytes(),
            Some(Duration::from_millis(100)),
            Some(Duration::from_millis(600)),
        )
        .unwrap();
        assert_eq!(summary.provider, LlmProvider::OpenAiChat);
        assert_eq!(summary.model.as_deref(), Some("gpt-4.1"));
        assert_eq!(summary.thinking, "Check weather");
        assert_eq!(summary.content, "Sure!");
        assert_eq!(summary.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(summary.tool_calls.len(), 1);
        assert_eq!(summary.tool_calls[0].id, "call_1");
        assert_eq!(summary.tool_calls[0].name, "get_weather");
        assert_eq!(summary.tool_calls[0].arguments, "{\"city\":\"Tokyo\"}");
        assert_eq!(summary.usage.input_tokens, Some(12));
        assert_eq!(summary.usage.output_tokens, Some(25));
        assert_eq!(summary.usage.reasoning_tokens, Some(10));
        assert_eq!(summary.usage.total_tokens, Some(37));
        assert_eq!(summary.tokens_per_sec(), Some(50.0));
    }

    #[test]
    fn parses_openai_responses_anthropic_and_gemini_and_ignores_generic_sse() {
        let responses_sse = concat!(
            "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"model\":\"o4-mini\"}}\n\n",
            "event: response.reasoning_summary_text.delta\ndata: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"Plan \"}\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Done\"}\n\n",
            "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_r1\",\"name\":\"lookup\"}}\n\n",
            "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"q\\\":\\\"zniff\\\"}\"}\n\n",
            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":8,\"total_tokens\":13}}}\n\n"
        );
        let r = parse_llm_sse_events(&present::sse_events(responses_sse), None, None).unwrap();
        assert_eq!(r.provider, LlmProvider::OpenAiResponses);
        assert_eq!(r.model.as_deref(), Some("o4-mini"));
        assert_eq!(r.thinking, "Plan ");
        assert_eq!(r.content, "Done");
        assert_eq!(r.tool_calls[0].name, "lookup");
        assert_eq!(r.tool_calls[0].arguments, "{\"q\":\"zniff\"}");
        assert_eq!(r.finish_reason.as_deref(), Some("completed"));

        let claude_sse = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":21}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Step 1\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"search\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"k\\\":1}\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":14}}\n\n"
        );
        let a = parse_llm_sse_events(&present::sse_events(claude_sse), None, None).unwrap();
        assert_eq!(a.provider, LlmProvider::Anthropic);
        assert_eq!(a.model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(a.thinking, "Step 1");
        assert_eq!(a.content, "Hello");
        assert_eq!(a.tool_calls[0].id, "toolu_1");
        assert_eq!(a.tool_calls[0].arguments, "{\"k\":1}");
        assert_eq!(a.usage.input_tokens, Some(21));
        assert_eq!(a.usage.output_tokens, Some(14));

        let gemini_sse = "data: {\"modelVersion\":\"gemini-2.5-pro\",\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Reasoning\",\"thought\":true},{\"text\":\"Answer\"},{\"functionCall\":{\"name\":\"fetch\",\"args\":{\"url\":\"https://api.vicanso.dev\"}}}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":9,\"candidatesTokenCount\":11,\"thoughtsTokenCount\":4,\"totalTokenCount\":24}}\n\n";
        let g = parse_llm_sse_events(&present::sse_events(gemini_sse), None, None).unwrap();
        assert_eq!(g.provider, LlmProvider::Gemini);
        assert_eq!(g.model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(g.thinking, "Reasoning");
        assert_eq!(g.content, "Answer");
        assert_eq!(g.tool_calls[0].name, "fetch");
        assert_eq!(g.usage.reasoning_tokens, Some(4));

        let non_llm = "event: ping\ndata: {\"ok\":true}\n\n";
        assert!(parse_llm_sse_events(&present::sse_events(non_llm), None, None).is_none());
    }
}

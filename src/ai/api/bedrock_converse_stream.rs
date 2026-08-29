//! Port of `pi-core/ai/src/api/bedrock-converse-stream.ts`: request building
//! and response-event handling for the Bedrock converse-stream API.
//!
//! The TypeScript implementation routes through the AWS SDK
//! (`@aws-sdk/client-bedrock-runtime`); the Rust port builds the same request
//! payload (observable through `onPayload`) and drives the same event-item
//! sequence. [`BedrockDispatchResponse`] plus [`stream_from_items`] replace
//! the SDK dispatch — the seam the TypeScript tests create by mocking the
//! SDK client. The wire transport composes the same driver.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::ai::api::constrained_sampling::{
    get_json_schema_tool_parameters, resolve_json_schema_strict_sampling,
};
use crate::ai::api::simple_options::{
    adjust_max_tokens_for_thinking, build_base_options, clamp_max_tokens_to_context,
    clamp_reasoning,
};
use crate::ai::api::transform_messages::transform_messages;
use crate::ai::models::calculate_cost;
use crate::ai::types::{
    AssistantContent, AssistantMessage, AssistantMessageDiagnostic, AssistantMessageEvent,
    BlockContent, CacheRetention, Context, Message, Model, ModelThinkingLevel, ProviderEnv,
    SimpleStreamOptions, StreamOptions, ThinkingBudgets, ThinkingContent, ThinkingLevel, Tool,
    ToolCall, ToolChoice, UserContent,
};
use crate::ai::utils::diagnostics::append_assistant_message_diagnostic;
use crate::ai::utils::error_body::{ProviderErrorParts, normalize_provider_error};
use crate::ai::utils::event_stream::{
    AssistantMessageEventStream, create_assistant_message_event_stream,
};
use crate::ai::utils::json_parse::parse_streaming_json;
use crate::ai::utils::provider_env::get_provider_env_value;
use crate::ai::utils::sanitize_unicode::sanitize_surrogates;

/// Port of `BedrockThinkingDisplay`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BedrockThinkingDisplay {
    Summarized,
    Omitted,
}

/// Port of `BedrockOptions`: `StreamOptions` plus the Bedrock fields.
#[derive(Clone, Default)]
pub struct BedrockOptions {
    pub base: StreamOptions,
    pub region: Option<String>,
    pub profile: Option<String>,
    pub tool_choice: Option<ToolChoice>,
    pub reasoning: Option<ThinkingLevel>,
    pub thinking_budgets: Option<ThinkingBudgets>,
    pub interleaved_thinking: Option<bool>,
    pub thinking_display: Option<BedrockThinkingDisplay>,
    pub request_metadata: Option<BTreeMap<String, String>>,
    pub bearer_token: Option<String>,
}

const EMPTY_TEXT_PLACEHOLDER: &str = "<empty>";

/// Matches the placeholder the Anthropic API path uses for redacted thinking.
const REDACTED_THINKING_PLACEHOLDER: &str = "[Reasoning redacted]";

/// The error shape the TypeScript code inspects on caught SDK exceptions.
#[derive(Clone, Debug, Default)]
pub struct BedrockError {
    /// `error.name` — modeled service exceptions end in `Exception`.
    pub name: String,
    pub message: String,
    pub status: Option<u16>,
    pub body: Option<String>,
    /// `$metadata.httpStatusCode`.
    pub http_status_code: Option<u16>,
    /// `$metadata.requestId`.
    pub request_id: Option<String>,
    /// `error instanceof BedrockRuntimeServiceException`.
    pub service_exception: bool,
}

impl BedrockError {
    /// A transport error (not an SDK service exception).
    pub fn transport(message: impl Into<String>) -> Self {
        BedrockError {
            message: message.into(),
            ..Default::default()
        }
    }
}

/// What the AWS SDK's `client.send()` returns in TypeScript: response
/// `$metadata` plus the stream of ConverseStreamOutput items.
#[derive(Clone, Debug, Default)]
pub struct BedrockDispatchResponse {
    pub http_status_code: Option<u16>,
    pub request_id: Option<String>,
    pub items: Vec<Value>,
    /// `client.send()` rejecting.
    pub send_error: Option<BedrockError>,
    /// The item iterator throwing mid-stream, after yielding `items`.
    pub stream_error: Option<BedrockError>,
}

/// The streaming block scratch state; the TS code keeps these fields on the
/// content blocks themselves and strips them at finalize.
#[derive(Clone, Debug)]
enum StreamingBlock {
    Text {
        index: Option<u64>,
        text: String,
    },
    Thinking {
        index: Option<u64>,
        thinking: String,
        thinking_signature: String,
        redacted: bool,
        redacted_chunks: Vec<Vec<u8>>,
    },
    ToolCall {
        index: Option<u64>,
        id: String,
        name: String,
        arguments: serde_json::Map<String, Value>,
        partial_json: String,
    },
}

impl StreamingBlock {
    fn index(&self) -> Option<u64> {
        match self {
            StreamingBlock::Text { index, .. }
            | StreamingBlock::Thinking { index, .. }
            | StreamingBlock::ToolCall { index, .. } => *index,
        }
    }

    /// The finalized content block (scratch fields stripped).
    fn into_content(self) -> AssistantContent {
        match self {
            StreamingBlock::Text { text, .. } => {
                AssistantContent::Text(crate::ai::types::TextContent {
                    text,
                    ..Default::default()
                })
            }
            StreamingBlock::Thinking {
                thinking,
                thinking_signature,
                redacted,
                redacted_chunks,
                ..
            } => {
                // Encrypted reasoning encodes into `thinkingSignature`; the
                // scratch buffer must never reach a persisted message.
                let thinking_signature = if redacted && !redacted_chunks.is_empty() {
                    Some(bytes_to_base64(&redacted_chunks))
                } else if thinking_signature.is_empty() {
                    None
                } else {
                    Some(thinking_signature)
                };
                AssistantContent::Thinking(ThinkingContent {
                    thinking,
                    thinking_signature,
                    redacted: redacted.then_some(true),
                    ..Default::default()
                })
            }
            StreamingBlock::ToolCall {
                id,
                name,
                arguments,
                ..
            } => AssistantContent::ToolCall(ToolCall {
                id,
                name,
                arguments,
                ..Default::default()
            }),
        }
    }
}

/// Port of `bytesToBase64` over the redacted chunk buffer.
fn bytes_to_base64(chunks: &[Vec<u8>]) -> String {
    use base64::Engine;
    let joined: Vec<u8> = chunks.concat();
    base64::engine::general_purpose::STANDARD.encode(joined)
}

/// Port of `base64ToBytes`; a non-base64 payload yields no bytes.
fn base64_to_bytes(data: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

fn now_millis() -> i64 {
    crate::ai::auth::resolve::now_millis()
}

/// `new URL(url).hostname` for the URL shapes Bedrock base URLs take.
#[cfg_attr(not(test), allow(dead_code))]
fn parse_hostname(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = rest.split('/').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

// ---------------------------------------------------------------------------
// Request building
// ---------------------------------------------------------------------------

/// Port of `getModelMatchCandidates`: the lowercased id/name plus the
/// separator-normalized variants (application inference profile support).
fn model_match_candidates(model_id: &str, model_name: Option<&str>) -> Vec<String> {
    let mut values = vec![model_id.to_string()];
    if let Some(name) = model_name {
        values.push(name.to_string());
    }
    values
        .into_iter()
        .flat_map(|value| {
            let lower = value.to_lowercase();
            let normalized = lower
                .chars()
                .map(|c| {
                    if c.is_whitespace() || matches!(c, '_' | '.' | ':') {
                        '-'
                    } else {
                        c
                    }
                })
                .collect::<String>();
            vec![lower, normalized]
        })
        .collect()
}

fn supports_adaptive_thinking(model_id: &str, model_name: Option<&str>) -> bool {
    model_match_candidates(model_id, model_name)
        .iter()
        .any(|s| {
            s.contains("opus-4-6")
                || s.contains("opus-4-7")
                || s.contains("opus-4-8")
                || s.contains("opus-5")
                || s.contains("sonnet-4-6")
                || s.contains("sonnet-5")
                || s.contains("fable-5")
        })
}

fn model_thinking_level(level: ThinkingLevel) -> ModelThinkingLevel {
    match level {
        ThinkingLevel::Minimal => ModelThinkingLevel::Minimal,
        ThinkingLevel::Low => ModelThinkingLevel::Low,
        ThinkingLevel::Medium => ModelThinkingLevel::Medium,
        ThinkingLevel::High => ModelThinkingLevel::High,
        ThinkingLevel::Xhigh => ModelThinkingLevel::Xhigh,
        ThinkingLevel::Max => ModelThinkingLevel::Max,
    }
}

fn supports_native_xhigh_effort(model: &Model) -> bool {
    model_match_candidates(&model.id, Some(&model.name))
        .iter()
        .any(|s| {
            s.contains("opus-4-7")
                || s.contains("opus-4-8")
                || s.contains("opus-5")
                || s.contains("sonnet-5")
                || s.contains("fable-5")
        })
}

fn map_thinking_level_to_effort(model: &Model, level: Option<ThinkingLevel>) -> String {
    if level == Some(ThinkingLevel::Xhigh) && supports_native_xhigh_effort(model) {
        return "xhigh".to_string();
    }
    if let Some(level) = level
        && let Some(mapped) = model
            .thinking_level_map
            .as_ref()
            .and_then(|map| map.get(&model_thinking_level(level)))
        && let Some(mapped) = mapped.as_deref()
    {
        return match mapped {
            "low" | "medium" | "high" | "xhigh" | "max" => mapped.to_string(),
            _ => "high".to_string(),
        };
    }
    match level {
        Some(ThinkingLevel::Minimal) | Some(ThinkingLevel::Low) => "low".to_string(),
        Some(ThinkingLevel::Medium) => "medium".to_string(),
        _ => "high".to_string(),
    }
}

/// Port of `resolveCacheRetention`: defaults to "short", `PI_CACHE_RETENTION`
/// selects "long" for compatibility.
fn resolve_cache_retention(
    cache_retention: Option<CacheRetention>,
    env: Option<&ProviderEnv>,
) -> CacheRetention {
    if let Some(cache_retention) = cache_retention {
        return cache_retention;
    }
    if get_provider_env_value("PI_CACHE_RETENTION", env).as_deref() == Some("long") {
        return CacheRetention::Long;
    }
    CacheRetention::Short
}

fn is_anthropic_claude_model(model: &Model) -> bool {
    let id = model.id.to_lowercase();
    let name = model.name.to_lowercase();
    id.contains("anthropic.claude")
        || id.contains("anthropic/claude")
        || name.contains("anthropic.claude")
        || name.contains("anthropic/claude")
        || name.contains("claude")
}

fn supports_prompt_caching(model: &Model, env: Option<&ProviderEnv>) -> bool {
    let candidates = model_match_candidates(&model.id, Some(&model.name));
    let has_claude_ref = candidates.iter().any(|s| s.contains("claude"));
    if !has_claude_ref {
        return get_provider_env_value("AWS_BEDROCK_FORCE_CACHE", env).as_deref() == Some("1");
    }
    candidates.iter().any(|s| {
        s.contains("fable-5")
            || s.contains("opus-5")
            || s.contains("sonnet-5")
            || s.contains("-4-")
            || s.contains("claude-3-7-sonnet")
            || s.contains("claude-3-5-haiku")
    })
}

fn supports_thinking_signature(model: &Model) -> bool {
    is_anthropic_claude_model(model)
}

fn cache_point_block(retention: CacheRetention) -> Value {
    match retention {
        CacheRetention::Long => json!({"cachePoint": {"type": "default", "ttl": "ONE_HOUR"}}),
        _ => json!({"cachePoint": {"type": "default"}}),
    }
}

/// Port of `buildSystemPrompt`.
fn build_system_prompt(
    system_prompt: Option<&str>,
    model: &Model,
    cache_retention: CacheRetention,
    env: Option<&ProviderEnv>,
) -> Option<Vec<Value>> {
    let system_prompt = system_prompt?;
    let mut blocks = vec![json!({ "text": sanitize_surrogates(system_prompt) })];
    if cache_retention != CacheRetention::None && supports_prompt_caching(model, env) {
        blocks.push(cache_point_block(cache_retention));
    }
    Some(blocks)
}

/// Port of `normalizeToolCallId`.
fn normalize_tool_call_id(id: &str) -> String {
    let sanitized: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    sanitized.chars().take(64).collect()
}

fn create_non_blank_text_block(text: &str) -> Option<Value> {
    let sanitized = sanitize_surrogates(text);
    (!sanitized.trim().is_empty()).then(|| json!({ "text": sanitized }))
}

fn create_required_text_block(text: &str) -> Value {
    create_non_blank_text_block(text).unwrap_or(json!({ "text": EMPTY_TEXT_PLACEHOLDER }))
}

/// Port of `sanitizeBedrockDocument`: drops empty keys, recurses.
fn sanitize_bedrock_document(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sanitize_bedrock_document).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| !key.is_empty())
                .map(|(key, nested)| (key.clone(), sanitize_bedrock_document(nested)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Port of `createImageBlock`; unknown mime types error like the TS path.
fn create_image_block(mime_type: &str, data: &str) -> Value {
    let format = match mime_type {
        "image/jpeg" | "image/jpg" => "jpeg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        other => panic!("Unknown image type: {other}"),
    };
    let bytes = base64_to_bytes(data).unwrap_or_default();
    json!({ "format": format, "source": { "bytes": bytes } })
}

fn convert_tool_result_content(content: &[BlockContent]) -> Vec<Value> {
    let mut result = Vec::new();
    for c in content {
        match c {
            BlockContent::Image(image) => result.push(json!({
                "image": create_image_block(&image.mime_type, &image.data)
            })),
            BlockContent::Text(text) => {
                if let Some(block) = create_non_blank_text_block(&text.text) {
                    result.push(block);
                }
            }
        }
    }
    if result.is_empty() {
        result.push(json!({ "text": EMPTY_TEXT_PLACEHOLDER }));
    }
    result
}

/// Port of `convertMessages`.
fn convert_messages(
    context: &Context,
    model: &Model,
    cache_retention: CacheRetention,
    env: Option<&ProviderEnv>,
) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();
    let transformed = transform_messages(
        &context.messages,
        model,
        Some(&|id: &str, _message: &AssistantMessage| normalize_tool_call_id(id)),
    );

    let mut messages = transformed.into_iter().peekable();
    while let Some(message) = messages.next() {
        match &message {
            Message::User(user) => {
                let mut content: Vec<Value> = Vec::new();
                match &user.content {
                    UserContent::Text(text) => content.push(create_required_text_block(text)),
                    UserContent::Blocks(items) => {
                        for c in items {
                            match c {
                                BlockContent::Text(text) => {
                                    if let Some(block) = create_non_blank_text_block(&text.text) {
                                        content.push(block);
                                    }
                                }
                                BlockContent::Image(image) => {
                                    content.push(create_image_block(&image.mime_type, &image.data))
                                }
                            }
                        }
                        if content.is_empty() {
                            content.push(json!({ "text": EMPTY_TEXT_PLACEHOLDER }));
                        }
                    }
                }
                result.push(json!({ "role": "user", "content": content }));
            }
            Message::Assistant(assistant) => {
                // Bedrock rejects empty assistant content arrays.
                if assistant.content.is_empty() {
                    continue;
                }
                let mut blocks: Vec<Value> = Vec::new();
                for c in &assistant.content {
                    match c {
                        AssistantContent::Text(text) => {
                            if let Some(block) = create_non_blank_text_block(&text.text) {
                                blocks.push(block);
                            }
                        }
                        AssistantContent::ToolCall(call) => blocks.push(json!({
                            "toolUse": {
                                "toolUseId": call.id,
                                "name": call.name,
                                "input": sanitize_bedrock_document(&Value::Object(call.arguments.clone())),
                            }
                        })),
                        AssistantContent::Thinking(thinking) => {
                            if thinking.redacted.unwrap_or(false) {
                                if let Some(bytes) = thinking
                                    .thinking_signature
                                    .as_deref()
                                    .and_then(base64_to_bytes)
                                    .filter(|bytes| !bytes.is_empty())
                                {
                                    blocks.push(json!({
                                        "reasoningContent": { "redactedContent": bytes }
                                    }));
                                }
                                continue;
                            }
                            let thinking_text = sanitize_surrogates(&thinking.thinking);
                            if thinking_text.trim().is_empty() {
                                continue;
                            }
                            if supports_thinking_signature(model) {
                                let signature = thinking
                                    .thinking_signature
                                    .as_deref()
                                    .unwrap_or_default()
                                    .trim()
                                    .to_string();
                                if signature.is_empty() {
                                    blocks.push(json!({ "text": thinking_text }));
                                } else {
                                    blocks.push(json!({
                                        "reasoningContent": {
                                            "reasoningText": {
                                                "text": thinking_text,
                                                "signature": signature,
                                            }
                                        }
                                    }));
                                }
                            } else {
                                blocks.push(json!({
                                    "reasoningContent": { "reasoningText": { "text": thinking_text } }
                                }));
                            }
                        }
                    }
                }
                if blocks.is_empty() {
                    continue;
                }
                result.push(json!({ "role": "assistant", "content": blocks }));
            }
            Message::ToolResult(tool_result) => {
                // Consecutive tool results merge into one user message.
                let mut tool_results = vec![tool_result_block(tool_result)];
                while let Some(next) = messages.peek() {
                    match next {
                        Message::ToolResult(next_result) => {
                            tool_results.push(tool_result_block(next_result));
                            messages.next();
                        }
                        _ => break,
                    }
                }
                result.push(json!({ "role": "user", "content": tool_results }));
            }
        }
    }

    // Cache point on the last user message for supported Claude models.
    if cache_retention != CacheRetention::None
        && supports_prompt_caching(model, env)
        && let Some(last) = result.last_mut()
        && last.get("role").and_then(Value::as_str) == Some("user")
        && let Some(content) = last.get_mut("content").and_then(Value::as_array_mut)
    {
        content.push(cache_point_block(cache_retention));
    }
    result
}

fn tool_result_block(tool_result: &crate::ai::types::ToolResultMessage) -> Value {
    json!({
        "toolResult": {
            "toolUseId": tool_result.tool_call_id,
            "content": convert_tool_result_content(&tool_result.content),
            "status": if tool_result.is_error { "error" } else { "success" },
        }
    })
}

/// Port of `convertToolConfig`.
fn convert_tool_config(
    tools: Option<&Vec<Tool>>,
    tool_choice: Option<&ToolChoice>,
    supports_strict_mode: bool,
) -> Option<Value> {
    let tools = tools?;
    if tools.is_empty() {
        return None;
    }
    if matches!(tool_choice, Some(ToolChoice::None)) {
        return None;
    }
    let bedrock_tools: Vec<Value> = tools
        .iter()
        .map(|tool| {
            let strict = resolve_json_schema_strict_sampling(tool, supports_strict_mode)
                .ok()
                .flatten();
            let parameters = get_json_schema_tool_parameters(tool, strict.unwrap_or(false))
                .unwrap_or_else(|_| tool.parameters.clone());
            let mut tool_spec = json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": { "json": parameters },
            });
            if strict == Some(true) {
                tool_spec["strict"] = json!(true);
            }
            json!({ "toolSpec": tool_spec })
        })
        .collect();
    let tool_choice_value = match tool_choice {
        Some(ToolChoice::Auto) => json!({ "auto": {} }),
        Some(ToolChoice::Any) => json!({ "any": {} }),
        Some(ToolChoice::Tool { name }) => json!({ "tool": { "name": name } }),
        _ => Value::Null,
    };
    Some(json!({
        "tools": bedrock_tools,
        "toolChoice": tool_choice_value,
    }))
}

/// Port of `buildAdditionalModelRequestFields`.
fn build_additional_model_request_fields(model: &Model, options: &BedrockOptions) -> Option<Value> {
    options.reasoning?;
    if !model.reasoning {
        return None;
    }
    if !is_anthropic_claude_model(model) {
        return None;
    }
    // GovCloud Bedrock rejects the Claude thinking.display field.
    let display = if is_gov_cloud_bedrock_target(model, options) {
        None
    } else {
        Some(
            match options
                .thinking_display
                .unwrap_or(BedrockThinkingDisplay::Summarized)
            {
                BedrockThinkingDisplay::Summarized => "summarized",
                BedrockThinkingDisplay::Omitted => "omitted",
            },
        )
    };
    let mut result = serde_json::Map::new();
    if supports_adaptive_thinking(&model.id, Some(&model.name)) {
        let mut thinking = json!({ "type": "adaptive" });
        if let Some(display) = display {
            thinking["display"] = json!(display);
        }
        result.insert("thinking".to_string(), thinking);
        result.insert(
            "output_config".to_string(),
            json!({ "effort": map_thinking_level_to_effort(model, options.reasoning) }),
        );
    } else {
        let default_budget = |level: ThinkingLevel| match level {
            ThinkingLevel::Minimal => 1024,
            ThinkingLevel::Low => 2048,
            ThinkingLevel::Medium => 8192,
            ThinkingLevel::High | ThinkingLevel::Xhigh | ThinkingLevel::Max => 16384,
        };
        // Custom budgets only cover token-based levels through high.
        let budget = match options.reasoning {
            Some(ThinkingLevel::Xhigh) | Some(ThinkingLevel::Max) => options
                .thinking_budgets
                .as_ref()
                .and_then(|budgets| budgets.high)
                .unwrap_or(default_budget(options.reasoning.unwrap())),
            level => options
                .thinking_budgets
                .as_ref()
                .and_then(|budgets| budget_for_level(budgets, level.unwrap()))
                .unwrap_or_else(|| default_budget(level.unwrap())),
        };
        let mut thinking = json!({ "type": "enabled", "budget_tokens": budget });
        if let Some(display) = display {
            thinking["display"] = json!(display);
        }
        result.insert("thinking".to_string(), thinking);
    }
    if !supports_adaptive_thinking(&model.id, Some(&model.name))
        && options.interleaved_thinking.unwrap_or(true)
    {
        result.insert(
            "anthropic_beta".to_string(),
            json!(["interleaved-thinking-2025-05-14"]),
        );
    }
    Some(Value::Object(result))
}

fn budget_for_level(budgets: &ThinkingBudgets, level: ThinkingLevel) -> Option<u64> {
    match level {
        ThinkingLevel::Minimal => budgets.minimal,
        ThinkingLevel::Low => budgets.low,
        ThinkingLevel::Medium => budgets.medium,
        ThinkingLevel::High => budgets.high,
        ThinkingLevel::Xhigh | ThinkingLevel::Max => None,
    }
}

/// Port of the region/config helpers shared with the wire layer.
pub(crate) fn get_configured_bedrock_region(
    region: Option<&str>,
    env: Option<&ProviderEnv>,
) -> Option<String> {
    region
        .map(str::to_string)
        .or_else(|| get_provider_env_value("AWS_REGION", env))
        .or_else(|| get_provider_env_value("AWS_DEFAULT_REGION", env))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn get_standard_bedrock_endpoint_region(base_url: Option<&str>) -> Option<String> {
    let base_url = base_url?;
    let hostname = parse_hostname(base_url)?;
    let lower = hostname.to_lowercase();
    let rest = lower
        .strip_prefix("bedrock-runtime")?
        .strip_suffix(".amazonaws.com")
        .or_else(|| {
            lower
                .strip_prefix("bedrock-runtime")?
                .strip_suffix(".amazonaws.com.cn")
        })?;
    // The optional -fips suffix precedes the region.
    let region = rest.strip_prefix("-fips").unwrap_or(rest);
    let region = region.strip_prefix('.').unwrap_or(region);
    (!region.is_empty())
        .then(|| region.to_string())
        .filter(|region| {
            region
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn should_use_explicit_bedrock_endpoint(
    base_url: &str,
    configured_region: Option<&str>,
    has_ambient_configured_profile: bool,
) -> bool {
    let endpoint_region = get_standard_bedrock_endpoint_region(Some(base_url));
    if endpoint_region.is_none() {
        return true;
    }
    configured_region.is_none() && !has_ambient_configured_profile
}

pub(crate) fn is_gov_cloud_bedrock_target(model: &Model, options: &BedrockOptions) -> bool {
    let region =
        get_configured_bedrock_region(options.region.as_deref(), options.base.base.env.as_ref());
    if region
        .as_deref()
        .is_some_and(|region| region.to_lowercase().starts_with("us-gov-"))
    {
        return true;
    }
    let model_id = model.id.to_lowercase();
    model_id.starts_with("us-gov.") || model_id.starts_with("arn:aws-us-gov:")
}

/// Port of the request-body assembly (`commandInput`).
pub(crate) fn build_command_input(
    model: &Model,
    context: &Context,
    options: &BedrockOptions,
) -> Value {
    let cache_retention =
        resolve_cache_retention(options.base.cache_retention, options.base.base.env.as_ref());
    let inference_max_tokens = options.base.max_tokens.or({
        if is_anthropic_claude_model(model) {
            Some(model.max_tokens)
        } else {
            None
        }
    });
    let mut inference_config = serde_json::Map::new();
    if let Some(max_tokens) = inference_max_tokens {
        inference_config.insert("maxTokens".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = options.base.temperature {
        inference_config.insert("temperature".to_string(), json!(temperature));
    }
    let mut input = serde_json::Map::new();
    input.insert("modelId".to_string(), json!(model.id));
    input.insert(
        "messages".to_string(),
        json!(convert_messages(
            context,
            model,
            cache_retention,
            options.base.base.env.as_ref()
        )),
    );
    if let Some(system) = build_system_prompt(
        context.system_prompt.as_deref(),
        model,
        cache_retention,
        options.base.base.env.as_ref(),
    ) {
        input.insert("system".to_string(), json!(system));
    }
    input.insert(
        "inferenceConfig".to_string(),
        Value::Object(inference_config),
    );
    if let Some(tool_config) = convert_tool_config(
        context.tools.as_ref(),
        options.tool_choice.as_ref(),
        model
            .compat
            .as_ref()
            .and_then(|compat| compat.supports_strict_mode)
            .unwrap_or(false),
    ) {
        input.insert("toolConfig".to_string(), tool_config);
    }
    if let Some(fields) = build_additional_model_request_fields(model, options) {
        input.insert("additionalModelRequestFields".to_string(), fields);
    }
    if let Some(metadata) = &options.request_metadata {
        input.insert(
            "requestMetadata".to_string(),
            json!(metadata.iter().collect::<BTreeMap<_, _>>()),
        );
    }
    Value::Object(input)
}

// ---------------------------------------------------------------------------
// Event handling
// ---------------------------------------------------------------------------

/// Port of `mapStopReason`.
fn map_stop_reason(reason: Option<&str>) -> (crate::ai::types::StopReason, Option<String>) {
    match reason {
        Some("end_turn") | Some("stop_sequence") => (crate::ai::types::StopReason::Stop, None),
        Some("max_tokens") | Some("model_context_window_exceeded") => {
            (crate::ai::types::StopReason::Length, None)
        }
        Some("tool_use") => (crate::ai::types::StopReason::ToolUse, None),
        Some(other) => (
            crate::ai::types::StopReason::Error,
            Some(format!("Provider stopped with: {other}")),
        ),
        None => (crate::ai::types::StopReason::Error, None),
    }
}

/// Human-readable prefixes for the Bedrock SDK exception names.
fn bedrock_error_prefix(name: &str) -> &str {
    match name {
        "InternalServerException" => "Internal server error",
        "ModelStreamErrorException" => "Model stream error",
        "ValidationException" => "Validation error",
        "ThrottlingException" => "Throttling error",
        "ServiceUnavailableException" => "Service unavailable",
        other => other,
    }
}

/// Port of `formatBedrockError`.
pub(crate) fn format_bedrock_error(error: &BedrockError) -> String {
    let norm = normalize_provider_error(ProviderErrorParts {
        status: error.status,
        body: error.body.clone(),
        message: error.message.clone(),
    });
    let core = match (&norm.status, &norm.body) {
        (Some(status), Some(body)) if !norm.message_carries_body => format!("{status}: {body}"),
        _ => norm.message.clone(),
    };
    let data_retention_hint = if core.to_lowercase().contains("data retention mode") {
        " See https://docs.aws.amazon.com/bedrock/latest/userguide/data-retention.html for supported data retention modes.".to_string()
    } else {
        String::new()
    };
    if error.service_exception {
        format!(
            "{}: {}{}",
            bedrock_error_prefix(&error.name),
            core,
            data_retention_hint
        )
    } else {
        format!("{core}{data_retention_hint}")
    }
}

/// Over-long header values are dropped rather than truncated.
const MAX_BEDROCK_DIAGNOSTIC_VALUE_CHARS: usize = 200;

fn normalize_diagnostic_value(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_BEDROCK_DIAGNOSTIC_VALUE_CHARS {
        return None;
    }
    Some(trimmed.to_string())
}

/// Modeled Bedrock errors all end in `Exception`, unlike transport names.
fn extract_bedrock_error_code(error: &BedrockError) -> Option<String> {
    if !error.name.ends_with("Exception") {
        return None;
    }
    normalize_diagnostic_value(Some(&error.name))
}

/// Port of `appendBedrockFailureDiagnostic`.
fn append_bedrock_failure_diagnostic(
    output: &mut AssistantMessage,
    error: &BedrockError,
    fallback_request_id: Option<&str>,
) {
    let mut details = serde_json::Map::new();
    if let Some(status) = error.http_status_code {
        details.insert("status".to_string(), json!(status));
    }
    if let Some(error_code) = extract_bedrock_error_code(error) {
        details.insert("errorCode".to_string(), json!(error_code));
    }
    let request_id = normalize_diagnostic_value(error.request_id.as_deref())
        .or_else(|| normalize_diagnostic_value(fallback_request_id));
    if let Some(request_id) = request_id {
        details.insert("requestId".to_string(), json!(request_id));
    }
    if details.is_empty() {
        return;
    }
    append_assistant_message_diagnostic(
        output,
        AssistantMessageDiagnostic {
            kind: "bedrock_response_failure".to_string(),
            timestamp: now_millis(),
            error: None,
            details: Some(Value::Object(details)),
        },
    );
}

/// The spawned dispatch outcome: success carries the finished message (plus
/// response request id); failure carries the partial message, the error, and
/// the response request id for the failure diagnostic.
type DispatchOutcome =
    Result<(AssistantMessage, Option<String>), (AssistantMessage, BedrockError, Option<String>)>;

struct EventDriver<'a> {
    model: &'a Model,
    stream: AssistantMessageEventStream,
    output: AssistantMessage,
    blocks: Vec<StreamingBlock>,
}

impl<'a> EventDriver<'a> {
    fn partial(&self) -> AssistantMessage {
        let mut partial = self.output.clone();
        partial.content = self
            .blocks
            .iter()
            .map(|block| block.clone().into_content())
            .collect();
        partial
    }

    fn block_position(&self, index: u64) -> Option<usize> {
        self.blocks
            .iter()
            .position(|block| block.index() == Some(index))
    }

    /// Port of `handleContentBlockStart`.
    fn handle_content_block_start(&mut self, event: &Value) {
        let index = event["contentBlockIndex"].as_u64().unwrap_or_default();
        let start = &event["start"];
        if let Some(tool_use) = start.get("toolUse").filter(|value| !value.is_null()) {
            self.blocks.push(StreamingBlock::ToolCall {
                index: Some(index),
                id: tool_use["toolUseId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                name: tool_use["name"].as_str().unwrap_or_default().to_string(),
                arguments: serde_json::Map::new(),
                partial_json: String::new(),
            });
            let content_index = self.blocks.len() - 1;
            self.stream.push(AssistantMessageEvent::ToolcallStart {
                content_index,
                partial: self.partial(),
            });
        }
    }

    /// Port of `handleContentBlockDelta`.
    fn handle_content_block_delta(&mut self, event: &Value) {
        let content_block_index = event["contentBlockIndex"].as_u64().unwrap_or_default();
        let delta = &event["delta"];

        if let Some(text) = delta.get("text").and_then(Value::as_str) {
            let position = self.block_position(content_block_index);
            let position = match position {
                Some(position) => position,
                None => {
                    // Text blocks get no contentBlockStart event.
                    self.blocks.push(StreamingBlock::Text {
                        index: Some(content_block_index),
                        text: String::new(),
                    });
                    let content_index = self.blocks.len() - 1;
                    self.stream.push(AssistantMessageEvent::TextStart {
                        content_index,
                        partial: self.partial(),
                    });
                    content_index
                }
            };
            if let StreamingBlock::Text {
                text: block_text, ..
            } = &mut self.blocks[position]
            {
                block_text.push_str(text);
            }
            self.stream.push(AssistantMessageEvent::TextDelta {
                content_index: position,
                delta: text.to_string(),
                partial: self.partial(),
            });
            return;
        }

        if let Some(tool_use) = delta.get("toolUse").filter(|value| !value.is_null()) {
            let input = tool_use["input"].as_str().unwrap_or_default().to_string();
            let mut pushed = None;
            if let Some(position) = self.block_position(content_block_index)
                && let StreamingBlock::ToolCall {
                    partial_json,
                    arguments,
                    ..
                } = &mut self.blocks[position]
            {
                partial_json.push_str(&input);
                *arguments = parse_streaming_json(Some(partial_json))
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                pushed = Some(position);
            }
            if let Some(position) = pushed {
                self.stream.push(AssistantMessageEvent::ToolcallDelta {
                    content_index: position,
                    delta: input,
                    partial: self.partial(),
                });
            }
            return;
        }

        if let Some(reasoning) = delta
            .get("reasoningContent")
            .filter(|value| !value.is_null())
        {
            let position = self.block_position(content_block_index);
            let position = match position {
                Some(position) => position,
                None => {
                    self.blocks.push(StreamingBlock::Thinking {
                        index: Some(content_block_index),
                        thinking: String::new(),
                        thinking_signature: String::new(),
                        redacted: false,
                        redacted_chunks: Vec::new(),
                    });
                    let content_index = self.blocks.len() - 1;
                    self.stream.push(AssistantMessageEvent::ThinkingStart {
                        content_index,
                        partial: self.partial(),
                    });
                    content_index
                }
            };
            // Collect the events first; the partial snapshots must include the
            // just-applied deltas, so the pushes happen after the mutation.
            let mut deltas: Vec<(usize, String)> = Vec::new();
            let StreamingBlock::Thinking {
                thinking,
                thinking_signature,
                redacted,
                redacted_chunks,
                ..
            } = &mut self.blocks[position]
            else {
                return;
            };
            if let Some(text) = reasoning.get("text").and_then(Value::as_str) {
                thinking.push_str(text);
                deltas.push((position, text.to_string()));
            }
            // `thinkingSignature` holds either an Anthropic signature or an
            // opaque redacted payload, never both.
            if let Some(signature) = reasoning.get("signature").and_then(Value::as_str)
                && !*redacted
            {
                thinking_signature.push_str(signature);
            }
            if let Some(redacted_content) = reasoning.get("redactedContent") {
                let bytes = redacted_content_bytes(redacted_content);
                if !bytes.is_empty() {
                    if !*redacted {
                        *redacted = true;
                        thinking_signature.clear();
                        thinking.push_str(REDACTED_THINKING_PLACEHOLDER);
                        deltas.push((position, REDACTED_THINKING_PLACEHOLDER.to_string()));
                    }
                    redacted_chunks.push(bytes);
                }
            }
            for (content_index, delta) in deltas {
                self.stream.push(AssistantMessageEvent::ThinkingDelta {
                    content_index,
                    delta,
                    partial: self.partial(),
                });
            }
        }
    }

    /// Port of `handleContentBlockStop`.
    fn handle_content_block_stop(&mut self, event: &Value) {
        let index = event["contentBlockIndex"].as_u64().unwrap_or_default();
        let Some(position) = self.block_position(index) else {
            return;
        };
        match &mut self.blocks[position] {
            StreamingBlock::Text { index, text } => {
                *index = None;
                let content = text.clone();
                self.stream.push(AssistantMessageEvent::TextEnd {
                    content_index: position,
                    content,
                    partial: self.partial(),
                });
            }
            StreamingBlock::Thinking {
                index, thinking, ..
            } => {
                *index = None;
                let content = thinking.clone();
                self.stream.push(AssistantMessageEvent::ThinkingEnd {
                    content_index: position,
                    content,
                    partial: self.partial(),
                });
            }
            StreamingBlock::ToolCall {
                index,
                arguments,
                partial_json,
                ..
            } => {
                *index = None;
                *arguments = parse_streaming_json(Some(partial_json))
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                let tool_call = match self.blocks[position].clone().into_content() {
                    AssistantContent::ToolCall(call) => call,
                    _ => unreachable!(),
                };
                self.stream.push(AssistantMessageEvent::ToolcallEnd {
                    content_index: position,
                    tool_call,
                    partial: self.partial(),
                });
            }
        }
    }

    /// Port of `handleMetadata`.
    fn handle_metadata(&mut self, event: &Value) {
        let Some(usage) = event.get("usage").filter(|value| !value.is_null()) else {
            return;
        };
        let input = usage["inputTokens"].as_u64().unwrap_or_default();
        let output = usage["outputTokens"].as_u64().unwrap_or_default();
        self.output.usage.input = input;
        self.output.usage.output = output;
        self.output.usage.cache_read = usage["cacheReadInputTokens"].as_u64().unwrap_or_default();
        self.output.usage.cache_write = usage["cacheWriteInputTokens"].as_u64().unwrap_or_default();
        self.output.usage.total_tokens = usage["totalTokens"]
            .as_u64()
            .unwrap_or(self.output.usage.input + self.output.usage.output);
        self.output.usage.cost = calculate_cost(self.model, &mut self.output.usage);
    }
}

/// The AWS SDK hands `redactedContent` over as base64 in JSON; accept either
/// the base64 string form or a raw byte array.
fn redacted_content_bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::String(text) => base64_to_bytes(text).unwrap_or_default(),
        Value::Array(bytes) => bytes
            .iter()
            .filter_map(|byte| byte.as_u64().map(|byte| byte as u8))
            .collect(),
        _ => Vec::new(),
    }
}

/// The mid-stream exception members, in the TS dispatch order.
fn stream_item_error(item: &Value) -> Option<BedrockError> {
    for (member, name) in [
        ("internalServerException", "InternalServerException"),
        ("modelStreamErrorException", "ModelStreamErrorException"),
        ("validationException", "ValidationException"),
        ("throttlingException", "ThrottlingException"),
        ("serviceUnavailableException", "ServiceUnavailableException"),
    ] {
        if let Some(exception) = item.get(member).filter(|value| !value.is_null()) {
            return Some(BedrockError {
                name: name.to_string(),
                message: exception["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                http_status_code: exception["$metadata"]["httpStatusCode"]
                    .as_u64()
                    .map(|v| v as u16),
                request_id: exception["$metadata"]["requestId"]
                    .as_str()
                    .map(str::to_string),
                service_exception: true,
                ..Default::default()
            });
        }
    }
    None
}

/// Port of the shared stream driver: runs the built request through
/// `onPayload`, dispatches, and drives the response items. `dispatch`
/// replaces the AWS SDK `client.send()` — it receives the (possibly
/// onPayload-replaced) command input.
fn drive_stream(
    model: &Model,
    context: &Context,
    options: &BedrockOptions,
    dispatch: impl FnOnce(
        Value,
    ) -> futures::future::BoxFuture<
        'static,
        Result<BedrockDispatchResponse, BedrockError>,
    > + Send
    + 'static,
) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let model = model.clone();
    let context = context.clone();
    let options = options.clone();
    let producer = stream.clone();
    tokio::spawn(async move {
        let outcome: DispatchOutcome = async {
            let output = AssistantMessage {
                api: "bedrock-converse-stream".to_string(),
                provider: model.provider.clone(),
                model: model.id.clone(),
                timestamp: now_millis(),
                ..Default::default()
            };
            let mut command_input = build_command_input(&model, &context, &options);
            if let Some(on_payload) = options.base.base.on_payload.clone()
                && let Some(replacement) = on_payload(command_input.clone(), &model).await
            {
                command_input = replacement;
            }
            let response = match dispatch(command_input).await {
                Ok(mut response) => {
                    if let Some(error) = response.send_error.take() {
                        return Err((output, error, None));
                    }
                    response
                }
                Err(error) => return Err((output, error, None)),
            };
            let response_request_id = normalize_diagnostic_value(response.request_id.as_deref());
            if let Some(on_response) = options.base.base.on_response.clone() {
                let mut headers = BTreeMap::new();
                if let Some(request_id) = &response.request_id {
                    headers.insert("x-amzn-requestid".to_string(), request_id.clone());
                }
                on_response(
                    &crate::ai::types::ProviderResponse {
                        status: response.http_status_code.unwrap_or(200),
                        headers,
                    },
                    &model,
                )
                .await;
            }

            let mut driver = EventDriver {
                model: &model,
                stream: producer.clone(),
                output,
                blocks: Vec::new(),
            };
            let result = match drive_items(&mut driver, &response.items) {
                Err(error) => Err(error),
                Ok(()) => match response.stream_error.clone() {
                    // The iterator throwing skips the post-loop checks.
                    Some(error) => Err(error),
                    None => post_loop_checks(&mut driver, &options),
                },
            };
            driver.output.content = driver
                .blocks
                .iter()
                .map(|block| block.clone().into_content())
                .collect();
            let output = driver.output;
            match result {
                Ok(()) => Ok((output, response_request_id)),
                Err(error) => Err((output, error, response_request_id)),
            }
        }
        .await;

        match outcome {
            Ok((output, _)) => {
                producer.push(AssistantMessageEvent::Done {
                    reason: done_reason(output.stop_reason),
                    message: output.clone(),
                });
                producer.end(Some(output));
            }
            Err((mut output, error, response_request_id)) => {
                let aborted = options
                    .base
                    .base
                    .signal
                    .as_ref()
                    .is_some_and(|signal| signal.is_cancelled());
                output.stop_reason = if aborted {
                    crate::ai::types::StopReason::Aborted
                } else {
                    crate::ai::types::StopReason::Error
                };
                output.error_message = Some(format_bedrock_error(&error));
                if output.stop_reason == crate::ai::types::StopReason::Error {
                    // The mid-stream request id must survive from the response
                    // metadata; the thrown error carries none of its own.
                    append_bedrock_failure_diagnostic(
                        &mut output,
                        &error,
                        response_request_id.as_deref(),
                    );
                }
                producer.push(AssistantMessageEvent::Error {
                    reason: if aborted {
                        crate::ai::types::ErrorReason::Aborted
                    } else {
                        crate::ai::types::ErrorReason::Error
                    },
                    error: output.clone(),
                });
                producer.end(Some(output));
            }
        }
    });
    stream
}

fn done_reason(stop_reason: crate::ai::types::StopReason) -> crate::ai::types::DoneReason {
    match stop_reason {
        crate::ai::types::StopReason::Length => crate::ai::types::DoneReason::Length,
        crate::ai::types::StopReason::ToolUse => crate::ai::types::DoneReason::ToolUse,
        _ => crate::ai::types::DoneReason::Stop,
    }
}

fn drive_items(driver: &mut EventDriver<'_>, items: &[Value]) -> Result<(), BedrockError> {
    for item in items {
        if item
            .get("messageStart")
            .is_some_and(|value| !value.is_null())
        {
            if item["messageStart"]["role"].as_str() != Some("assistant") {
                return Err(BedrockError::transport(
                    "Unexpected assistant message start but got user message start instead",
                ));
            }
            driver.stream.push(AssistantMessageEvent::Start {
                partial: driver.partial(),
            });
        } else if item
            .get("contentBlockStart")
            .is_some_and(|value| !value.is_null())
        {
            driver.handle_content_block_start(&item["contentBlockStart"]);
        } else if item
            .get("contentBlockDelta")
            .is_some_and(|value| !value.is_null())
        {
            driver.handle_content_block_delta(&item["contentBlockDelta"]);
        } else if item
            .get("contentBlockStop")
            .is_some_and(|value| !value.is_null())
        {
            driver.handle_content_block_stop(&item["contentBlockStop"]);
        } else if item
            .get("messageStop")
            .is_some_and(|value| !value.is_null())
        {
            let raw = item["messageStop"]["stopReason"].as_str();
            driver.output.raw_stop_reason = raw.map(str::to_string);
            let (stop_reason, error_message) = map_stop_reason(raw);
            driver.output.stop_reason = stop_reason;
            if let Some(error_message) = error_message {
                driver.output.error_message = Some(error_message);
            }
        } else if item.get("metadata").is_some_and(|value| !value.is_null()) {
            driver.handle_metadata(&item["metadata"]);
        } else if let Some(error) = stream_item_error(item) {
            return Err(error);
        }
    }

    Ok(())
}

/// The post-loop validations; a mid-stream iterator throw skips them, like
/// the TS `for await` exiting via the exception.
fn post_loop_checks(
    driver: &mut EventDriver<'_>,
    options: &BedrockOptions,
) -> Result<(), BedrockError> {
    let cancelled = options
        .base
        .base
        .signal
        .as_ref()
        .is_some_and(|signal| signal.is_cancelled());
    if cancelled {
        return Err(BedrockError::transport("Request was aborted"));
    }
    if driver.output.stop_reason == crate::ai::types::StopReason::Pending {
        return Err(BedrockError::transport(
            "Bedrock stream ended without a stop reason",
        ));
    }
    if driver.output.stop_reason == crate::ai::types::StopReason::Error
        || driver.output.stop_reason == crate::ai::types::StopReason::Aborted
    {
        return Err(BedrockError::transport(
            driver
                .output
                .error_message
                .clone()
                .unwrap_or_else(|| "An unknown error occurred".to_string()),
        ));
    }
    Ok(())
}

/// Port of the TS test seam: drive the pipeline from a mocked SDK dispatch
/// response. This is the surface the TypeScript bedrock tests exercise by
/// mocking `@aws-sdk/client-bedrock-runtime`.
pub fn stream_from_items(
    model: &Model,
    context: &Context,
    options: Option<&BedrockOptions>,
    response: BedrockDispatchResponse,
) -> AssistantMessageEventStream {
    let options = options.cloned().unwrap_or_default();
    drive_stream(model, context, &options, move |_command_input| {
        Box::pin(std::future::ready(Ok(response)))
    })
}

/// Port of `streamSimple`: adapts `SimpleStreamOptions` into
/// `BedrockOptions` (the thinking-budget adjustments) and dispatches.
pub fn stream_simple_from_items(
    model: &Model,
    context: &Context,
    options: Option<&SimpleStreamOptions>,
    response: BedrockDispatchResponse,
) -> AssistantMessageEventStream {
    let base = build_base_options(model, context, options, None);
    let mut bedrock = BedrockOptions {
        base,
        tool_choice: options.and_then(|options| options.tool_choice.clone()),
        ..Default::default()
    };
    let Some(reasoning) = options.and_then(|options| options.reasoning) else {
        return stream_from_items(model, context, Some(&bedrock), response);
    };
    bedrock.reasoning = Some(reasoning);
    bedrock.thinking_budgets = options.and_then(|options| options.thinking_budgets.clone());
    if is_anthropic_claude_model(model) && !supports_adaptive_thinking(&model.id, Some(&model.name))
    {
        // Budget-based Claude: reserve thinking headroom from the output cap.
        let adjusted = adjust_max_tokens_for_thinking(
            bedrock.base.max_tokens,
            model.max_tokens,
            reasoning,
            bedrock.thinking_budgets.as_ref(),
        );
        let max_tokens = clamp_max_tokens_to_context(model, context, adjusted.0);
        bedrock.base.max_tokens = Some(max_tokens);
        let level = clamp_reasoning(Some(reasoning)).unwrap_or(ThinkingLevel::High);
        let budget = adjusted.1.min(max_tokens.saturating_sub(1024));
        let mut budgets = bedrock.thinking_budgets.clone().unwrap_or_default();
        set_budget(&mut budgets, level, budget);
        bedrock.thinking_budgets = Some(budgets);
    }
    stream_from_items(model, context, Some(&bedrock), response)
}

fn set_budget(budgets: &mut ThinkingBudgets, level: ThinkingLevel, budget: u64) {
    match level {
        ThinkingLevel::Minimal => budgets.minimal = Some(budget),
        ThinkingLevel::Low => budgets.low = Some(budget),
        ThinkingLevel::Medium => budgets.medium = Some(budget),
        ThinkingLevel::High | ThinkingLevel::Xhigh | ThinkingLevel::Max => {
            budgets.high = Some(budget)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_standard_endpoint_regions() {
        assert_eq!(
            get_standard_bedrock_endpoint_region(Some(
                "https://bedrock-runtime.us-east-1.amazonaws.com"
            )),
            Some("us-east-1".to_string())
        );
        assert_eq!(
            get_standard_bedrock_endpoint_region(Some(
                "https://bedrock-runtime-fips.us-gov-west-1.amazonaws.com"
            )),
            Some("us-gov-west-1".to_string())
        );
        assert_eq!(
            get_standard_bedrock_endpoint_region(Some(
                "https://bedrock-runtime.cn-north-1.amazonaws.com.cn"
            )),
            Some("cn-north-1".to_string())
        );
        assert_eq!(
            get_standard_bedrock_endpoint_region(Some("https://gateway.example/v1")),
            None
        );
        assert_eq!(get_standard_bedrock_endpoint_region(None), None);
    }

    #[test]
    fn pins_explicit_endpoints_only_without_region_or_profile() {
        assert!(should_use_explicit_bedrock_endpoint(
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            None,
            false
        ));
        assert!(!should_use_explicit_bedrock_endpoint(
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            Some("eu-west-1"),
            false
        ));
        assert!(!should_use_explicit_bedrock_endpoint(
            "https://bedrock-runtime.us-east-1.amazonaws.com",
            None,
            true
        ));
        // Custom endpoints are always pinned.
        assert!(should_use_explicit_bedrock_endpoint(
            "https://gateway.example",
            Some("us-east-1"),
            true
        ));
    }
}

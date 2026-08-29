//! Port of `pi-core/ai/src/api/openai-codex-responses.ts` (SSE transport).
//!
//! The TypeScript adapter prefers a WebSocket transport with SSE fallback;
//! the Rust port implements the SSE path (the transport-fallback protocol
//! behavior is preserved: the SSE request shape, URL resolution, retry
//! policy, and the Codex event mapping are ported 1:1). WebSocket transport
//! selection is recorded as a documented limitation in MIGRATION.md.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::ai::api::constrained_sampling::create_grammar_tool_input_properties;
use crate::ai::api::openai_completions::clamp_openai_prompt_cache_key;
use crate::ai::api::openai_responses_shared::{
    ConvertResponsesMessagesOptions, ResponsesStreamOptions, convert_responses_messages,
    convert_responses_tools, process_responses_stream,
};
use crate::ai::api::simple_options::build_base_options;
use crate::ai::models::clamp_thinking_level;
use crate::ai::types::{
    CacheRetention, Context, Model, ProviderHeaders, SimpleStreamOptions, StreamOptions,
    ThinkingLevel, Usage,
};
use crate::ai::utils::deferred_tools::split_deferred_tools;
use crate::ai::utils::error_body::{
    ProviderErrorParts, format_provider_error, normalize_provider_error,
};
use crate::ai::utils::event_stream::{
    AssistantMessageEventStream, create_assistant_message_event_stream,
};
use crate::ai::utils::http::{HttpBody, HttpMethod, HttpRequest};
use crate::ai::utils::provider_retry::retry_provider_request;
use crate::ai::utils::reqwest_fetch::default_fetch;

const DEFAULT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";
const DEFAULT_MAX_RETRIES: u32 = 0;
const BASE_DELAY_MS: u64 = 1000;
const DEFAULT_MAX_RETRY_DELAY_MS: u64 = 60_000;

fn codex_tool_call_providers() -> BTreeSet<String> {
    ["openai", "openai-codex", "opencode"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Port of `OpenAICodexResponsesOptions`.
#[derive(Clone, Default)]
pub struct OpenAICodexResponsesOptions {
    pub base: StreamOptions,
    /// "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max".
    pub reasoning_effort: Option<String>,
    /// "auto" | "concise" | "detailed" | "off" | "on".
    pub reasoning_summary: Option<String>,
    pub service_tier: Option<String>,
    /// "low" | "medium" | "high".
    pub text_verbosity: Option<String>,
    /// "auto" | "none" | "required".
    pub tool_choice: Option<String>,
}

fn has_header(headers: Option<&ProviderHeaders>, name: &str) -> bool {
    let Some(headers) = headers else {
        return false;
    };
    let expected = name.to_lowercase();
    headers.iter().any(|(key, value)| {
        key.to_lowercase() == expected
            && value.as_ref().is_some_and(|value| !value.trim().is_empty())
    })
}

/// Port of `getApiKeyValue` equivalent: explicit key or header-owned auth.
fn get_client_api_key(
    provider: &str,
    api_key: Option<&str>,
    headers: Option<&ProviderHeaders>,
) -> Result<String, String> {
    if let Some(api_key) = api_key {
        return Ok(api_key.to_string());
    }
    if has_header(headers, "authorization") || has_header(headers, "cf-aig-authorization") {
        return Ok("unused".to_string());
    }
    Err(format!("No API key for provider: {provider}"))
}

/// Port of `extractAccountId`: decodes the JWT payload claim
/// `https://api.openai.com/auth.chatgpt_account_id`.
fn extract_account_id(auth_token: &str) -> Option<String> {
    let token = auth_token.strip_prefix("Bearer ").unwrap_or(auth_token);
    let mut parts = token.split('.');
    let (_header, payload) = (parts.next()?, parts.next()?);
    let decoded = decode_base64_url(payload)?;
    let parsed: Value = serde_json::from_str(&decoded).ok()?;
    parsed
        .pointer(&format!(
            "/{}/chatgpt_account_id",
            "https://api.openai.com/auth"
        ))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Minimal URL-safe base64 decoder (no external dependency).
fn decode_base64_url(input: &str) -> Option<String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut values = Vec::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '-' => values.push(62),
            '_' => values.push(63),
            '=' => break,
            other => {
                let index = ALPHABET
                    .iter()
                    .position(|candidate| *candidate as char == other)?;
                values.push(index as u32);
            }
        }
    }
    let mut bytes = Vec::with_capacity(values.len() * 3 / 4);
    for chunk in values.chunks(4) {
        match chunk.len() {
            4 => {
                let n = (chunk[0] << 18) | (chunk[1] << 12) | (chunk[2] << 6) | chunk[3];
                bytes.push((n >> 16) as u8);
                bytes.push((n >> 8) as u8);
                bytes.push(n as u8);
            }
            3 => {
                let n = (chunk[0] << 18) | (chunk[1] << 12) | (chunk[2] << 6);
                bytes.push((n >> 16) as u8);
                bytes.push((n >> 8) as u8);
            }
            2 => {
                let n = (chunk[0] << 18) | (chunk[1] << 12);
                bytes.push((n >> 16) as u8);
            }
            _ => return None,
        }
    }
    String::from_utf8(bytes).ok()
}

/// Port of `resolveCodexUrl`.
pub fn resolve_codex_url(base_url: Option<&str>) -> String {
    let raw = base_url
        .filter(|url| !url.trim().is_empty())
        .unwrap_or(DEFAULT_CODEX_BASE_URL);
    let normalized = raw.trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_string()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

/// Port of `isTerminalRateLimitError`.
fn is_terminal_rate_limit_error(error_text: &str) -> bool {
    let patterns = [
        "GoUsageLimitError",
        "FreeUsageLimitError",
        "Monthly usage limit reached",
        "available balance",
        "insufficient_quota",
        "out of budget",
        "quota exceeded",
        "billing",
    ];
    let lowered = error_text.to_lowercase();
    patterns
        .iter()
        .any(|pattern| lowered.contains(&pattern.to_lowercase()))
}

/// Port of `isRetryableError`.
fn is_retryable_error(status: u16, error_text: &str) -> bool {
    if status == 429 && is_terminal_rate_limit_error(error_text) {
        return false;
    }
    if matches!(status, 429 | 500 | 502 | 503 | 504) {
        return true;
    }
    let lowered = error_text.to_lowercase();
    [
        "rate.?limit",
        "overloaded",
        "service.?unavailable",
        "upstream.?connect",
        "connection.?refused",
    ]
    .iter()
    .any(|pattern| regex_search(pattern, &lowered))
}

/// Minimal case-insensitive substring regex for the simple patterns used
/// here (`.` wildcards and `?` optional chars collapse to substring checks
/// over both spellings).
fn regex_search(pattern: &str, text: &str) -> bool {
    let literal = pattern.replace(".?", " ").replace('.', " ");
    let compact: String = literal.chars().filter(|ch| !ch.is_whitespace()).collect();
    let expanded = pattern.replace(".?", "").replace('.', "");
    text.contains(&compact) || text.contains(&expanded.to_lowercase())
}

/// Port of `getRetryAfterDelayMs`.
fn get_retry_after_delay_ms(headers: &[(String, String)], now_ms: i64) -> Option<f64> {
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    if let Some(retry_after_ms) = header("retry-after-ms")
        && let Ok(millis) = retry_after_ms.parse::<f64>()
        && millis.is_finite()
    {
        return Some(millis.max(0.0));
    }

    let retry_after = header("retry-after")?;
    if let Ok(seconds) = retry_after.parse::<f64>()
        && seconds.is_finite()
    {
        return Some((seconds * 1000.0).max(0.0));
    }
    if let Ok(date_ms) = httpdate::parse_http_date(&retry_after).map(|time| {
        time.duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or_default()
    }) {
        return Some(((date_ms - now_ms) as f64).max(0.0));
    }
    None
}

/// Port of `buildRequestBody`.
#[allow(clippy::too_many_lines)]
pub fn build_request_body(
    model: &Model,
    context: &Context,
    options: Option<&OpenAICodexResponsesOptions>,
    cache_session_id: Option<&str>,
    grammar_tool_input_properties: &BTreeMap<String, String>,
) -> Result<Value, String> {
    let options = options.cloned().unwrap_or_default();
    let supports_strict_mode = model
        .compat
        .as_ref()
        .and_then(|compat| compat.supports_strict_mode)
        .unwrap_or(true);
    let supports_openai_grammar_tools = model
        .compat
        .as_ref()
        .and_then(|compat| compat.supports_open_ai_grammar_tools)
        .unwrap_or(false);
    let deferred_tools_mode = if model
        .compat
        .as_ref()
        .and_then(|compat| compat.supports_additional_tools)
        .unwrap_or(false)
    {
        Some(crate::ai::api::openai_responses_shared::DeferredToolsMode::AdditionalTools)
    } else if model
        .compat
        .as_ref()
        .and_then(|compat| compat.supports_tool_search)
        .unwrap_or(false)
    {
        Some(crate::ai::api::openai_responses_shared::DeferredToolsMode::ToolSearch)
    } else {
        None
    };
    let tool_placement = split_deferred_tools(context, deferred_tools_mode.is_some(), |name| {
        name.to_string()
    });
    let messages = convert_responses_messages(
        model,
        context,
        &codex_tool_call_providers(),
        Some(&ConvertResponsesMessagesOptions {
            include_system_prompt: Some(false),
            grammar_tool_input_properties: Some(grammar_tool_input_properties),
            deferred_tools: Some(&tool_placement.deferred_btree()),
            deferred_tools_mode,
            tool_options: Some(
                crate::ai::api::openai_responses_shared::ConvertResponsesToolsOptions {
                    strict: Some(None),
                    supports_strict_mode: Some(supports_strict_mode),
                    supports_openai_grammar_tools: Some(supports_openai_grammar_tools),
                    defer_loading: None,
                },
            ),
        }),
    )?;

    let mut body = json!({
        "model": model.id,
        "store": false,
        "stream": true,
        "instructions": context.system_prompt.clone().unwrap_or_else(|| "You are a helpful assistant.".to_string()),
        "input": messages,
        "text": {"verbosity": options.text_verbosity.clone().unwrap_or_else(|| "low".to_string())},
        "include": ["reasoning.encrypted_content"],
        "prompt_cache_key": cache_session_id,
        "tool_choice": options.tool_choice.clone().unwrap_or_else(|| "auto".to_string()),
        "parallel_tool_calls": true,
    });

    if let Some(temperature) = options.base.temperature {
        body["temperature"] = json!(temperature);
    }

    if let Some(service_tier) = &options.service_tier {
        body["service_tier"] = json!(service_tier);
    }

    if !tool_placement.immediate.is_empty() {
        body["tools"] = Value::Array(convert_responses_tools(
            &tool_placement.immediate,
            Some(
                &crate::ai::api::openai_responses_shared::ConvertResponsesToolsOptions {
                    strict: Some(None),
                    supports_strict_mode: Some(supports_strict_mode),
                    supports_openai_grammar_tools: Some(supports_openai_grammar_tools),
                    defer_loading: None,
                },
            ),
        )?);
    }

    if let Some(reasoning_effort) = &options.reasoning_effort {
        let key = match reasoning_effort.as_str() {
            "minimal" => Some(crate::ai::types::ModelThinkingLevel::Minimal),
            "low" => Some(crate::ai::types::ModelThinkingLevel::Low),
            "medium" => Some(crate::ai::types::ModelThinkingLevel::Medium),
            "high" => Some(crate::ai::types::ModelThinkingLevel::High),
            "xhigh" => Some(crate::ai::types::ModelThinkingLevel::Xhigh),
            "max" => Some(crate::ai::types::ModelThinkingLevel::Max),
            _ => None,
        };
        let effort = if reasoning_effort == "none" {
            model
                .thinking_level_map
                .as_ref()
                .and_then(|map| map.get(&crate::ai::types::ModelThinkingLevel::Off))
                .cloned()
                .flatten()
                .unwrap_or_else(|| "none".to_string())
        } else {
            key.and_then(|key| {
                model
                    .thinking_level_map
                    .as_ref()
                    .and_then(|map| map.get(&key))
                    .cloned()
                    .flatten()
            })
            .unwrap_or_else(|| reasoning_effort.clone())
        };
        body["reasoning"] = json!({
            "effort": effort,
            "summary": options.reasoning_summary.clone().unwrap_or_else(|| "auto".to_string()),
        });
    }

    Ok(body)
}

/// Port of `resolveCodexServiceTier`.
fn resolve_codex_service_tier(
    response_service_tier: Option<&str>,
    request_service_tier: Option<&str>,
) -> Option<String> {
    if response_service_tier == Some("default")
        && matches!(request_service_tier, Some("flex") | Some("priority"))
    {
        return request_service_tier.map(str::to_string);
    }
    response_service_tier
        .map(str::to_string)
        .or_else(|| request_service_tier.map(str::to_string))
}

fn get_service_tier_cost_multiplier(model: &Model, service_tier: Option<&str>) -> f64 {
    match service_tier {
        Some("flex") => 0.5,
        Some("priority") => {
            if model.id == "gpt-5.5" {
                2.5
            } else {
                2.0
            }
        }
        _ => 1.0,
    }
}

fn apply_service_tier_pricing(usage: &mut Usage, service_tier: Option<&str>, model: &Model) {
    let multiplier = get_service_tier_cost_multiplier(model, service_tier);
    if multiplier == 1.0 {
        return;
    }

    usage.cost.input = (f64::from(usage.cost.input) * multiplier).into();
    usage.cost.output = (f64::from(usage.cost.output) * multiplier).into();
    usage.cost.cache_read = (f64::from(usage.cost.cache_read) * multiplier).into();
    usage.cost.cache_write = (f64::from(usage.cost.cache_write) * multiplier).into();
    usage.cost.total = (f64::from(usage.cost.input)
        + f64::from(usage.cost.output)
        + f64::from(usage.cost.cache_read)
        + f64::from(usage.cost.cache_write))
    .into();
}

/// Port of `mapCodexEvents`: normalizes Codex event frames to the shared
/// Responses event stream (end_turn capture, status normalization, error
/// extraction).
fn map_codex_events(
    events: Vec<Value>,
    output: &mut crate::ai::types::AssistantMessage,
) -> Vec<Value> {
    let mut mapped = Vec::with_capacity(events.len());
    for event in events {
        let Some(event_type) = event
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };

        if event_type == "error" {
            let code = event
                .get("code")
                .and_then(Value::as_str)
                .map(str::to_string);
            let message = event
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string);
            let detail = message
                .clone()
                .or_else(|| code.clone())
                .unwrap_or_else(|| serde_json::to_string(&event).unwrap_or_default());
            // Return the error through a synthetic error event so the shared
            // processor surfaces it with the Codex prefix.
            mapped.push(json!({
                "type": "error",
                "code": code.unwrap_or_default(),
                "message": format!("Codex error: {detail}"),
            }));
            return mapped;
        }

        if event_type == "response.failed" {
            let code = event
                .pointer("/response/error/code")
                .and_then(Value::as_str)
                .map(str::to_string);
            let message = event
                .pointer("/response/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| "Codex response failed".to_string());
            mapped.push(json!({
                "type": "error",
                "code": code.unwrap_or_default(),
                "message": message,
            }));
            return mapped;
        }

        if matches!(
            event_type.as_str(),
            "response.done" | "response.completed" | "response.incomplete"
        ) {
            let mut event = event;
            if let Some(response) = event.get_mut("response").and_then(Value::as_object_mut) {
                if let Some(end_turn) = response.get("end_turn")
                    && end_turn.is_boolean()
                {
                    output.end_turn = Some(end_turn.as_bool().unwrap_or(false));
                }
                // Normalize unknown statuses to undefined.
                if let Some(status) = response.get("status")
                    && !matches!(
                        status.as_str(),
                        Some("completed")
                            | Some("incomplete")
                            | Some("failed")
                            | Some("cancelled")
                            | Some("queued")
                            | Some("in_progress")
                    )
                {
                    response.remove("status");
                }
            }
            let mut normalized = event.clone();
            normalized["type"] = json!("response.completed");
            mapped.push(normalized);
            return mapped;
        }

        mapped.push(event);
    }
    mapped
}

/// Port of the `stream` stream function (SSE transport).
#[allow(clippy::too_many_lines)]
pub fn stream(
    model: &Model,
    context: &Context,
    options: Option<&OpenAICodexResponsesOptions>,
) -> AssistantMessageEventStream {
    let stream = create_assistant_message_event_stream();
    let model = model.clone();
    let context = context.clone();
    let options = options.cloned();
    let producer = stream.clone();
    tokio::spawn(async move {
        let mut output = crate::ai::types::AssistantMessage {
            role: crate::ai::types::RoleAssistant,
            content: Vec::new(),
            api: "openai-codex-responses".to_string(),
            provider: model.provider.clone(),
            model: model.id.clone(),
            usage: Default::default(),
            stop_reason: crate::ai::types::StopReason::Pending,
            timestamp: crate::ai::auth::resolve::now_millis(),
            ..Default::default()
        };

        let result = run_stream(&model, &context, options.as_ref(), &mut output, &producer).await;
        if let Err(error) = result {
            let aborted = options
                .as_ref()
                .and_then(|options| options.base.base.signal.as_ref())
                .is_some_and(|token| token.is_cancelled());
            output.stop_reason = if aborted {
                crate::ai::types::StopReason::Aborted
            } else {
                crate::ai::types::StopReason::Error
            };
            let formatted = format_provider_error(
                &normalize_provider_error(ProviderErrorParts {
                    status: None,
                    body: None,
                    message: error,
                }),
                None,
            );
            output.error_message = Some(formatted);
            producer.push(crate::ai::types::AssistantMessageEvent::Error {
                reason: if output.stop_reason == crate::ai::types::StopReason::Aborted {
                    crate::ai::types::ErrorReason::Aborted
                } else {
                    crate::ai::types::ErrorReason::Error
                },
                error: output.clone(),
            });
            producer.end(None);
        }
    });
    stream
}

async fn run_stream(
    model: &Model,
    context: &Context,
    options: Option<&OpenAICodexResponsesOptions>,
    output: &mut crate::ai::types::AssistantMessage,
    producer: &AssistantMessageEventStream,
) -> Result<(), String> {
    let api_key = get_client_api_key(
        &model.provider,
        options.and_then(|options| options.base.base.api_key.as_deref()),
        options.and_then(|options| options.base.base.headers.as_ref()),
    )?;

    let account_id = extract_account_id(&api_key);
    let grammar_tool_input_properties = create_grammar_tool_input_properties(
        context.tools.as_deref(),
        model
            .compat
            .as_ref()
            .and_then(|compat| compat.supports_open_ai_grammar_tools)
            .unwrap_or(false),
    )?;
    let cache_session_id =
        if options.and_then(|options| options.base.cache_retention) == Some(CacheRetention::None) {
            None
        } else {
            options.and_then(|options| options.base.session_id.clone())
        };
    let codex_session_id = clamp_openai_prompt_cache_key(cache_session_id.as_deref());
    let mut body = build_request_body(
        model,
        context,
        options,
        codex_session_id.as_deref(),
        &grammar_tool_input_properties,
    )?;
    if let Some(on_payload) = options.and_then(|options| options.base.base.on_payload.as_ref())
        && let Some(next_body) = on_payload(body.clone(), model).await
    {
        body = next_body;
    }

    let mut headers: Vec<(String, String)> = vec![
        ("accept".to_string(), "text/event-stream".to_string()),
        ("content-type".to_string(), "application/json".to_string()),
        ("authorization".to_string(), format!("Bearer {api_key}")),
    ];
    if let Some(account_id) = &account_id {
        headers.push(("chatgpt-account-id".to_string(), account_id.clone()));
    }
    if let Some(session_id) = &codex_session_id {
        headers.push(("session_id".to_string(), session_id.clone()));
        headers.push(("x-client-request-id".to_string(), session_id.clone()));
    }
    if let Some(model_headers) = &model.headers {
        for (name, value) in model_headers {
            headers.push((name.clone(), value.clone()));
        }
    }
    if let Some(options_headers) = options.and_then(|options| options.base.base.headers.as_ref()) {
        for (name, value) in options_headers {
            if let Some(value) = value {
                headers.push((name.clone(), value.clone()));
            }
        }
    }

    let fetch = options
        .and_then(|options| options.base.base.fetch.clone())
        .unwrap_or_else(default_fetch);
    let url = resolve_codex_url(Some(&model.base_url));
    let request = HttpRequest {
        method: HttpMethod::Post,
        url,
        headers,
        body: HttpBody::Json(body),
    };

    // Port of the SSE retry loop: bounded attempts, retryable statuses, and
    // Retry-After handling with the terminal-rate-limit exclusion.
    let max_retries = options
        .and_then(|options| options.base.base.max_retries)
        .unwrap_or(DEFAULT_MAX_RETRIES);
    let now_ms = || crate::ai::auth::resolve::now_millis();
    let mut response = None;
    'attempts: for attempt in 0..=max_retries {
        let signal = options.and_then(|options| options.base.base.signal.clone());
        if signal.as_ref().is_some_and(|token| token.is_cancelled()) {
            return Err("Request was aborted".to_string());
        }

        let current = retry_provider_request(
            || {
                let fetch = Arc::clone(&fetch);
                let request = request.clone();
                async move {
                    fetch.fetch(request).await.map_err(|error| {
                        crate::ai::utils::provider_retry::ProviderHttpError::new(
                            error.to_string(),
                            None,
                            Vec::new(),
                        )
                    })
                }
            },
            crate::ai::utils::provider_retry::ProviderRetryOptions {
                max_retries: Some(0),
                max_retry_delay_ms: options
                    .and_then(|options| options.base.base.max_retry_delay_ms),
                signal: signal.clone(),
            },
        )
        .await;

        match current {
            Ok(current) => {
                let error_headers = current.headers.clone();
                let response_headers: BTreeMap<String, String> = current
                    .headers
                    .iter()
                    .map(|(name, value)| (name.to_lowercase(), value.clone()))
                    .collect();
                if let Some(on_response) =
                    options.and_then(|options| options.base.base.on_response.as_ref())
                {
                    on_response(
                        &crate::ai::types::ProviderResponse {
                            status: current.status,
                            headers: response_headers,
                        },
                        model,
                    )
                    .await;
                }

                if (200..300).contains(&current.status) {
                    response = Some(current);
                    break 'attempts;
                }

                let error_status = current.status;
                let error_text = crate::ai::utils::http::collect_text(current).await;
                if attempt < max_retries && is_retryable_error(error_status, &error_text) {
                    let retry_after_delay_ms = get_retry_after_delay_ms(&error_headers, now_ms());
                    let delay_ms = match retry_after_delay_ms {
                        Some(delay) => {
                            let max_delay = options
                                .and_then(|options| options.base.base.max_retry_delay_ms)
                                .unwrap_or(DEFAULT_MAX_RETRY_DELAY_MS);
                            if max_delay > 0 && delay > max_delay as f64 {
                                return Err(format!(
                                    "Server requested {}s retry delay (max: {}s)",
                                    (delay / 1000.0).ceil(),
                                    (max_delay as f64 / 1000.0).ceil(),
                                ));
                            }
                            delay as u64
                        }
                        None => BASE_DELAY_MS * 2u64.pow(attempt),
                    };
                    abortable_sleep(delay_ms, signal.as_ref()).await?;
                    continue;
                }

                return Err(format_provider_error(
                    &normalize_provider_error(ProviderErrorParts {
                        status: Some(error_status),
                        body: Some(error_text),
                        message: format!("{} status code", error_status),
                    }),
                    None,
                ));
            }
            Err(error) => {
                if error.message == "Request was aborted" {
                    return Err("Request was aborted".to_string());
                }
                if attempt < max_retries && !error.message.contains("usage limit") {
                    abortable_sleep(BASE_DELAY_MS * 2u64.pow(attempt), signal.as_ref()).await?;
                    continue;
                }
                return Err(error.message);
            }
        }
    }

    let response = response.ok_or_else(|| "Failed after retries".to_string())?;
    producer.push(crate::ai::types::AssistantMessageEvent::Start {
        partial: output.clone(),
    });

    // Parse SSE data frames, map Codex events, then run the shared processor.
    let mut sse = crate::ai::utils::sse::SseStream::new(response.body);
    let mut raw_events: Vec<Value> = Vec::new();
    while let Some(sse_event) = futures::StreamExt::next(&mut sse).await {
        if sse_event.event.as_deref() == Some("__error__") {
            return Err(sse_event.data);
        }
        let data = sse_event
            .data
            .lines()
            .filter(|line| line.starts_with("data:"))
            .map(|line| line[5..].trim())
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<Value>(&data) {
            raw_events.push(event);
        }
    }
    let mapped = map_codex_events(raw_events, output);

    let model_owned = model.clone();
    let stream_options = ResponsesStreamOptions {
        service_tier: options.and_then(|options| options.service_tier.clone()),
        grammar_tool_input_properties: Some(grammar_tool_input_properties),
        resolve_service_tier: Some(Box::new(
            |response_tier: Option<&str>, request_tier: Option<&str>| {
                resolve_codex_service_tier(response_tier, request_tier)
            },
        )),
        apply_service_tier_pricing: Some(Box::new(
            move |usage: &mut Usage, service_tier: Option<&str>| {
                apply_service_tier_pricing(usage, service_tier, &model_owned);
            },
        )),
    };
    process_responses_stream(mapped, output, producer, model, Some(&stream_options)).await?;

    let signal = options.and_then(|options| options.base.base.signal.clone());
    if signal.as_ref().is_some_and(|token| token.is_cancelled()) {
        return Err("Request was aborted".to_string());
    }

    if output.stop_reason == crate::ai::types::StopReason::Pending {
        return Err("Codex stream ended without a stop reason".to_string());
    }
    if output.stop_reason == crate::ai::types::StopReason::Error
        || output.stop_reason == crate::ai::types::StopReason::Aborted
    {
        return Err(output
            .error_message
            .clone()
            .unwrap_or_else(|| "An unknown error occurred".to_string()));
    }

    let reason = match output.stop_reason {
        crate::ai::types::StopReason::Length => crate::ai::types::DoneReason::Length,
        crate::ai::types::StopReason::ToolUse => crate::ai::types::DoneReason::ToolUse,
        crate::ai::types::StopReason::Deferred => crate::ai::types::DoneReason::Deferred,
        _ => crate::ai::types::DoneReason::Stop,
    };
    producer.push(crate::ai::types::AssistantMessageEvent::Done {
        reason,
        message: output.clone(),
    });
    producer.end(None);
    Ok(())
}

async fn abortable_sleep(
    ms: u64,
    signal: Option<&tokio_util::sync::CancellationToken>,
) -> Result<(), String> {
    match signal {
        Some(signal) => {
            if signal.is_cancelled() {
                return Err("Request was aborted".to_string());
            }
            tokio::select! {
                () = tokio::time::sleep(std::time::Duration::from_millis(ms)) => Ok(()),
                _ = signal.cancelled() => Err("Request was aborted".to_string()),
            }
        }
        None => {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            Ok(())
        }
    }
}

/// Port of `streamSimple`.
pub fn stream_simple(
    model: &Model,
    context: &Context,
    options: Option<&SimpleStreamOptions>,
) -> AssistantMessageEventStream {
    if get_client_api_key(
        &model.provider,
        options.and_then(|options| options.base.base.api_key.as_deref()),
        options.and_then(|options| options.base.base.headers.as_ref()),
    )
    .is_err()
    {
        return crate::ai::api::openai_completions::error_stream(
            model,
            &format!("No API key for provider: {}", model.provider),
        );
    }

    let options = options.cloned().unwrap_or_default();
    let base = build_base_options(
        model,
        context,
        Some(&options),
        options.base.base.api_key.as_deref(),
    );
    let clamped_reasoning = options.reasoning.map(|reasoning| {
        clamp_thinking_level(model, crate::ai::types::ModelThinkingLevel::from(reasoning))
    });
    let reasoning_effort = clamped_reasoning
        .and_then(|level| level.as_thinking_level())
        .map(|level| {
            match level {
                ThinkingLevel::Minimal => "minimal",
                ThinkingLevel::Low => "low",
                ThinkingLevel::Medium => "medium",
                ThinkingLevel::High => "high",
                ThinkingLevel::Xhigh => "xhigh",
                ThinkingLevel::Max => "max",
            }
            .to_string()
        });

    stream(
        model,
        context,
        Some(&OpenAICodexResponsesOptions {
            base,
            reasoning_effort,
            ..Default::default()
        }),
    )
}

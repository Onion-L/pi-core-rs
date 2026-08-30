//! Port of `pi-core/ai/test/openai-codex-stream.test.ts` (SSE transport) and
//! the Codex payload case from `pi-core/ai/test/max-thinking.test.ts`.
//!
//! The TypeScript suite mocks `globalThis.fetch` (and `WebSocket` for the
//! websocket transport); the Rust port drives the same observable surface
//! through a mock `HttpFetch` transport. The `transport: "sse"` option of the
//! TS calls is implicit: the Rust adapter implements the SSE path only.
//!
//! Not ported (documented deviation: the Rust port implements the SSE
//! transport only, so the websocket code paths have no Rust counterpart):
//!
//! - "forwards auto transport from streamSimple options and uses cached
//!   websocket context"
//! - "scopes cached websockets to the authenticated account"
//! - "closes one-shot websockets when cacheRetention is none"
//! - "falls back to SSE when websocket connect does not open before the
//!   connect timeout"
//! - "reconnects once when the websocket connection limit is reached before
//!   output starts"
//! - "falls back to SSE when a websocket is idle before the first event"
//! - "errors when a websocket is idle after the stream started"
//! - "opens a fresh cached websocket before the backend connection age limit"
//! - "sends only response input deltas in websocket-cached mode"
//! - "recovers a missing cached websocket continuation via websocket"
//! - "recovers a missing cached websocket continuation via sse"
//! - "zstd-compresses SSE request bodies" (the Rust SSE path sends the
//!   uncompressed JSON request body; zstd request compression is not ported)
//!
//! Harness differences (behavior-preserving):
//!
//! - The TS timeout test asserts the SSE fetch receives an abort signal; the
//!   Rust `HttpRequest` carries no signal, so only the observable timeout
//!   error is asserted.
//! - The TS max-thinking test throws from `onPayload` to skip the request;
//!   the Rust `onPayload` cannot fail, so the mock answers a completed SSE
//!   stream instead.
//! - The TS retry tests freeze `Date.now()` and spy `setTimeout`; the Rust
//!   port uses paused tokio time for the deterministic headers and a tight
//!   wall-clock window for the HTTP-date case.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;

use base64::Engine as _;
use bytes::Bytes;
use futures::StreamExt;
use futures::future::BoxFuture;
use pi_core::ai::api::openai_codex_responses::{
    OpenAICodexResponsesOptions, stream as stream_codex, stream_simple as stream_simple_codex,
};
use pi_core::ai::types::{
    AssistantMessage, AssistantMessageEvent, CacheRetention, ConstrainedSamplingConfig,
    ConstrainedSamplingStrict, Context, JsF64, Message, Model, ModelCost, ModelCostRates,
    ModelInput, ProviderRequestOptions, RoleUser, SimpleStreamOptions, StopReason, StreamOptions,
    ThinkingLevel, Tool, ToolConstrainedSampling, UserContent, UserMessage,
};
use pi_core::ai::utils::http::{HttpBody, HttpFetch, HttpFetchError, HttpRequest, HttpResponse};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Port of `mockToken`: a JWT-shaped token carrying the ChatGPT account id.
fn mock_token() -> String {
    let payload = json!({"https://api.openai.com/auth": {"chatgpt_account_id": "acc_test"}});
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_string(&payload).unwrap());
    format!("aaa.{encoded}.bbb")
}

fn codex_model(id: &str, name: &str) -> Model {
    Model {
        id: id.to_string(),
        name: name.to_string(),
        api: "openai-codex-responses".to_string(),
        provider: "openai-codex".to_string(),
        base_url: "https://chatgpt.com/backend-api".to_string(),
        reasoning: true,
        input: vec![ModelInput::Text],
        cost: ModelCost::default(),
        context_window: 400_000,
        max_tokens: 128_000,
        ..Default::default()
    }
}

fn user_message(text: &str) -> Message {
    Message::User(UserMessage {
        role: RoleUser,
        content: UserContent::Text(text.to_string()),
        timestamp: 1,
    })
}

fn context(prompt: &str, message: &str) -> Context {
    Context {
        system_prompt: Some(prompt.to_string()),
        messages: vec![user_message(message)],
        ..Default::default()
    }
}

fn sse_from_events(events: &[Value]) -> String {
    events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}

fn output_item_added() -> Value {
    json!({
        "type": "response.output_item.added",
        "item": { "type": "message", "id": "msg_1", "role": "assistant", "status": "in_progress", "content": [] },
    })
}

fn content_part_added() -> Value {
    json!({"type": "response.content_part.added", "part": {"type": "output_text", "text": ""}})
}

fn output_text_delta(delta: &str) -> Value {
    json!({"type": "response.output_text.delta", "delta": delta})
}

fn output_item_done(text: &str) -> Value {
    json!({
        "type": "response.output_item.done",
        "item": {
            "type": "message",
            "id": "msg_1",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": text}],
        },
    })
}

/// Port of `buildSSEPayload`: the standard five-event Codex fixture.
fn build_sse_payload(status: &str, include_done: bool, end_turn: Option<bool>) -> String {
    let terminal_type = if status == "incomplete" {
        "response.incomplete"
    } else {
        "response.completed"
    };
    let mut terminal = Map::new();
    terminal.insert("status".to_string(), json!(status));
    if let Some(end_turn) = end_turn {
        terminal.insert("end_turn".to_string(), json!(end_turn));
    }
    terminal.insert(
        "incomplete_details".to_string(),
        if status == "incomplete" {
            json!({"reason": "max_output_tokens"})
        } else {
            Value::Null
        },
    );
    terminal.insert(
        "usage".to_string(),
        json!({
            "input_tokens": 5,
            "output_tokens": 3,
            "total_tokens": 8,
            "input_tokens_details": {"cached_tokens": 0},
        }),
    );

    let mut events = vec![
        output_item_added(),
        content_part_added(),
        output_text_delta("Hello"),
        output_item_done("Hello"),
        json!({"type": terminal_type, "response": terminal}),
    ];
    if include_done {
        events.push(json!("[DONE]"));
    }
    sse_from_events(&events)
}

// ---------------------------------------------------------------------------
// Mock transport
// ---------------------------------------------------------------------------

enum ScriptedResponse {
    /// `200` SSE response whose full body arrives at once and closes.
    Sse(String),
    /// `200` SSE response whose body emits `chunk` once and stays open.
    SseOpen(String),
    /// `200` SSE response delivering chunks after delays; the flag is set
    /// when the body is dropped (the TS `ReadableStream.cancel()` callback).
    SseTimed(Vec<(u64, String)>, Option<Arc<AtomicBool>>),
    /// JSON error response with headers.
    Error(u16, String, Vec<(String, String)>),
    /// Response headers never arrive (the fetch promise never settles).
    Pending,
}

/// Mock transport standing in for the TS `vi.stubGlobal("fetch", ...)`.
struct CodexFetch {
    queue: Mutex<VecDeque<ScriptedResponse>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl CodexFetch {
    fn new(responses: Vec<ScriptedResponse>) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn request(&self) -> HttpRequest {
        self.requests
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("request captured")
    }
}

impl HttpFetch for CodexFetch {
    fn fetch<'a>(
        &'a self,
        request: HttpRequest,
    ) -> BoxFuture<'a, Result<HttpResponse, HttpFetchError>> {
        self.requests.lock().unwrap().push(request);
        let next = self
            .queue
            .lock()
            .unwrap()
            .pop_front()
            .expect("no scripted response left");
        Box::pin(async move {
            match next {
                ScriptedResponse::Sse(body) => Ok(sse_response(futures::stream::iter(vec![Ok(
                    Bytes::from(body),
                )]))),
                ScriptedResponse::SseOpen(chunk) => Ok(sse_response(
                    futures::stream::once(async move { Ok(Bytes::from(chunk)) })
                        .chain(futures::stream::pending()),
                )),
                ScriptedResponse::SseTimed(chunks, dropped) => {
                    let body = futures::stream::unfold(chunks.into_iter(), |mut rest| async move {
                        let (delay, chunk) = rest.next()?;
                        if delay > 0 {
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                        }
                        Some((Ok(Bytes::from(chunk)), rest))
                    });
                    Ok(sse_response(DropFlagStream {
                        inner: Box::pin(body),
                        dropped,
                    }))
                }
                ScriptedResponse::Error(status, body, headers) => Ok(HttpResponse {
                    status,
                    headers,
                    body: Box::pin(futures::stream::iter(vec![Ok(Bytes::from(body))])),
                }),
                ScriptedResponse::Pending => {
                    std::future::pending::<Result<HttpResponse, HttpFetchError>>().await
                }
            }
        })
    }
}

fn sse_response(
    body: impl futures::Stream<Item = Result<Bytes, HttpFetchError>> + Send + 'static,
) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: vec![("content-type".to_string(), "text/event-stream".to_string())],
        body: Box::pin(body),
    }
}

/// A body stream that reports cancellation through a flag on drop, mirroring
/// the TS `ReadableStream.cancel()` callback.
struct DropFlagStream {
    inner: Pin<Box<dyn futures::Stream<Item = Result<Bytes, HttpFetchError>> + Send>>,
    dropped: Option<Arc<AtomicBool>>,
}

impl futures::Stream for DropFlagStream {
    type Item = Result<Bytes, HttpFetchError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl Drop for DropFlagStream {
    fn drop(&mut self) {
        if let Some(dropped) = &self.dropped {
            dropped.store(true, Ordering::SeqCst);
        }
    }
}

fn base_options(fetch: Arc<CodexFetch>) -> StreamOptions {
    StreamOptions {
        base: ProviderRequestOptions {
            api_key: Some(mock_token()),
            fetch: Some(fetch),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn codex_options(fetch: Arc<CodexFetch>) -> OpenAICodexResponsesOptions {
    OpenAICodexResponsesOptions {
        base: base_options(fetch),
        ..Default::default()
    }
}

fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn body_of(request: &HttpRequest) -> Value {
    match &request.body {
        HttpBody::Json(body) => body.clone(),
        other => panic!("expected JSON body, got {other:?}"),
    }
}

fn text_of(message: &AssistantMessage) -> String {
    message
        .content
        .iter()
        .find_map(|block| match block {
            pi_core::ai::types::AssistantContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn event_type(event: &AssistantMessageEvent) -> &'static str {
    match event {
        AssistantMessageEvent::Start { .. } => "start",
        AssistantMessageEvent::TextStart { .. } => "text_start",
        AssistantMessageEvent::TextDelta { .. } => "text_delta",
        AssistantMessageEvent::TextEnd { .. } => "text_end",
        AssistantMessageEvent::ThinkingStart { .. } => "thinking_start",
        AssistantMessageEvent::ThinkingDelta { .. } => "thinking_delta",
        AssistantMessageEvent::ThinkingEnd { .. } => "thinking_end",
        AssistantMessageEvent::ToolcallStart { .. } => "toolcall_start",
        AssistantMessageEvent::ToolcallDelta { .. } => "toolcall_delta",
        AssistantMessageEvent::ToolcallEnd { .. } => "toolcall_end",
        AssistantMessageEvent::Done { .. } => "done",
        AssistantMessageEvent::Error { .. } => "error",
    }
}

// ---------------------------------------------------------------------------
// openai-codex-stream.test.ts (SSE transport)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streams_sse_responses_into_assistant_message_event_stream() {
    // The TS test's inline SSE fixture: no end_turn / incomplete_details.
    let sse = sse_from_events(&[
        output_item_added(),
        content_part_added(),
        output_text_delta("Hello"),
        output_item_done("Hello"),
        json!({
            "type": "response.completed",
            "response": {
                "status": "completed",
                "usage": {
                    "input_tokens": 5,
                    "output_tokens": 3,
                    "total_tokens": 8,
                    "input_tokens_details": {"cached_tokens": 0},
                },
            },
        }),
    ]);
    let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(sse)]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");

    let stream = stream_codex(&model, &context, Some(&codex_options(fetch.clone())));
    let mut saw_text_delta = false;
    let mut saw_done = false;
    while let Some(event) = stream.next().await {
        match &event {
            AssistantMessageEvent::TextDelta { .. } => saw_text_delta = true,
            AssistantMessageEvent::Done { message, .. } => {
                saw_done = true;
                assert_eq!(text_of(message), "Hello");
            }
            _ => {}
        }
    }

    assert!(saw_text_delta);
    assert!(saw_done);

    let request = fetch.request();
    assert_eq!(
        request.url,
        "https://chatgpt.com/backend-api/codex/responses"
    );
    let token = mock_token();
    assert_eq!(
        header(&request, "Authorization"),
        Some(format!("Bearer {token}").as_str())
    );
    assert_eq!(header(&request, "chatgpt-account-id"), Some("acc_test"));
    assert_eq!(
        header(&request, "OpenAI-Beta"),
        Some("responses=experimental")
    );
    assert_eq!(header(&request, "originator"), Some("pi"));
    // TS: `pi (${platform()} ${release()}; ${arch})` from node:os; the Rust
    // port builds the same shape from the Rust target.
    assert_eq!(
        header(&request, "User-Agent"),
        Some(pi_core::ai::session_resources::get_pi_user_agent().as_str())
    );
    assert_eq!(header(&request, "accept"), Some("text/event-stream"));
    assert!(header(&request, "x-api-key").is_none());
}

#[tokio::test]
async fn completes_after_response_completed_even_when_the_sse_body_stays_open() {
    let sse = build_sse_payload("completed", true, Some(false));
    let fetch = CodexFetch::new(vec![ScriptedResponse::SseOpen(sse)]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");

    // The TS test races the result against a 1s timeout.
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        stream_codex(&model, &context, Some(&codex_options(fetch))).result(),
    )
    .await
    .expect("Timed out waiting for completed SSE stream");

    assert_eq!(text_of(&result), "Hello");
    assert_eq!(result.stop_reason, StopReason::Stop);
    assert_eq!(result.end_turn, Some(false));
}

#[tokio::test]
async fn maps_response_incomplete_to_stop_reason_length_even_when_the_body_stays_open() {
    let sse = build_sse_payload("incomplete", false, None);
    let fetch = CodexFetch::new(vec![ScriptedResponse::SseOpen(sse)]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        stream_codex(&model, &context, Some(&codex_options(fetch))).result(),
    )
    .await
    .expect("Timed out waiting for incomplete SSE stream");

    assert_eq!(text_of(&result), "Hello");
    assert_eq!(result.stop_reason, StopReason::Length);
}

#[tokio::test]
async fn aborts_sse_fetch_after_the_configured_http_timeout_when_response_headers_do_not_arrive() {
    // Headers never arrive: the fetch promise never settles.
    let fetch = CodexFetch::new(vec![ScriptedResponse::Pending]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");
    let mut options = codex_options(fetch.clone());
    options.base.base.timeout_ms = Some(10);

    let result = stream_codex(&model, &context, Some(&options))
        .result()
        .await;

    assert_eq!(fetch.request_count(), 1);
    assert_eq!(result.stop_reason, StopReason::Error);
    assert_eq!(
        result.error_message.as_deref(),
        Some("Codex SSE response headers timed out after 10ms")
    );
}

#[tokio::test]
async fn aborts_sse_body_reads_after_response_headers_arrive() {
    let part_one = sse_from_events(&[
        output_item_added(),
        content_part_added(),
        output_text_delta("one"),
    ]);
    let part_two = sse_from_events(&[output_text_delta("two")]);
    let part_three = sse_from_events(&[
        output_item_done("onetwo"),
        json!({
            "type": "response.completed",
            "response": {
                "status": "completed",
                "usage": {
                    "input_tokens": 5,
                    "output_tokens": 3,
                    "total_tokens": 8,
                    "input_tokens_details": {"cached_tokens": 0},
                },
            },
        }),
    ]);
    let dropped = Arc::new(AtomicBool::new(false));
    let fetch = CodexFetch::new(vec![ScriptedResponse::SseTimed(
        vec![(0, part_one), (10, part_two), (20, part_three)],
        Some(Arc::clone(&dropped)),
    )]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");
    let controller = CancellationToken::new();
    let mut options = codex_options(fetch);
    options.base.base.signal = Some(controller.clone());

    let stream = stream_codex(&model, &context, Some(&options));
    let mut events: Vec<String> = Vec::new();
    while let Some(event) = stream.next().await {
        match &event {
            AssistantMessageEvent::TextDelta { delta, .. } => {
                events.push(format!("text_delta:{delta}"));
                if delta == "one" {
                    controller.cancel();
                }
            }
            other => events.push(event_type(other).to_string()),
        }
    }

    let result = stream.result().await;
    assert_eq!(result.stop_reason, StopReason::Aborted);
    assert_eq!(result.error_message.as_deref(), Some("Request was aborted"));
    assert!(events.contains(&"text_delta:one".to_string()));
    assert!(!events.contains(&"text_delta:two".to_string()));
    assert!(dropped.load(Ordering::SeqCst), "SSE body must be cancelled");
}

#[tokio::test]
async fn sets_session_id_x_client_request_id_headers_and_prompt_cache_key_when_session_id_is_provided()
 {
    let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(build_sse_payload(
        "completed",
        false,
        None,
    ))]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");
    let session_id = "test-session-123";
    let mut options = codex_options(fetch.clone());
    options.base.session_id = Some(session_id.to_string());

    stream_codex(&model, &context, Some(&options))
        .result()
        .await;

    let request = fetch.request();
    assert_eq!(header(&request, "session-id"), Some(session_id));
    assert!(header(&request, "session_id").is_none());
    assert_eq!(header(&request, "x-client-request-id"), Some(session_id));
    assert_eq!(
        body_of(&request).get("prompt_cache_key"),
        Some(&json!(session_id))
    );
}

#[tokio::test]
async fn omits_sse_cache_affinity_when_cache_retention_is_none() {
    let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(build_sse_payload(
        "completed",
        false,
        None,
    ))]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");
    let mut options = codex_options(fetch.clone());
    options.base.cache_retention = Some(CacheRetention::None);
    options.base.session_id = Some("one-off-summary".to_string());

    stream_codex(&model, &context, Some(&options))
        .result()
        .await;

    let request = fetch.request();
    assert!(header(&request, "session-id").is_none());
    assert!(header(&request, "x-client-request-id").is_none());
    assert!(body_of(&request).get("prompt_cache_key").is_none());
}

#[tokio::test]
async fn clamps_prompt_cache_key_to_openais_64_character_limit() {
    let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(build_sse_payload(
        "completed",
        false,
        None,
    ))]);
    let captured: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");
    let mut options = codex_options(fetch);
    options.base.session_id = Some("x".repeat(67));
    let capture = Arc::clone(&captured);
    options.base.base.on_payload = Some(Arc::new(
        move |payload: Value, _model: &Model| -> BoxFuture<'static, Option<Value>> {
            let capture = Arc::clone(&capture);
            Box::pin(async move {
                *capture.lock().unwrap() = Some(payload);
                None
            })
        },
    ));

    stream_codex(&model, &context, Some(&options))
        .result()
        .await;

    let payload = captured.lock().unwrap().clone().expect("payload captured");
    assert_eq!(
        payload.get("prompt_cache_key"),
        Some(&json!("x".repeat(64)))
    );
}

#[tokio::test]
async fn clamps_codex_session_id_header_to_64_characters() {
    let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(build_sse_payload(
        "completed",
        false,
        None,
    ))]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");
    let mut options = codex_options(fetch.clone());
    options.base.session_id = Some("x".repeat(67));

    stream_codex(&model, &context, Some(&options))
        .result()
        .await;

    let request = fetch.request();
    assert_eq!(
        header(&request, "session-id"),
        Some("x".repeat(64).as_str())
    );
    assert_eq!(
        header(&request, "x-client-request-id"),
        Some("x".repeat(64).as_str())
    );
}

#[tokio::test]
async fn preserves_gpt_5_5_xhigh_reasoning_effort_from_simple_options() {
    let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(build_sse_payload(
        "completed",
        false,
        None,
    ))]);

    let mut model = codex_model("gpt-5.5", "GPT-5.5");
    model.thinking_level_map = Some(
        [(
            pi_core::ai::types::ModelThinkingLevel::Xhigh,
            Some("xhigh".to_string()),
        )]
        .into_iter()
        .collect(),
    );
    let context = context("You are a helpful assistant.", "Say hello");

    let options = SimpleStreamOptions {
        base: base_options(fetch.clone()),
        reasoning: Some(ThinkingLevel::Xhigh),
        ..Default::default()
    };
    stream_simple_codex(&model, &context, Some(&options))
        .result()
        .await;

    // The TS fetch mock captures `body.reasoning` from the request.
    let reasoning = body_of(&fetch.request()).get("reasoning").cloned();
    assert_eq!(
        reasoning,
        Some(json!({"effort": "xhigh", "summary": "auto"}))
    );
}

#[tokio::test]
async fn forwards_required_tool_choice() {
    let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(build_sse_payload(
        "completed",
        false,
        None,
    ))]);

    let model = codex_model("gpt-5.5", "GPT-5.5");
    let context = Context {
        messages: vec![user_message("Do not call ping. Respond with text instead.")],
        tools: Some(vec![Tool {
            name: "ping".to_string(),
            description: "Ping".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {"value": {"type": "string"}},
                "required": ["value"],
            }),
            constrained_sampling: None,
        }]),
        ..Default::default()
    };
    let mut options = codex_options(fetch.clone());
    options.tool_choice = Some("required".to_string());

    stream_codex(&model, &context, Some(&options))
        .result()
        .await;

    assert_eq!(
        body_of(&fetch.request()).get("tool_choice"),
        Some(&json!("required"))
    );
}

#[tokio::test]
async fn sets_codex_strict_mode_explicitly_and_honors_constrained_sampling() {
    let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(build_sse_payload(
        "completed",
        false,
        None,
    ))]);
    let captured: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));

    let model = codex_model("gpt-5.5", "GPT-5.5");
    let context = Context {
        messages: vec![user_message("Use a tool")],
        tools: Some(vec![
            Tool {
                name: "optional".to_string(),
                description: "Optional constrained sampling".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"],
                }),
                constrained_sampling: Some(ToolConstrainedSampling::Disabled(false)),
            },
            Tool {
                name: "strict".to_string(),
                description: "Strict constrained sampling".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"],
                    "additionalProperties": false,
                }),
                constrained_sampling: Some(ToolConstrainedSampling::Config(
                    ConstrainedSamplingConfig::JsonSchema {
                        strict: ConstrainedSamplingStrict::Prefer,
                    },
                )),
            },
        ]),
        ..Default::default()
    };
    let mut options = codex_options(fetch);
    let capture = Arc::clone(&captured);
    options.base.base.on_payload = Some(Arc::new(
        move |payload: Value, _model: &Model| -> BoxFuture<'static, Option<Value>> {
            let capture = Arc::clone(&capture);
            Box::pin(async move {
                *capture.lock().unwrap() = Some(payload);
                None
            })
        },
    ));

    stream_codex(&model, &context, Some(&options))
        .result()
        .await;

    let payload = captured.lock().unwrap().clone().expect("payload captured");
    let tools = payload
        .get("tools")
        .and_then(Value::as_array)
        .expect("tools");
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["type"], json!("function"));
    assert_eq!(tools[0]["name"], json!("optional"));
    assert_eq!(tools[0]["strict"], json!(null));
    assert_eq!(tools[1]["type"], json!("function"));
    assert_eq!(tools[1]["name"], json!("strict"));
    assert_eq!(tools[1]["strict"], json!(true));
}

/// it.each(["gpt-5.3-codex", "gpt-5.4", "gpt-5.5"]) — looped per the repo's
/// convention for it.each ports.
#[tokio::test]
async fn clamps_minimal_reasoning_effort_to_low() {
    for model_id in ["gpt-5.3-codex", "gpt-5.4", "gpt-5.5"] {
        let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(build_sse_payload(
            "completed",
            false,
            None,
        ))]);

        let mut model = codex_model(model_id, model_id);
        model.thinking_level_map = Some(
            [(
                pi_core::ai::types::ModelThinkingLevel::Minimal,
                Some("low".to_string()),
            )]
            .into_iter()
            .collect(),
        );
        let context = context("You are a helpful assistant.", "Say hello");
        let mut options = codex_options(fetch.clone());
        options.reasoning_effort = Some("minimal".to_string());

        stream_codex(&model, &context, Some(&options))
            .result()
            .await;

        assert_eq!(
            body_of(&fetch.request()).get("reasoning"),
            Some(&json!({"effort": "low", "summary": "auto"})),
            "model {model_id}"
        );
    }
}

/// it.each over (model, tier, multiplier) — the service-tier fixture echoes
/// `service_tier: "default"` and expects the client-sent tier's pricing.
#[tokio::test]
async fn uses_the_client_sent_service_tier_when_codex_echoes_default() {
    let cases: &[(&str, &str, f64)] = &[
        ("gpt-5.1-codex", "flex", 0.5),
        ("gpt-5.1-codex", "priority", 2.0),
        ("gpt-5.5", "flex", 0.5),
        ("gpt-5.5", "priority", 2.5),
    ];

    for (model_id, service_tier, multiplier) in cases {
        let sse = sse_from_events(&[
            output_item_added(),
            content_part_added(),
            output_text_delta("Hello"),
            output_item_done("Hello"),
            json!({
                "type": "response.completed",
                "response": {
                    "status": "completed",
                    "service_tier": "default",
                    "usage": {
                        "input_tokens": 1000000,
                        "output_tokens": 1000000,
                        "total_tokens": 2000000,
                        "input_tokens_details": {"cached_tokens": 0},
                    },
                },
            }),
        ]);
        let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(sse)]);

        let mut model = codex_model(model_id, "GPT-5.1 Codex");
        if *model_id == "gpt-5.5" {
            model.name = "GPT-5.5".to_string();
        }
        model.cost = ModelCost {
            rates: ModelCostRates {
                input: JsF64(1.0),
                output: JsF64(2.0),
                cache_read: JsF64(0.0),
                cache_write: JsF64(0.0),
            },
            tiers: None,
        };
        let context = context("You are a helpful assistant.", "Say hello");
        let mut options = codex_options(fetch);
        options.service_tier = Some(service_tier.to_string());

        let result = stream_codex(&model, &context, Some(&options))
            .result()
            .await;

        assert_eq!(
            result.usage.cost.input.0,
            1.0 * multiplier,
            "{model_id}/{service_tier}"
        );
        assert_eq!(
            result.usage.cost.output.0,
            2.0 * multiplier,
            "{model_id}/{service_tier}"
        );
        assert_eq!(
            result.usage.cost.total.0,
            3.0 * multiplier,
            "{model_id}/{service_tier}"
        );
    }
}

#[tokio::test]
async fn does_not_set_session_id_x_client_request_id_headers_when_session_id_is_not_provided() {
    let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(build_sse_payload(
        "completed",
        false,
        None,
    ))]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");

    stream_codex(&model, &context, Some(&codex_options(fetch.clone())))
        .result()
        .await;

    let request = fetch.request();
    assert!(header(&request, "session-id").is_none());
    assert!(header(&request, "session_id").is_none());
    assert!(header(&request, "x-client-request-id").is_none());
}

/// Shared body of the it.each "uses %s for SSE retries" cases with a
/// deterministic Retry-After value: the retry must not fire before the delay
/// and the stream must complete right after it (the TS test asserts
/// `setTimeout(fn, expectedDelay)` under fake timers).
async fn assert_sse_retry_delay(expected_delay_ms: u64, retry_headers: Vec<(String, String)>) {
    let fetch = CodexFetch::new(vec![
        ScriptedResponse::Error(
            429,
            json!({"error": {"code": "rate_limit_exceeded", "message": "rate limited"}})
                .to_string(),
            retry_headers,
        ),
        ScriptedResponse::Sse(build_sse_payload("completed", false, None)),
    ]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");
    let mut options = codex_options(fetch.clone());
    options.base.base.max_retries = Some(1);

    let stream = stream_codex(&model, &context, Some(&options));
    let mut result = std::pin::pin!(stream.result());
    tokio::time::sleep(Duration::from_millis(expected_delay_ms - 1)).await;
    assert_eq!(
        fetch.request_count(),
        1,
        "retry must not fire before {expected_delay_ms}ms"
    );
    let result = tokio::time::timeout(Duration::from_millis(2), result.as_mut())
        .await
        .expect("retry must fire at the Retry-After delay");

    assert_eq!(text_of(&result), "Hello");
    assert_eq!(fetch.request_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn uses_retry_after_ms_for_sse_retries() {
    assert_sse_retry_delay(
        1500,
        vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("retry-after-ms".to_string(), "1500".to_string()),
        ],
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn uses_retry_after_seconds_for_sse_retries() {
    assert_sse_retry_delay(
        60_000,
        vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("retry-after".to_string(), "60".to_string()),
        ],
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn uses_retry_after_http_date_for_sse_retries() {
    // The TS test freezes Date.now() and sends now + 45s; the Rust port reads
    // the wall clock when parsing the HTTP date, so the observed delay is
    // 45s minus the test's own overhead — assert a tight window around it.
    let retry_at = std::time::SystemTime::now() + Duration::from_secs(45);
    let retry_after = httpdate::fmt_http_date(retry_at);

    let fetch = CodexFetch::new(vec![
        ScriptedResponse::Error(
            429,
            json!({"error": {"code": "rate_limit_exceeded", "message": "rate limited"}})
                .to_string(),
            vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("retry-after".to_string(), retry_after),
            ],
        ),
        ScriptedResponse::Sse(build_sse_payload("completed", false, None)),
    ]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");
    let mut options = codex_options(fetch.clone());
    options.base.base.max_retries = Some(1);

    let stream = stream_codex(&model, &context, Some(&options));
    let mut result = std::pin::pin!(stream.result());
    tokio::time::sleep(Duration::from_millis(45_000 - 1_000)).await;
    assert_eq!(
        fetch.request_count(),
        1,
        "retry must not fire before the HTTP-date delay"
    );
    let result = tokio::time::timeout(Duration::from_millis(3_000), result.as_mut())
        .await
        .expect("retry must fire at the HTTP-date delay");

    assert_eq!(text_of(&result), "Hello");
    assert_eq!(fetch.request_count(), 2);
}

/// it.each([429, 503]) — looped per the repo's convention for it.each ports.
#[tokio::test]
async fn fails_immediately_when_a_retry_delay_exceeds_the_limit() {
    for status in [429u16, 503] {
        let fetch = CodexFetch::new(vec![ScriptedResponse::Error(
            status,
            json!({"error": {"code": "temporarily_unavailable", "message": "retry later"}})
                .to_string(),
            vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("retry-after".to_string(), "2".to_string()),
            ],
        )]);

        let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
        let context = context("You are a helpful assistant.", "Say hello");
        let mut options = codex_options(fetch.clone());
        options.base.base.max_retries = Some(3);
        options.base.base.max_retry_delay_ms = Some(1000);

        let result = stream_codex(&model, &context, Some(&options))
            .result()
            .await;

        assert_eq!(result.stop_reason, StopReason::Error, "status {status}");
        assert_eq!(
            result.error_message.as_deref(),
            Some("Server requested 2s retry delay (max: 1s)"),
            "status {status}"
        );
        assert_eq!(fetch.request_count(), 1, "status {status}");
    }
}

#[tokio::test(start_paused = true)]
async fn uses_exponential_backoff_across_repeated_sse_retries_without_retry_headers() {
    let rate_limited = || {
        ScriptedResponse::Error(
            429,
            json!({"error": {"code": "rate_limit_exceeded", "message": "rate limited"}})
                .to_string(),
            vec![("content-type".to_string(), "application/json".to_string())],
        )
    };
    let fetch = CodexFetch::new(vec![
        rate_limited(),
        rate_limited(),
        rate_limited(),
        ScriptedResponse::Sse(build_sse_payload("completed", false, None)),
    ]);

    let model = codex_model("gpt-5.1-codex", "GPT-5.1 Codex");
    let context = context("You are a helpful assistant.", "Say hello");
    let mut options = codex_options(fetch.clone());
    options.base.base.max_retries = Some(3);

    let stream = stream_codex(&model, &context, Some(&options));
    let mut result = std::pin::pin!(stream.result());

    // The TS test observes setTimeout delays of 1000, 2000, and 4000ms; pin
    // each delay from both sides of virtual time.
    tokio::time::sleep(Duration::from_millis(999)).await;
    assert_eq!(fetch.request_count(), 1, "first delay must be 1000ms");
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert_eq!(fetch.request_count(), 2, "first retry fired after 1000ms");

    tokio::time::sleep(Duration::from_millis(1998)).await;
    assert_eq!(fetch.request_count(), 2, "second delay must be 2000ms");
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert_eq!(fetch.request_count(), 3, "second retry fired after 2000ms");

    tokio::time::sleep(Duration::from_millis(3998)).await;
    assert_eq!(fetch.request_count(), 3, "third delay must be 4000ms");
    let result = tokio::time::timeout(Duration::from_millis(2), result.as_mut())
        .await
        .expect("third retry fired after 4000ms");

    assert_eq!(text_of(&result), "Hello");
    assert_eq!(fetch.request_count(), 4);
}

// ---------------------------------------------------------------------------
// max-thinking.test.ts — "sends max to the Codex Responses API"
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sends_max_to_the_codex_responses_api() {
    let fetch = CodexFetch::new(vec![ScriptedResponse::Sse(build_sse_payload(
        "completed",
        false,
        None,
    ))]);
    let captured: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));

    let model = pi_core::ai::providers::builtin::get_builtin_model("openai-codex", "gpt-5.6-sol")
        .expect("gpt-5.6-sol model");
    let context = context("You are a helpful assistant.", "Hello");

    let capture = Arc::clone(&captured);
    let options = SimpleStreamOptions {
        base: StreamOptions {
            base: ProviderRequestOptions {
                api_key: Some(mock_token()),
                fetch: Some(fetch),
                on_payload: Some(Arc::new(
                    move |payload: Value, _model: &Model| -> BoxFuture<'static, Option<Value>> {
                        let capture = Arc::clone(&capture);
                        Box::pin(async move {
                            *capture.lock().unwrap() = Some(payload);
                            None
                        })
                    },
                )),
                ..Default::default()
            },
            ..Default::default()
        },
        reasoning: Some(ThinkingLevel::Max),
        ..Default::default()
    };
    stream_simple_codex(&model, &context, Some(&options))
        .result()
        .await;

    let payload = captured.lock().unwrap().clone().expect("payload captured");
    assert_eq!(
        payload.get("reasoning"),
        Some(&json!({"effort": "max", "summary": "auto"}))
    );
}

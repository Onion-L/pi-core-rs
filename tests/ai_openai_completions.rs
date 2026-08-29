//! Port of `openai-completions-raw-stop-reason.test.ts`,
//! `openai-completions-prompt-cache.test.ts`, and
//! `openai-completions-empty-tools.test.ts` against a canned SSE transport.

use std::sync::{Arc, Mutex};

use pi_core::ai::api::openai_completions::{OpenAICompletionsOptions, stream};
use pi_core::ai::types::{Context, Message, Model, RoleUser, StopReason, UserContent, UserMessage};
use pi_core::ai::utils::http::{HttpBody, HttpFetch, HttpFetchError, HttpRequest, HttpResponse};
use serde_json::Value;

fn model() -> Model {
    Model {
        id: "test-model".to_string(),
        name: "Test Model".to_string(),
        api: "openai-completions".to_string(),
        provider: "openai".to_string(),
        base_url: "https://api.openai.com/v1".to_string(),
        reasoning: false,
        input: vec![pi_core::ai::types::ModelInput::Text],
        cost: pi_core::ai::types::ModelCost::default(),
        context_window: 128_000,
        max_tokens: 4_096,
        ..Default::default()
    }
}

fn context() -> Context {
    Context {
        messages: vec![Message::User(UserMessage {
            role: RoleUser,
            content: UserContent::Text("hello".to_string()),
            timestamp: 0,
        })],
        ..Default::default()
    }
}

fn chunk_events(chunks: &[Value]) -> String {
    chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect()
}

/// Mock transport returning canned SSE data and capturing requests.
struct RecordingFetch {
    body: String,
    requests: Mutex<Vec<HttpRequest>>,
}

impl RecordingFetch {
    fn new(body: String) -> Self {
        Self {
            body,
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl HttpFetch for RecordingFetch {
    fn fetch<'a>(
        &'a self,
        request: HttpRequest,
    ) -> futures::future::BoxFuture<'a, Result<HttpResponse, HttpFetchError>> {
        self.requests.lock().unwrap().push(request);
        let body = self.body.clone();
        Box::pin(async move {
            Ok(HttpResponse {
                status: 200,
                headers: vec![("content-type".to_string(), "text/event-stream".to_string())],
                body: Box::pin(futures::stream::iter(vec![Ok(bytes::Bytes::from(body))])),
            })
        })
    }
}

fn options_with(fetch: Arc<RecordingFetch>, session_id: Option<&str>) -> OpenAICompletionsOptions {
    OpenAICompletionsOptions {
        base: pi_core::ai::types::StreamOptions {
            base: pi_core::ai::types::ProviderRequestOptions {
                api_key: Some("test".to_string()),
                fetch: Some(fetch),
                ..Default::default()
            },
            session_id: session_id.map(str::to_string),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn preserves_raw_finish_reasons_for_successful_stops() {
    let chunks = vec![
        serde_json::json!({"id": "chatcmpl-1", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
    ];
    let fetch = Arc::new(RecordingFetch::new(chunk_events(&chunks)));

    let message = stream(&model(), &context(), Some(&options_with(fetch, None)))
        .result()
        .await;

    assert_eq!(message.stop_reason, StopReason::Stop);
    assert_eq!(message.raw_stop_reason.as_deref(), Some("stop"));
    assert!(message.error_message.is_none());
}

#[tokio::test]
async fn preserves_raw_finish_reasons_for_provider_error_stops() {
    let chunks = vec![
        serde_json::json!({"id": "chatcmpl-2", "choices": [{"index": 0, "delta": {}, "finish_reason": "content_filter"}]}),
    ];
    let fetch = Arc::new(RecordingFetch::new(chunk_events(&chunks)));

    let message = stream(&model(), &context(), Some(&options_with(fetch, None)))
        .result()
        .await;

    assert_eq!(message.stop_reason, StopReason::Error);
    assert_eq!(message.raw_stop_reason.as_deref(), Some("content_filter"));
    assert_eq!(
        message.error_message.as_deref(),
        Some("Provider finish_reason: content_filter")
    );
}

#[tokio::test]
async fn sends_prompt_cache_key_for_openai_with_session() {
    let chunks = vec![serde_json::json!({
        "choices": [{"delta": {}, "finish_reason": "stop"}],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "prompt_tokens_details": {"cached_tokens": 0},
            "completion_tokens_details": {"reasoning_tokens": 0},
        },
    })];
    let fetch = Arc::new(RecordingFetch::new(chunk_events(&chunks)));

    let message = stream(
        &model(),
        &context(),
        Some(&options_with(fetch.clone(), Some("session-123"))),
    )
    .result()
    .await;

    assert_eq!(message.stop_reason, StopReason::Stop);
    let requests = fetch.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    match &requests[0].body {
        HttpBody::Json(body) => {
            assert_eq!(
                body.get("prompt_cache_key")
                    .and_then(serde_json::Value::as_str),
                Some("session-123")
            );
            assert_eq!(
                body.get("stream_options"),
                Some(&serde_json::json!({"include_usage": true}))
            );
        }
        _ => panic!("expected JSON body"),
    }
    // Usage parsed from the final chunk: cached tokens counted as reads.
    assert_eq!(message.usage.input, 1);
    assert_eq!(message.usage.output, 1);
}

#[tokio::test]
async fn empty_tools_with_tool_history_sends_empty_tools_param() {
    let chunks = vec![
        serde_json::json!({"id": "chatcmpl-3", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
    ];
    let fetch = Arc::new(RecordingFetch::new(chunk_events(&chunks)));

    let context_with_tool_result = Context {
        system_prompt: None,
        messages: vec![
            Message::Assistant(Box::new(
                pi_core::ai::providers::faux::faux_assistant_message(
                    "",
                    pi_core::ai::providers::faux::FauxMessageOptions {
                        stop_reason: Some(StopReason::ToolUse),
                        ..Default::default()
                    },
                ),
            )),
            Message::ToolResult(Box::new(pi_core::ai::types::ToolResultMessage {
                tool_call_id: "call_1".to_string(),
                tool_name: "echo".to_string(),
                content: vec![pi_core::ai::types::BlockContent::Text(
                    pi_core::ai::types::TextContent {
                        text: "done".to_string(),
                        ..Default::default()
                    },
                )],
                is_error: false,
                timestamp: 0,
                ..Default::default()
            })),
        ],
        tools: None,
    };

    let message = stream(
        &model(),
        &context_with_tool_result,
        Some(&options_with(fetch.clone(), None)),
    )
    .result()
    .await;
    assert_eq!(message.stop_reason, StopReason::Stop);

    let requests = fetch.requests.lock().unwrap();
    match &requests[0].body {
        HttpBody::Json(body) => {
            // Tool history present with no tools: empty tools param is sent.
            assert_eq!(body.get("tools"), Some(&serde_json::json!([])));
        }
        _ => panic!("expected JSON body"),
    }
}

#[tokio::test]
async fn streaming_text_deltas_produce_text_blocks() {
    let chunks = vec![
        serde_json::json!({"id": "chatcmpl-4", "choices": [{"index": 0, "delta": {"content": "Hello"}, "finish_reason": null}]}),
        serde_json::json!({"id": "chatcmpl-4", "choices": [{"index": 0, "delta": {"content": " world"}, "finish_reason": "stop"}]}),
    ];
    let fetch = Arc::new(RecordingFetch::new(chunk_events(&chunks)));

    let stream = stream(&model(), &context(), Some(&options_with(fetch, None)));
    let mut saw_text_delta = false;
    let mut saw_text_end = false;
    while let Some(event) = stream.next().await {
        match event {
            pi_core::ai::types::AssistantMessageEvent::TextDelta { delta, .. } => {
                saw_text_delta = true;
                assert!(!delta.is_empty());
            }
            pi_core::ai::types::AssistantMessageEvent::TextEnd { .. } => saw_text_end = true,
            _ => {}
        }
    }
    let message = stream.result().await;
    assert!(saw_text_delta);
    assert!(saw_text_end);
    assert_eq!(
        message.content[0],
        pi_core::ai::types::AssistantContent::Text(pi_core::ai::types::TextContent {
            text: "Hello world".to_string(),
            ..Default::default()
        })
    );
}

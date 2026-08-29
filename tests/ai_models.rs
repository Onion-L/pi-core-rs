//! Port of `pi-core/ai/test/faux-provider.test.ts` (the parts testable
//! without a global provider registry) plus `models-runtime.test.ts`
//! coverage for the `Models` collection over the faux provider.

use std::sync::Arc;

use pi_core::ai::models::{
    Models, clamp_thinking_level, get_supported_thinking_levels, models_are_equal,
};
use pi_core::ai::providers::faux::{
    FauxContent, FauxMessageOptions, FauxResponseStep, faux_assistant_message, faux_provider,
    faux_text, faux_thinking, faux_tool_call,
};
use pi_core::ai::types::{
    AssistantContent, Context, Message, Model, ModelInput, ModelThinkingLevel, SimpleStreamOptions,
    StopReason, StreamOptions, TextContent, ThinkingContent, ToolCall, UserContent, UserMessage,
};
use pi_core::ai::utils::event_stream::{
    AssistantMessageEventStream, create_assistant_message_event_stream,
};

fn user_context(content: &str) -> Context {
    Context {
        system_prompt: None,
        messages: vec![Message::User(UserMessage {
            role: pi_core::ai::types::RoleUser,
            content: UserContent::Text(content.to_string()),
            timestamp: 1_735_689_600_000,
        })],
        tools: None,
    }
}

#[tokio::test]
async fn faux_provider_registers_and_estimates_usage() {
    let faux = faux_provider(Default::default());
    let models = Arc::new(Models::new(Default::default()));
    models.set_provider(Arc::clone(&faux.provider));
    faux.set_responses(vec![FauxResponseStep::Message(Box::new(
        faux_assistant_message("hello world", FauxMessageOptions::default()),
    ))]);

    let context = Context {
        system_prompt: Some("Be concise.".to_string()),
        messages: vec![Message::User(UserMessage {
            role: pi_core::ai::types::RoleUser,
            content: UserContent::Text("hi there".to_string()),
            timestamp: 1_735_689_600_000,
        })],
        tools: None,
    };

    let response = models.complete(&faux.get_model(), &context, None).await;
    assert_eq!(
        response.content,
        vec![AssistantContent::Text(TextContent {
            text: "hello world".to_string(),
            ..Default::default()
        })]
    );
    assert!(response.usage.input > 0);
    assert!(response.usage.output > 0);
    assert_eq!(
        response.usage.total_tokens,
        response.usage.input + response.usage.output
    );
    let state = faux.state.lock().unwrap();
    assert_eq!(state.call_count, 1);
}

#[tokio::test]
async fn faux_provider_supports_helper_blocks() {
    let faux = faux_provider(Default::default());
    let models = Arc::new(Models::new(Default::default()));
    models.set_provider(Arc::clone(&faux.provider));
    let mut scripted = faux_assistant_message("unused", FauxMessageOptions::default());
    scripted.content = vec![
        faux_thinking("think"),
        faux_tool_call("echo", serde_json::json!({"text": "hi"}), None),
        faux_text("done"),
    ];
    scripted.stop_reason = StopReason::ToolUse;
    faux.set_responses(vec![FauxResponseStep::Message(Box::new(scripted))]);

    let response = models
        .complete(&faux.get_model(), &user_context("hi"), None)
        .await;

    assert_eq!(response.content.len(), 3);
    assert_eq!(
        response.content[0],
        AssistantContent::Thinking(ThinkingContent {
            thinking: "think".to_string(),
            ..Default::default()
        })
    );
    match &response.content[1] {
        AssistantContent::ToolCall(tool_call) => {
            assert!(!tool_call.id.is_empty());
            assert_eq!(tool_call.name, "echo");
            let arguments: serde_json::Value = serde_json::to_value(&tool_call.arguments).unwrap();
            assert_eq!(arguments, serde_json::json!({"text": "hi"}));
        }
        other => panic!("expected tool call, got {other:?}"),
    }
    assert_eq!(
        response.content[2],
        AssistantContent::Text(TextContent {
            text: "done".to_string(),
            ..Default::default()
        })
    );
    assert_eq!(response.stop_reason, StopReason::ToolUse);
}

#[tokio::test]
async fn faux_provider_supports_multiple_models_and_factories() {
    let faux = faux_provider(pi_core::ai::providers::faux::RegisterFauxProviderOptions {
        models: vec![
            pi_core::ai::providers::faux::FauxModelDefinition {
                id: "faux-fast".to_string(),
                name: Some("Faux Fast".to_string()),
                reasoning: Some(false),
                ..Default::default()
            },
            pi_core::ai::providers::faux::FauxModelDefinition {
                id: "faux-thinker".to_string(),
                name: Some("Faux Thinker".to_string()),
                reasoning: Some(true),
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    let models = Arc::new(Models::new(Default::default()));
    models.set_provider(Arc::clone(&faux.provider));
    faux.set_responses(vec![
        FauxResponseStep::Factory(Arc::new(|_context, _options, _state, model| {
            Ok(faux_assistant_message(
                format!("{}:{}", model.id, model.reasoning),
                FauxMessageOptions::default(),
            ))
        })),
        FauxResponseStep::Factory(Arc::new(|_context, _options, _state, model| {
            Ok(faux_assistant_message(
                format!("{}:{}", model.id, model.reasoning),
                FauxMessageOptions::default(),
            ))
        })),
    ]);

    assert_eq!(
        faux.models
            .iter()
            .map(|model| model.id.clone())
            .collect::<Vec<_>>(),
        vec!["faux-fast".to_string(), "faux-thinker".to_string()]
    );
    assert_eq!(faux.get_model().id, "faux-fast");
    assert!(!faux.get_model_by_id("faux-fast").unwrap().reasoning);
    assert!(faux.get_model_by_id("faux-thinker").unwrap().reasoning);

    let fast = models
        .complete(
            &faux.get_model_by_id("faux-fast").unwrap(),
            &user_context("hi"),
            None,
        )
        .await;
    let thinker = models
        .complete(
            &faux.get_model_by_id("faux-thinker").unwrap(),
            &user_context("hi"),
            None,
        )
        .await;

    assert_eq!(
        fast.content,
        vec![AssistantContent::Text(TextContent {
            text: "faux-fast:false".to_string(),
            ..Default::default()
        })]
    );
    assert_eq!(
        thinker.content,
        vec![AssistantContent::Text(TextContent {
            text: "faux-thinker:true".to_string(),
            ..Default::default()
        })]
    );
}

#[tokio::test]
async fn faux_provider_rewrites_api_provider_and_model() {
    let faux = faux_provider(pi_core::ai::providers::faux::RegisterFauxProviderOptions {
        api: Some("faux:test".to_string()),
        provider: Some("faux-provider".to_string()),
        models: vec![pi_core::ai::providers::faux::FauxModelDefinition {
            id: "faux-model".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    });
    let models = Arc::new(Models::new(Default::default()));
    models.set_provider(Arc::clone(&faux.provider));
    faux.set_responses(vec![FauxResponseStep::Message(Box::new(
        faux_assistant_message("hello", FauxMessageOptions::default()),
    ))]);

    let response = models
        .complete(&faux.get_model(), &user_context("hi"), None)
        .await;

    assert_eq!(response.api, "faux:test");
    assert_eq!(response.provider, "faux-provider");
    assert_eq!(response.model, "faux-model");
}

#[tokio::test]
async fn faux_provider_consumes_responses_in_order_and_errors_when_exhausted() {
    let faux = faux_provider(Default::default());
    let models = Arc::new(Models::new(Default::default()));
    models.set_provider(Arc::clone(&faux.provider));
    faux.set_responses(vec![
        FauxResponseStep::Message(Box::new(faux_assistant_message(
            "first",
            FauxMessageOptions::default(),
        ))),
        FauxResponseStep::Message(Box::new(faux_assistant_message(
            "second",
            FauxMessageOptions::default(),
        ))),
    ]);

    let context = user_context("hi");
    let first = models.complete(&faux.get_model(), &context, None).await;
    let second = models.complete(&faux.get_model(), &context, None).await;
    let third = models.complete(&faux.get_model(), &context, None).await;

    assert_eq!(first_text(&first), "first");
    assert_eq!(first_text(&second), "second");
    assert_eq!(third.stop_reason, StopReason::Error);
    assert_eq!(
        third.error_message.as_deref(),
        Some("No more faux responses queued")
    );
    assert_eq!(faux.get_pending_response_count(), 0);
}

fn first_text(message: &pi_core::ai::types::AssistantMessage) -> String {
    match &message.content[0] {
        AssistantContent::Text(text) => text.text.clone(),
        other => panic!("expected text, got {other:?}"),
    }
}

#[tokio::test]
async fn faux_provider_streams_events_in_protocol_order() {
    let faux = faux_provider(Default::default());
    let models = Arc::new(Models::new(Default::default()));
    models.set_provider(Arc::clone(&faux.provider));
    let mut scripted = faux_assistant_message("unused", FauxMessageOptions::default());
    scripted.content = vec![faux_text("hello"), faux_thinking("deep")];
    scripted.stop_reason = StopReason::Stop;
    faux.set_responses(vec![FauxResponseStep::Message(Box::new(scripted))]);

    let stream = models.stream(&faux.get_model(), &user_context("hi"), None);
    let mut kinds = Vec::new();
    while let Some(event) = stream.next().await {
        kinds.push(match event {
            pi_core::ai::types::AssistantMessageEvent::Start { .. } => "start",
            pi_core::ai::types::AssistantMessageEvent::TextStart { .. } => "text_start",
            pi_core::ai::types::AssistantMessageEvent::TextDelta { .. } => "text_delta",
            pi_core::ai::types::AssistantMessageEvent::TextEnd { .. } => "text_end",
            pi_core::ai::types::AssistantMessageEvent::ThinkingStart { .. } => "thinking_start",
            pi_core::ai::types::AssistantMessageEvent::ThinkingDelta { .. } => "thinking_delta",
            pi_core::ai::types::AssistantMessageEvent::ThinkingEnd { .. } => "thinking_end",
            pi_core::ai::types::AssistantMessageEvent::ToolcallStart { .. } => "toolcall_start",
            pi_core::ai::types::AssistantMessageEvent::ToolcallDelta { .. } => "toolcall_delta",
            pi_core::ai::types::AssistantMessageEvent::ToolcallEnd { .. } => "toolcall_end",
            pi_core::ai::types::AssistantMessageEvent::Done { .. } => "done",
            pi_core::ai::types::AssistantMessageEvent::Error { .. } => "error",
        });
    }
    let result = stream.result().await;

    assert_eq!(
        kinds,
        vec![
            "start",
            "text_start",
            "text_delta",
            "text_end",
            "thinking_start",
            "thinking_delta",
            "thinking_end",
            "done",
        ]
    );
    assert_eq!(result.stop_reason, StopReason::Stop);
}

// ---------------------------------------------------------------------------
// models-runtime.test.ts port (runtime collection behavior)
// ---------------------------------------------------------------------------

#[test]
fn models_runtime_model_lookup_and_equality() {
    let faux = faux_provider(Default::default());
    let models = Arc::new(Models::new(Default::default()));
    models.set_provider(Arc::clone(&faux.provider));

    assert_eq!(models.get_providers().len(), 1);
    assert!(models.get_provider("faux").is_some());
    assert!(models.get_provider("missing").is_none());
    assert!(!models.get_models(None).is_empty());
    assert!(models.get_model("faux", "faux-1").is_some());
    assert!(models.get_model("faux", "missing").is_none());
    assert!(models.get_model("missing", "faux-1").is_none());

    let model = models.get_model("faux", "faux-1").unwrap();
    assert!(models_are_equal(Some(&model), Some(&model)));
    let other = Model {
        id: "other".to_string(),
        ..model.clone()
    };
    assert!(!models_are_equal(Some(&model), Some(&other)));
    assert!(!models_are_equal(Some(&model), None));
}

#[test]
fn models_runtime_thinking_levels() {
    let mut model = faux_provider(Default::default()).get_model();
    model.reasoning = false;
    assert_eq!(
        get_supported_thinking_levels(&model),
        vec![ModelThinkingLevel::Off]
    );

    model.reasoning = true;
    let levels = get_supported_thinking_levels(&model);
    assert_eq!(
        levels,
        vec![
            ModelThinkingLevel::Off,
            ModelThinkingLevel::Minimal,
            ModelThinkingLevel::Low,
            ModelThinkingLevel::Medium,
            ModelThinkingLevel::High,
        ]
    );
    assert_eq!(
        clamp_thinking_level(&model, ModelThinkingLevel::Xhigh),
        ModelThinkingLevel::High
    );

    // null marks a level as unsupported.
    let mut map = std::collections::BTreeMap::new();
    map.insert(ModelThinkingLevel::High, Some("adaptive".to_string()));
    map.insert(ModelThinkingLevel::Medium, None);
    model.thinking_level_map = Some(map);
    let levels = get_supported_thinking_levels(&model);
    assert!(!levels.contains(&ModelThinkingLevel::Medium));
    assert!(levels.contains(&ModelThinkingLevel::High));
    // TS clamps upward first: medium is unsupported (null), high maps to a
    // provider value, so medium clamps to high.
    assert_eq!(
        clamp_thinking_level(&model, ModelThinkingLevel::Medium),
        ModelThinkingLevel::High
    );
}

#[tokio::test]
async fn models_runtime_requires_known_provider() {
    let faux = faux_provider(Default::default());
    let model = faux.get_model();
    let models = Arc::new(Models::new(Default::default()));
    // Provider not registered: the lazy stream surfaces the setup error.
    let response = models.complete(&model, &user_context("hi"), None).await;
    assert_eq!(response.stop_reason, StopReason::Error);
    assert_eq!(
        response.error_message.as_deref(),
        Some("Unknown provider: faux")
    );
}

// ---------------------------------------------------------------------------
// Multi-api dispatch: port of the "produces a stream error for a model whose
// api has no implementation" case from `pi-core/ai/test/providers.test.ts`.

/// The `testModel` fixture from providers.test.ts.
fn provider_test_model(api: &str, id: &str) -> Model {
    Model {
        id: id.to_string(),
        name: id.to_string(),
        api: api.to_string(),
        provider: "mixed".to_string(),
        base_url: "https://example.test/v1".to_string(),
        reasoning: false,
        input: vec![ModelInput::Text],
        context_window: 10_000,
        max_tokens: 1000,
        ..Default::default()
    }
}

#[derive(Default)]
struct NoopStreams;

impl pi_core::ai::models::ProviderStreams for NoopStreams {
    fn stream(
        &self,
        _model: &Model,
        _context: &Context,
        _options: Option<&StreamOptions>,
    ) -> AssistantMessageEventStream {
        create_assistant_message_event_stream()
    }

    fn stream_simple(
        &self,
        _model: &Model,
        _context: &Context,
        _options: Option<&SimpleStreamOptions>,
    ) -> AssistantMessageEventStream {
        create_assistant_message_event_stream()
    }
}

struct TestApiKeyAuth;

impl pi_core::ai::auth::types::ApiKeyAuth for TestApiKeyAuth {
    fn name(&self) -> &str {
        "Test"
    }

    fn resolve(
        &self,
        _input: pi_core::ai::auth::types::ApiKeyAuthInput,
    ) -> pi_core::ai::auth::types::AuthFuture<
        Result<
            Option<pi_core::ai::auth::types::AuthResult>,
            pi_core::ai::auth::types::AuthStorageError,
        >,
    > {
        Box::pin(std::future::ready(Ok(Some(
            pi_core::ai::auth::types::AuthResult::default(),
        ))))
    }
}

#[tokio::test]
async fn produces_a_stream_error_for_a_model_whose_api_has_no_implementation() {
    let mut by_api = std::collections::BTreeMap::new();
    by_api.insert(
        "api-a".to_string(),
        Arc::new(NoopStreams) as Arc<dyn pi_core::ai::models::ProviderStreams>,
    );
    let provider =
        pi_core::ai::models::create_provider(pi_core::ai::models::CreateProviderOptions {
            id: "mixed".to_string(),
            name: None,
            base_url: None,
            headers: None,
            auth: pi_core::ai::auth::types::ProviderAuth::api_key(Arc::new(TestApiKeyAuth)),
            models: vec![provider_test_model("api-a", "model-a")],
            fetch_models: None,
            filter_models: None,
            api: pi_core::ai::models::ProviderApi::ByApi(by_api),
        });

    let context = Context::default();
    let result = provider
        .stream_simple(&provider_test_model("api-ghost", "model-x"), &context, None)
        .result()
        .await;

    assert_eq!(result.stop_reason, StopReason::Error);
    assert!(
        result
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("no API implementation"))
    );
}

#[allow(unused)]
fn helper_type_check(_: FauxContent, _: ToolCall) {}

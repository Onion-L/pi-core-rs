//! Port of `pi-core/ai/test/providers.test.ts` (provider-auth cases; the
//! builtin-catalog and dispatch cases live in this file too as they land).

use std::sync::{Arc, Mutex};

use pi_core::ai::auth::types::{
    ApiKeyAuthInput, ApiKeyCredential, AuthContext, AuthEvent, AuthFuture, AuthInteraction,
    AuthPrompt, AuthStorageError,
};
use pi_core::ai::models::{CreateModelsOptions, Models};
use pi_core::ai::providers::cloudflare_ai_gateway::cloudflare_ai_gateway_provider;
use pi_core::ai::providers::cloudflare_workers_ai::cloudflare_workers_ai_provider;
use pi_core::ai::providers::google_vertex::google_vertex_provider;
use pi_core::ai::types::ProviderEnv;

/// The `fakeAuthContext(env, files)` helper from providers.test.ts.
struct FakeAuthContext {
    env: ProviderEnv,
    files: Vec<String>,
}

impl AuthContext for FakeAuthContext {
    fn env(&self, name: &str) -> AuthFuture<Option<String>> {
        Box::pin(std::future::ready(self.env.get(name).cloned()))
    }

    fn file_exists(&self, path: &str) -> AuthFuture<bool> {
        Box::pin(std::future::ready(
            self.files.iter().any(|file| file == path),
        ))
    }
}

fn fake_auth_context(env: &[(&str, &str)], files: &[&str]) -> Arc<FakeAuthContext> {
    Arc::new(FakeAuthContext {
        env: env
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
        files: files.iter().map(|file| file.to_string()).collect(),
    })
}

fn auth_input(ctx: Arc<FakeAuthContext>, credential: Option<ApiKeyCredential>) -> ApiKeyAuthInput {
    ApiKeyAuthInput {
        ctx,
        credential,
        signal: tokio_util::sync::CancellationToken::new(),
    }
}

/// A prompt interaction answering from a scripted queue (the TS tests'
/// `prompt: async () => answers.shift()!`).
struct ScriptedInteraction {
    answers: Mutex<Vec<String>>,
    events: Mutex<Vec<AuthEvent>>,
}

impl AuthInteraction for ScriptedInteraction {
    fn signal(&self) -> Option<tokio_util::sync::CancellationToken> {
        None
    }

    fn prompt(&self, _prompt: AuthPrompt) -> AuthFuture<Result<String, AuthStorageError>> {
        let answer = {
            let mut answers = self.answers.lock().unwrap();
            (!answers.is_empty()).then(|| answers.remove(0))
        };
        Box::pin(std::future::ready(match answer {
            Some(answer) => Ok(answer),
            None => Err(AuthStorageError("no more answers".to_string())),
        }))
    }

    fn notify(&self, event: AuthEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn scripted(answers: &[&str]) -> Arc<ScriptedInteraction> {
    Arc::new(ScriptedInteraction {
        answers: Mutex::new(answers.iter().map(|answer| answer.to_string()).collect()),
        events: Mutex::new(Vec::new()),
    })
}

#[tokio::test]
async fn runs_provider_owned_vertex_api_key_and_adc_login_flows() {
    let auth = google_vertex_provider()
        .auth()
        .api_key
        .clone()
        .expect("api key auth");

    let key_interaction = scripted(&["api-key", "vertex-key"]);
    let credential = auth.login(key_interaction).expect("login").await.unwrap();
    assert_eq!(
        credential,
        ApiKeyCredential {
            key: Some("vertex-key".to_string()),
            env: None,
        }
    );

    let adc_interaction = scripted(&["adc", "project-id", "us-central1"]);
    let credential = auth
        .login(adc_interaction.clone())
        .expect("login")
        .await
        .unwrap();
    let mut expected_env = ProviderEnv::new();
    expected_env.insert("GOOGLE_CLOUD_PROJECT".to_string(), "project-id".to_string());
    expected_env.insert(
        "GOOGLE_CLOUD_LOCATION".to_string(),
        "us-central1".to_string(),
    );
    assert_eq!(
        credential,
        ApiKeyCredential {
            key: None,
            env: Some(expected_env),
        }
    );
    let events = adc_interaction.events.lock().unwrap().clone();
    match &events[0] {
        AuthEvent::Info { links, .. } => assert!(links.iter().any(|link| {
            link.label
                .as_deref()
                .is_some_and(|label| label.contains("Application Default Credentials"))
        })),
        other => panic!("expected info event, got {other:?}"),
    }

    let mut credential_env = ProviderEnv::new();
    credential_env.insert("GOOGLE_CLOUD_PROJECT".to_string(), "project-id".to_string());
    credential_env.insert(
        "GOOGLE_CLOUD_LOCATION".to_string(),
        "us-central1".to_string(),
    );
    let resolved = auth
        .resolve(auth_input(
            fake_auth_context(
                &[],
                &["~/.config/gcloud/application_default_credentials.json"],
            ),
            Some(ApiKeyCredential {
                key: None,
                env: Some(credential_env.clone()),
            }),
        ))
        .await
        .unwrap()
        .expect("configured");
    assert_eq!(resolved.auth, Default::default());
    assert_eq!(resolved.env, Some(credential_env));
}

#[tokio::test]
async fn resolves_vertex_via_adc_file_plus_project_and_location() {
    let adc = "~/.config/gcloud/application_default_credentials.json";
    let configured = Arc::new(Models::new(CreateModelsOptions {
        auth_context: Some(fake_auth_context(
            &[
                ("GOOGLE_CLOUD_PROJECT", "proj"),
                ("GOOGLE_CLOUD_LOCATION", "us-central1"),
            ],
            &[adc],
        )),
        ..Default::default()
    }));
    configured.set_provider(google_vertex_provider());

    let result = configured
        .get_auth("google-vertex", None, None)
        .await
        .unwrap()
        .expect("configured");
    assert_eq!(result.auth, Default::default());
    assert!(
        result
            .source
            .as_deref()
            .is_some_and(|source| source.contains("application default"))
    );

    // ADC without project/location is not configured.
    let partial = Arc::new(Models::new(CreateModelsOptions {
        auth_context: Some(fake_auth_context(
            &[("GOOGLE_CLOUD_PROJECT", "proj")],
            &[adc],
        )),
        ..Default::default()
    }));
    partial.set_provider(google_vertex_provider());
    assert!(
        partial
            .get_auth("google-vertex", None, None)
            .await
            .unwrap()
            .is_none()
    );

    // An explicit key wins over ADC.
    let keyed = Arc::new(Models::new(CreateModelsOptions {
        auth_context: Some(fake_auth_context(
            &[("GOOGLE_CLOUD_API_KEY", "vertex-key")],
            &[],
        )),
        ..Default::default()
    }));
    keyed.set_provider(google_vertex_provider());
    let result = keyed
        .get_auth("google-vertex", None, None)
        .await
        .unwrap()
        .expect("configured");
    assert_eq!(result.auth.api_key.as_deref(), Some("vertex-key"));
}

#[tokio::test]
async fn requires_cloudflare_workers_ai_account_config_and_returns_scoped_env() {
    let missing_account = Arc::new(Models::new(CreateModelsOptions {
        auth_context: Some(fake_auth_context(&[("CLOUDFLARE_API_KEY", "cf-key")], &[])),
        ..Default::default()
    }));
    missing_account.set_provider(cloudflare_workers_ai_provider());
    assert!(
        missing_account
            .get_auth("cloudflare-workers-ai", None, None)
            .await
            .unwrap()
            .is_none()
    );

    let configured = Arc::new(Models::new(CreateModelsOptions {
        auth_context: Some(fake_auth_context(
            &[
                ("CLOUDFLARE_API_KEY", "cf-key"),
                ("CLOUDFLARE_ACCOUNT_ID", "account-id"),
            ],
            &[],
        )),
        ..Default::default()
    }));
    configured.set_provider(cloudflare_workers_ai_provider());
    let result = configured
        .get_auth("cloudflare-workers-ai", None, None)
        .await
        .unwrap()
        .expect("configured");
    assert_eq!(result.auth.api_key.as_deref(), Some("cf-key"));
    let mut expected_env = ProviderEnv::new();
    expected_env.insert(
        "CLOUDFLARE_ACCOUNT_ID".to_string(),
        "account-id".to_string(),
    );
    assert_eq!(result.env, Some(expected_env));
}

#[tokio::test]
async fn requires_cloudflare_ai_gateway_account_and_gateway_config_and_returns_scoped_env_headers()
{
    let missing_gateway = Arc::new(Models::new(CreateModelsOptions {
        auth_context: Some(fake_auth_context(
            &[
                ("CLOUDFLARE_API_KEY", "cf-key"),
                ("CLOUDFLARE_ACCOUNT_ID", "account-id"),
            ],
            &[],
        )),
        ..Default::default()
    }));
    missing_gateway.set_provider(cloudflare_ai_gateway_provider());
    assert!(
        missing_gateway
            .get_auth("cloudflare-ai-gateway", None, None)
            .await
            .unwrap()
            .is_none()
    );

    let configured = Arc::new(Models::new(CreateModelsOptions {
        auth_context: Some(fake_auth_context(
            &[
                ("CLOUDFLARE_API_KEY", "cf-key"),
                ("CLOUDFLARE_ACCOUNT_ID", "account-id"),
                ("CLOUDFLARE_GATEWAY_ID", "gateway-id"),
            ],
            &[],
        )),
        ..Default::default()
    }));
    configured.set_provider(cloudflare_ai_gateway_provider());
    let result = configured
        .get_auth("cloudflare-ai-gateway", None, None)
        .await
        .unwrap()
        .expect("configured");

    let mut expected_headers = pi_core::ai::types::ProviderHeaders::new();
    expected_headers.insert(
        "cf-aig-authorization".to_string(),
        Some("Bearer cf-key".to_string()),
    );
    expected_headers.insert("Authorization".to_string(), None);
    expected_headers.insert("x-api-key".to_string(), None);
    assert_eq!(result.auth.headers, Some(expected_headers));

    let mut expected_env = ProviderEnv::new();
    expected_env.insert(
        "CLOUDFLARE_ACCOUNT_ID".to_string(),
        "account-id".to_string(),
    );
    expected_env.insert(
        "CLOUDFLARE_GATEWAY_ID".to_string(),
        "gateway-id".to_string(),
    );
    assert_eq!(result.env, Some(expected_env));
}

// ---------------------------------------------------------------------------
// Builtin catalogue (providers.test.ts "builtin providers" cases).

#[tokio::test]
async fn builtin_models_registers_every_builtin_provider_with_models() {
    let models = pi_core::ai::providers::builtin::builtin_models(Default::default());
    let providers = models.get_providers();
    assert_eq!(
        providers.len(),
        pi_core::ai::providers::builtin::builtin_providers().len()
    );
    let ids: Vec<&str> = providers.iter().map(|p| p.id()).collect();
    assert!(ids.contains(&"anthropic"));

    let anthropic = models.get_model("anthropic", "claude-haiku-4-5").unwrap();
    assert_eq!(anthropic.api, "anthropic-messages");

    assert!(models.get_models(None).len() > 500);

    // Static providers list models immediately; radius (purely dynamic) is
    // not registered yet and lands with its provider port.
    for provider in &providers {
        let list = models.get_models(Some(provider.id()));
        assert!(!list.is_empty(), "no models for {}", provider.id());
        assert!(list.iter().all(|m| m.provider == provider.id()));
    }
}

#[test]
fn stores_native_constrained_sampling_capabilities_in_model_metadata() {
    use pi_core::ai::providers::builtin::get_builtin_model;
    let gpt4o = get_builtin_model("openai", "gpt-4o").unwrap();
    assert_eq!(
        gpt4o.compat.as_ref().unwrap().supports_strict_mode,
        Some(true)
    );
    assert_eq!(
        gpt4o
            .compat
            .as_ref()
            .unwrap()
            .supports_open_ai_grammar_tools,
        None
    );
    let gpt54 = get_builtin_model("openai", "gpt-5.4").unwrap();
    let compat = gpt54.compat.as_ref().unwrap();
    assert_eq!(compat.supports_strict_mode, Some(true));
    assert_eq!(compat.supports_open_ai_grammar_tools, Some(true));
    let haiku = get_builtin_model("anthropic", "claude-haiku-4-5").unwrap();
    assert_eq!(
        haiku.compat.as_ref().unwrap().supports_strict_tools,
        Some(true)
    );
}

#[test]
fn uses_official_kimi_k3_pricing_for_moonshot_providers() {
    let models = pi_core::ai::providers::builtin::builtin_models(Default::default());
    for provider in ["moonshotai", "moonshotai-cn"] {
        let cost = models.get_model(provider, "kimi-k3").unwrap().cost;
        assert_eq!(cost.rates.input.0, 3.0, "{provider} input");
        assert_eq!(cost.rates.output.0, 15.0, "{provider} output");
        assert_eq!(cost.rates.cache_read.0, 0.3, "{provider} cacheRead");
        assert_eq!(cost.rates.cache_write.0, 0.0, "{provider} cacheWrite");
    }
}

#[test]
fn uses_api_equivalent_implied_pricing_for_kimi_coding_subscription_models() {
    let models = pi_core::ai::providers::builtin::builtin_models(Default::default());
    let expected: &[(&str, f64, f64, f64, f64)] = &[
        // (modelId, input, output, cacheRead, cacheWrite)
        ("k3", 3.0, 15.0, 0.3, 0.0),
        ("kimi-for-coding-highspeed", 1.9, 8.0, 0.38, 0.0),
    ];
    for (model_id, input, output, cache_read, cache_write) in expected {
        let cost = models
            .get_model("kimi-coding", model_id)
            .unwrap_or_else(|| panic!("missing kimi-coding/{model_id}"))
            .cost;
        assert_eq!(cost.rates.input.0, *input, "{model_id} input");
        assert_eq!(cost.rates.output.0, *output, "{model_id} output");
        assert_eq!(cost.rates.cache_read.0, *cache_read, "{model_id} cacheRead");
        assert_eq!(
            cost.rates.cache_write.0, *cache_write,
            "{model_id} cacheWrite"
        );
    }
}

#[tokio::test]
async fn resolves_anthropic_bearer_auth_from_env_with_auth_token_precedence() {
    let models = Arc::new(Models::new(CreateModelsOptions {
        auth_context: Some(fake_auth_context(
            &[
                ("ANTHROPIC_AUTH_TOKEN", "auth-token"),
                ("ANTHROPIC_OAUTH_TOKEN", "oauth-token"),
                ("ANTHROPIC_API_KEY", "api-key"),
            ],
            &[],
        )),
        ..Default::default()
    }));
    models.set_provider(pi_core::ai::providers::anthropic::anthropic_provider());

    let result = models
        .get_auth("anthropic", None, None)
        .await
        .unwrap()
        .expect("configured");
    let mut expected_headers = pi_core::ai::types::ProviderHeaders::new();
    expected_headers.insert(
        "Authorization".to_string(),
        Some("Bearer auth-token".to_string()),
    );
    assert_eq!(result.auth.headers, Some(expected_headers),);
    assert_eq!(result.source.as_deref(), Some("ANTHROPIC_AUTH_TOKEN"));
}

#[tokio::test]
async fn preserves_anthropic_oauth_token_precedence_over_the_api_key() {
    let models = Arc::new(Models::new(CreateModelsOptions {
        auth_context: Some(fake_auth_context(
            &[
                ("ANTHROPIC_API_KEY", "key"),
                ("ANTHROPIC_OAUTH_TOKEN", "oauth-token"),
            ],
            &[],
        )),
        ..Default::default()
    }));
    models.set_provider(pi_core::ai::providers::anthropic::anthropic_provider());

    let result = models
        .get_auth("anthropic", None, None)
        .await
        .unwrap()
        .expect("configured");
    assert_eq!(result.auth.api_key.as_deref(), Some("oauth-token"));
    assert_eq!(result.source.as_deref(), Some("ANTHROPIC_OAUTH_TOKEN"));
}

// ---------------------------------------------------------------------------
// envApiKeyAuth (providers.test.ts cases).

#[tokio::test]
async fn env_api_key_auth_prefers_stored_credential_and_falls_back_in_order() {
    use pi_core::ai::auth::helpers::env_api_key_auth;
    let auth = env_api_key_auth("Test key", &["FIRST_KEY", "SECOND_KEY"]);

    let stored = auth
        .resolve(auth_input(
            fake_auth_context(&[("FIRST_KEY", "env")], &[]),
            Some(ApiKeyCredential {
                key: Some("stored".to_string()),
                env: None,
            }),
        ))
        .await
        .unwrap()
        .expect("configured");
    assert_eq!(stored.auth.api_key.as_deref(), Some("stored"));
    assert_eq!(stored.source.as_deref(), Some("stored credential"));

    let second = auth
        .resolve(auth_input(
            fake_auth_context(&[("SECOND_KEY", "second")], &[]),
            None,
        ))
        .await
        .unwrap()
        .expect("configured");
    assert_eq!(second.auth.api_key.as_deref(), Some("second"));
    assert_eq!(second.source.as_deref(), Some("SECOND_KEY"));

    assert!(
        auth.resolve(auth_input(fake_auth_context(&[], &[]), None))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn env_api_key_auth_login_prompts_for_a_secret() {
    use pi_core::ai::auth::helpers::env_api_key_auth;
    use pi_core::ai::auth::types::AuthPromptKind;
    let auth = env_api_key_auth("Test key", &["TEST_KEY"]);

    struct PromptInspectingInteraction;
    impl AuthInteraction for PromptInspectingInteraction {
        fn signal(&self) -> Option<tokio_util::sync::CancellationToken> {
            None
        }
        fn prompt(&self, prompt: AuthPrompt) -> AuthFuture<Result<String, AuthStorageError>> {
            assert!(matches!(prompt.kind, AuthPromptKind::Secret { .. }));
            Box::pin(std::future::ready(Ok("entered-key".to_string())))
        }
        fn notify(&self, _event: AuthEvent) {}
    }

    let credential = auth
        .login(Arc::new(PromptInspectingInteraction))
        .expect("login supported")
        .await
        .unwrap();
    assert_eq!(
        credential,
        ApiKeyCredential {
            key: Some("entered-key".to_string()),
            env: None,
        }
    );
}

// ---------------------------------------------------------------------------
// Builtin images models (images-models.test.ts case 6).

#[tokio::test]
async fn builtin_images_models_registers_the_openrouter_provider_with_its_catalog() {
    use pi_core::ai::providers::builtin::builtin_images_models;
    let models = builtin_images_models(CreateModelsOptions {
        auth_context: Some(fake_auth_context(&[("OPENROUTER_API_KEY", "or-key")], &[])),
        ..Default::default()
    });

    let providers = models.get_providers();
    let ids: Vec<&str> = providers.iter().map(|p| p.id()).collect();
    assert_eq!(ids, vec!["openrouter"]);

    let list = models.get_models(Some("openrouter"));
    assert!(!list.is_empty());
    assert!(list.iter().all(|m| m.api == "openrouter-images"));

    let auth = models
        .get_auth_for_model(&list[0], None)
        .await
        .unwrap()
        .expect("configured");
    assert_eq!(auth.auth.api_key.as_deref(), Some("or-key"));
}

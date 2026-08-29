//! Port of `pi-core/ai/test/providers.test.ts` (provider-auth cases; the
//! builtin-catalog and dispatch cases live in this file too as they land).

use std::sync::{Arc, Mutex};

use pi_core::ai::auth::types::{
    ApiKeyAuthInput, ApiKeyCredential, AuthContext, AuthEvent, AuthFuture, AuthInteraction,
    AuthPrompt, AuthStorageError,
};
use pi_core::ai::models::{CreateModelsOptions, Models};
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

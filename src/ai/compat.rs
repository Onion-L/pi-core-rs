//! Port of `pi-core/ai/src/compat/extension-oauth-types.ts` and
//! `pi-core/ai/src/oauth.ts`: the legacy extension OAuth declaration surface.
//!
//! TypeScript exports these as type-only compatibility shapes for the
//! coding-agent extension API. The Rust port mirrors the payload structs;
//! the callback surface (`OAuthLoginCallbacks`) becomes a trait.

use crate::ai::types::ProviderEnv;
use std::future::Future;
use std::pin::Pin;

/// Boxed future returned by extension OAuth callbacks.
pub type BoxedCompatFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
use tokio_util::sync::CancellationToken;

/// Port of `OAuthCredentials` re-export: the extension-shaped OAuth token.
/// The TS shape has an open index signature; unknown fields are preserved in
/// `extra`.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExtensionOAuthCredentials {
    pub refresh: String,
    pub access: String,
    pub expires: i64,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Port of the legacy extension `OAuthPrompt`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OAuthPrompt {
    pub message: String,
    pub placeholder: Option<String>,
    pub allow_empty: Option<bool>,
}

/// Port of the legacy extension `OAuthAuthInfo`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OAuthAuthInfo {
    pub url: String,
    pub instructions: Option<String>,
}

/// Port of the legacy extension `OAuthDeviceCodeInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct OAuthDeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub interval_seconds: Option<u64>,
    pub expires_in_seconds: Option<u64>,
}

/// Port of the legacy extension `OAuthSelectOption`.
#[derive(Clone, Debug, PartialEq)]
pub struct OAuthSelectOption {
    pub id: String,
    pub label: String,
}

/// Port of the legacy extension `OAuthSelectPrompt`.
#[derive(Clone, Debug, PartialEq)]
pub struct OAuthSelectPrompt {
    pub message: String,
    pub options: Vec<OAuthSelectOption>,
}

/// Port of `OAuthLoginCallbacks`: the callback surface retained for
/// coding-agent extension compatibility.
pub trait OAuthLoginCallbacks: Send + Sync {
    /// Authorization URL notification.
    fn on_auth(&self, info: OAuthAuthInfo);

    /// Device-code notification.
    fn on_device_code(&self, info: OAuthDeviceCodeInfo);

    /// Prompts the user; errors on cancel/abort.
    fn on_prompt(&self, prompt: OAuthPrompt) -> BoxedCompatFuture<Result<String, String>>;

    /// Optional progress notification.
    fn on_progress(&self, _message: &str) {}

    /// Optional manual code input.
    fn on_manual_code_input(&self) -> BoxedCompatFuture<Result<String, String>> {
        Box::pin(async { Err("manual code input is not supported".to_string()) })
    }

    /// Optional select prompt; `None` cancels the selection.
    fn on_select(
        &self,
        prompt: OAuthSelectPrompt,
    ) -> BoxedCompatFuture<Result<Option<String>, String>>;

    /// Optional flow cancellation signal.
    fn signal(&self) -> Option<CancellationToken> {
        None
    }
}

/// Scoped environment values used by extension OAuth flows.
pub type ExtensionOAuthEnv = ProviderEnv;

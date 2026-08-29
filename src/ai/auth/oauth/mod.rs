//! Port of `pi-core/ai/src/auth/oauth`: OAuth flows and shared helpers.

pub mod device_code;
pub mod kimi_coding;
pub mod pkce;
pub mod xai;

/// Injectable wall clock (epoch milliseconds) shared by the OAuth flows.
pub type NowMs = std::sync::Arc<dyn Fn() -> i64 + Send + Sync>;

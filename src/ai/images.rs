//! Port of `pi-core/ai/src/images.ts`, `images-api-registry.ts`, and
//! `providers/images/register-builtins.ts`: image API dispatch.
//!
//! The TypeScript registry exists to lazy-load API modules and validate api
//! tags at runtime (`Mismatched api` / `No API provider registered`). Rust
//! links API adapters at compile time, so dispatch is a static match and the
//! mismatch branch is structurally unreachable.

use crate::ai::api::openrouter_images;
use crate::ai::types::{AssistantImages, ImagesContext, ImagesModel, ImagesOptions};

/// Port of `generateImages` from `images.ts`. Rejects (returns `Err`) when no
/// API provider is registered for the model's api, matching the TypeScript
/// async throw.
pub async fn generate_images(
    model: &ImagesModel,
    context: &ImagesContext,
    options: Option<&ImagesOptions>,
) -> Result<AssistantImages, String> {
    match model.api.as_str() {
        "openrouter-images" => {
            Ok(openrouter_images::generate_images(model, context, options).await)
        }
        api => Err(format!("No API provider registered for api: {api}")),
    }
}

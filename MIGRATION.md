# Migration checklist

Tracking for the TypeScript → Rust port of `@earendil-works/pi-telemetry`,
`@earendil-works/pi-ai`, and `@earendil-works/pi-agent-core` v0.84.4.

Status legend:

- **pending** — not yet ported.
- **done** — ported with its applicable tests passing.
- **exception** — ported with a documented, minimal deviation (the note says
  where the deviation is documented), or deliberately out of scope.

Out of scope per `AGENTS.md`: the `node:sqlite` session backend (a separate
upstream package).

## pi-telemetry

### Source modules

| TypeScript source | Rust module | Status |
|---|---|---|
| `src/index.ts` | `src/telemetry/mod.rs` | done |
| `src/noop.ts` | `src/telemetry/noop.rs` | done |
| `src/memory.ts` | `src/telemetry/memory.rs` | done |
| `src/testing/index.ts` | `src/telemetry/testing/mod.rs` | done |
| `src/testing/types.ts` | `src/telemetry/testing/types.rs` | done |
| `src/testing/conformance.ts` | `src/telemetry/testing/conformance.rs` | done |
### Tests

| TypeScript test | Rust test | Status |
|---|---|---|
| `test/telemetry.test.ts` | `tests/telemetry.rs` | done |
| `test/conformance.test.ts` | `tests/telemetry_conformance.rs` | done |
### Documented deviations

- TypeScript callback rejections map to `Result` errors on the Rust `try_*`
  wrappers; automatic error-status names use the short Rust type name instead
  of `error.name` (documented in `src/telemetry/mod.rs`).
- JS `Proxy`-based passivity cases are adapted to plain-value equivalents
  (documented inline in `src/telemetry/testing/conformance.rs`).
- TypeScript conditional-type vocabulary checking (`SchemaTelemetrySpan`,
  `InferStartAttributes`, …) is compile-time only; the runtime behavior of
  `createTypedSpanStarter` is ported, the type-level checks have no Rust
  equivalent (documented in `src/telemetry/mod.rs`).

## pi-ai

### Source modules

| TypeScript source | Rust module | Status |
|---|---|---|
| `src/api/anthropic-messages.lazy.ts` | `src/ai/providers/apis.rs` | done |
| `src/api/anthropic-messages.ts` | `src/ai/api/anthropic_messages.rs` | done |
| `src/api/azure-openai-responses.lazy.ts` | `src/ai/providers/apis.rs` | done |
| `src/api/azure-openai-responses.ts` | `src/ai/api/azure_openai_responses.rs` | done |
| `src/api/bedrock-converse-stream.lazy.ts` | | pending |
| `src/api/bedrock-converse-stream.ts` | `src/ai/api/bedrock_converse_stream.rs` | partial (request building, event handling, and the SDK-mock seam ported with their tests; the SigV4/bearer wire transport and event-stream framing land next) |
| `src/api/cloudflare-gateway-binding.ts` | `src/ai/api/cloudflare_gateway_binding.rs` | done (Request/init split and fetch-signal forwarding have no Rust transport equivalent; documented in the module) |
| `src/api/cloudflare.ts` | `src/ai/api/cloudflare.rs` | done |
| `src/api/constrained-sampling.ts` | `src/ai/api/constrained-sampling.rs` | done |
| `src/api/github-copilot-headers.ts` | `src/ai/api/github-copilot-headers.rs` | done |
| `src/api/google-generative-ai.lazy.ts` | `src/ai/providers/apis.rs` | done |
| `src/api/google-generative-ai.ts` | `src/ai/api/google_generative_ai.rs` | done |
| `src/api/google-shared.ts` | `src/ai/api/google_shared.rs` | done |
| `src/api/google-vertex.lazy.ts` | `src/ai/providers/apis.rs` | done |
| `src/api/google-vertex.ts` | `src/ai/api/google_vertex.rs` | done |
| `src/api/lazy.ts` | `src/ai/providers/apis.rs` | done (lazy loading collapses to direct dispatch; the Node module-registry probe in `test/lazy-module-load.test.ts` has no Rust equivalent — disposition recorded with `src/providers/all.ts`) |
| `src/api/mistral-conversations.lazy.ts` | `src/ai/providers/apis.rs` | done |
| `src/api/mistral-conversations.ts` | `src/ai/api/mistral_conversations.rs` | done |
| `src/api/openai-codex-responses.lazy.ts` | `src/ai/providers/apis.rs` | done |
| `src/api/openai-codex-responses.ts` | `src/ai/api/openai_codex_responses.rs` | done (SSE transport; WebSocket transport + zstd compression deferred, see note) |
| `src/api/openai-completions.lazy.ts` | `src/ai/providers/apis.rs` | done |
| `src/api/openai-completions.ts` | `src/ai/api/openai_completions.rs` | done |
| `src/api/openai-prompt-cache.ts` | `src/ai/api/openai_completions.rs` | done |
| `src/api/openai-responses-shared.ts` | `src/ai/api/openai_responses_shared.rs` | done |
| `src/api/openai-responses.lazy.ts` | `src/ai/providers/apis.rs` | done |
| `src/api/openai-responses.ts` | `src/ai/api/openai_responses.rs` | done |
| `src/api/openrouter-images.lazy.ts` | `src/ai/providers/builtin.rs` | done (direct dispatch adapter) |
| `src/api/openrouter-images.ts` | `src/ai/api/openrouter_images.rs` | done (transport cannot observe the abort token; a pre-flight cancellation check replaces OpenAI-SDK signal handling) |
| `src/api/pi-messages.lazy.ts` | `src/ai/providers/apis.rs` | done |
| `src/api/pi-messages.ts` | `src/ai/api/pi_messages.rs` | done |
| `src/api/simple-options.ts` | `src/ai/api/simple-options.rs` | done |
| `src/api/transform-messages.ts` | `src/ai/api/transform-messages.rs` | done |
| `src/auth/context.ts` | `src/ai/auth/context.rs` | done |
| `src/auth/credential-store.ts` | `src/ai/auth/credential_store.rs` | done |
| `src/auth/helpers.ts` | `src/ai/auth/helpers.rs` | done |
| `src/auth/oauth/anthropic.ts` | `src/ai/auth/oauth/anthropic.rs` | done (loopback callback over tokio TcpListener; client id inlined decoded) |
| `src/auth/oauth/device-code.ts` | `src/ai/auth/oauth/device_code.rs` | done (tokio time drives the fake-timer tests deterministically) |
| `src/auth/oauth/github-copilot.ts` | `src/ai/auth/oauth/github_copilot.rs` | done (client id inlined decoded; rate-limit budget uses injectable clock) |
| `src/auth/oauth/kimi-coding.ts` | `src/ai/auth/oauth/kimi_coding.rs` | done (injectable transport/clock/env; tokio timeout reproduces the 30s request signal) |
| `src/auth/oauth/load.ts` | `src/ai/auth/oauth/load.rs` | done (static linking replaces dynamic imports; bundled-loader registration is a bundler concern with no counterpart) |
| `src/auth/oauth/oauth-page.ts` | `src/ai/auth/oauth/oauth_page.rs` | done |
| `src/auth/oauth/openai-codex.ts` | `src/ai/auth/oauth/openai_codex.rs` | done (browser callback over tokio TcpListener; `accountId` rides the credential extension map) |
| `src/auth/oauth/openrouter.ts` | `src/ai/auth/oauth/openrouter.rs` | done (loopback callback over tokio TcpListener; callback path uses UUIDv7 in place of crypto.randomUUID) |
| `src/auth/oauth/pkce.ts` | `src/ai/auth/oauth/pkce.rs` | done |
| `src/auth/oauth/radius.ts` | `src/ai/auth/oauth/radius.rs` | done (loopback callback over tokio TcpListener; credential `scope` rides the extension map) |
| `src/auth/oauth/xai.ts` | `src/ai/auth/oauth/xai.rs` | done (injectable transport + clock replace global fetch and `vi.setSystemTime`) |
| `src/auth/resolve.ts` | `src/ai/auth/resolve.rs` | done |
| `src/auth/types.ts` | `src/ai/auth/types.rs` | done |
| `src/bedrock-provider.ts` | | pending |
| `src/bun-oauth.ts` | | pending |
| `src/cli.ts` | | pending |
| `src/compat.ts` | | pending |
| `src/compat/extension-oauth-types.ts` | `src/ai/compat.rs` | done |
| `src/env-api-keys.ts` | `src/ai/env_api_keys.rs` | done |
| `src/image-models.generated.ts` | `src/ai/models_generated.rs (embedded with models catalog)` | done |
| `src/image-models.ts` | `src/ai/models_generated.rs` | done (`image_model`/`image_provider_ids`/`image_models_for_provider`) |
| `src/images-api-registry.ts` | `src/ai/images.rs` | done (static dispatch; lazy module loading is a compile-time no-op in Rust) |
| `src/images-models.ts` | `src/ai/images_models.rs` | done |
| `src/images.ts` | `src/ai/images.rs` | done |
| `src/index.ts` | | pending |
| `src/legacy-api-aliases.ts` | | pending |
| `src/model-catalog.ts` | `src/ai/model_catalog.rs` | done (identity helper; TS generics are compile-time only) |
| `src/models-store.ts` | `src/ai/models_store.rs` | pending |
| `src/models.generated.ts` | `src/ai/models_generated.rs (+ src/ai/data/models.generated.json via scripts/oracle/export-model-catalog.mts)` | done |
| `src/models.ts` | `src/ai/models.rs` | done |
| `src/oauth.ts` | `src/ai/compat.rs (type re-exports)` | done |
| `src/providers/all.ts` | `src/ai/providers/builtin.rs` | done (the amazon-bedrock factory lands with the bedrock converse-stream module; noted in the module) |
| `src/providers/amazon-bedrock.models.ts` | | pending |
| `src/providers/amazon-bedrock.ts` | | pending |
| `src/providers/ant-ling.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/ant-ling.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/anthropic.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/anthropic.ts` | `src/ai/providers/anthropic.rs` | done |
| `src/providers/azure-openai-responses.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/azure-openai-responses.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/baseten.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/baseten.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/cerebras.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/cerebras.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/cloudflare-ai-gateway.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/cloudflare-ai-gateway.ts` | `src/ai/providers/cloudflare_ai_gateway.rs` | done |
| `src/providers/cloudflare-auth.ts` | `src/ai/providers/cloudflare_auth.rs` | done |
| `src/providers/cloudflare-stream.ts` | `src/ai/providers/cloudflare_stream.rs` | done |
| `src/providers/cloudflare-workers-ai.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/cloudflare-workers-ai.ts` | `src/ai/providers/cloudflare_workers_ai.rs` | done |
| `src/providers/deepseek.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/deepseek.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/faux.ts` | `src/ai/providers/faux.rs` | done |
| `src/providers/fireworks.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/fireworks.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/github-copilot.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/github-copilot.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/google-vertex.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/google-vertex.ts` | `src/ai/providers/google_vertex.rs` | done |
| `src/providers/google.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/google.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/groq.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/groq.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/huggingface.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/huggingface.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/images/register-builtins.ts` | `src/ai/images.rs` | done (openrouter-images registered statically) |
| `src/providers/kimi-coding.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/kimi-coding.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/minimax-cn.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/minimax-cn.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/minimax.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/minimax.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/mistral.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/mistral.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/moonshotai-cn.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/moonshotai-cn.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/moonshotai.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/moonshotai.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/nvidia.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/nvidia.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/openai-codex.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/openai-codex.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/openai.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/openai.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/opencode-go.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/opencode-go.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/opencode.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/opencode.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/openrouter-images.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/openrouter.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/openrouter.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/qwen-token-plan-cn.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/qwen-token-plan-cn.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/qwen-token-plan-individual.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/qwen-token-plan-individual.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/qwen-token-plan.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/qwen-token-plan.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/radius-config.ts` | | pending |
| `src/providers/radius.ts` | `src/ai/providers/radius.rs` | done |
| `src/providers/together.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/together.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/vercel-ai-gateway.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/vercel-ai-gateway.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/xai.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/xai.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/xiaomi-token-plan-ams.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/xiaomi-token-plan-ams.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/xiaomi-token-plan-cn.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/xiaomi-token-plan-cn.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/xiaomi-token-plan-sgp.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/xiaomi-token-plan-sgp.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/xiaomi.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/xiaomi.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/zai-coding-cn.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/zai-coding-cn.ts` | `src/ai/providers/builtin.rs` | done |
| `src/providers/zai.models.ts` | `src/ai/data/models.generated.json` | done (generated catalog; see `src/ai/models_generated.rs`) |
| `src/providers/zai.ts` | `src/ai/providers/builtin.rs` | done |
| `src/session-resources.ts` | `src/ai/session_resources.rs` | done |
| `src/types.ts` | `src/ai/types.rs` | done |
| `src/utils/abort-signals.ts` | `src/ai/utils/abort.rs` | done |
| `src/utils/abort.ts` | `src/ai/utils/abort.rs` | done |
| `src/utils/deferred-tools.ts` | `src/ai/utils/deferred_tools.rs` | done |
| `src/utils/diagnostics.ts` | `src/ai/utils/diagnostics.rs` | done |
| `src/utils/error-body.ts` | `src/ai/utils/error_body.rs` | done |
| `src/utils/estimate.ts` | `src/ai/utils/estimate.rs` | done |
| `src/utils/event-stream.ts` | `src/ai/utils/event_stream.rs` | done |
| `src/utils/hash.ts` | `src/ai/utils/text.rs (short_hash)` | done |
| `src/utils/headers.ts` | `src/ai/utils/headers.rs` | done |
| `src/utils/json-parse.ts` | `src/ai/utils/json_parse.rs` | done |
| `src/utils/node-http-proxy.ts` | `src/ai/utils/node_http_proxy.rs` | done |
| `src/utils/overflow.ts` | `src/ai/utils/overflow.rs` | done |
| `src/utils/pi-user-agent.ts` | `src/ai/session_resources.rs (get_pi_user_agent)` | done |
| `src/utils/provider-env.ts` | `src/ai/utils/provider_env.rs` | done |
| `src/utils/provider-retry.ts` | `src/ai/utils/provider_retry.rs` | done |
| `src/utils/retry.ts` | `src/ai/utils/retry.rs` | done |
| `src/utils/sanitize-unicode.ts` | `src/ai/utils/sanitize_unicode.rs` | done |
| `src/utils/sleep.ts` | `src/ai/utils/abort.rs (abortable_sleep)` | done |
| `src/utils/text.ts` | `src/ai/utils/text.rs` | done |
| `src/utils/typebox-helpers.ts` | `folded into tool schema handling (validation.rs); StringEnum is a schema-shape helper` | done |
| `src/utils/uuid.ts` | `src/ai/utils/uuid.rs` | done |
| `src/utils/validation.ts` | `src/ai/utils/validation.rs` | done |

### Tests

| TypeScript test | Rust test | Status |
|---|---|---|
| `test/abort.test.ts` | | pending |
| `test/anthropic-adaptive-thinking-models.test.ts` | | pending |
| `test/anthropic-auth-token.test.ts` | `tests/ai_anthropic_auth_token.rs` | done (SDK-mock cases assert on the captured HTTP request instead of SDK constructor options) |
| `test/anthropic-cache-write-1h-cost.test.ts` | | pending |
| `test/anthropic-eager-tool-input-compat.test.ts` | | pending |
| `test/anthropic-eager-tool-input-e2e.test.ts` | | pending |
| `test/anthropic-empty-thinking-signature-compat.test.ts` | | pending |
| `test/anthropic-force-adaptive-thinking.test.ts` | | pending |
| `test/anthropic-long-cache-retention-e2e.test.ts` | | pending |
| `test/anthropic-oauth.test.ts` | `tests/ai_oauth_anthropic.rs` | done |
| `test/anthropic-opus-4-8-smoke.test.ts` | | pending |
| `test/anthropic-sse-parsing.test.ts` | | pending |
| `test/anthropic-temperature-compat.test.ts` | | pending |
| `test/anthropic-thinking-disable.test.ts` | | pending |
| `test/anthropic-tool-name-normalization.test.ts` | | pending |
| `test/azure-openai-base-url.test.ts` | | pending |
| `test/azure-openai-responses-reasoning-replay.test.ts` | | pending |
| `test/azure-openai-tool-choice.test.ts` | | pending |
| `test/azure-utils.ts` | | pending |
| `test/baseten-models.test.ts` | | pending |
| `test/bedrock-convert-messages.test.ts` | `tests/ai_bedrock_stream.rs` | done (the two unknown-content-block cases are N/A: Rust's closed content enums cannot carry unknown block types) |
| `test/bedrock-credentials.test.ts` | | pending |
| `test/bedrock-custom-headers.test.ts` | | pending |
| `test/bedrock-endpoint-resolution.test.ts` | | pending |
| `test/bedrock-error-metadata.test.ts` | `tests/ai_bedrock_stream.rs` | done |
| `test/bedrock-models.test.ts` | | pending |
| `test/bedrock-raw-stop-reason.test.ts` | `tests/ai_bedrock_stream.rs` | done |
| `test/bedrock-redacted-reasoning.test.ts` | `tests/ai_bedrock_stream.rs` | done |
| `test/bedrock-response-headers.test.ts` | | pending |
| `test/bedrock-thinking-payload.test.ts` | `tests/ai_bedrock_stream.rs` | done (credentials-gated E2E case skips like the TS `describe.skipIf`; payload captured via onPayload with an aborted signal instead of a thrown capture) |
| `test/bedrock-utils.ts` | | pending |
| `test/cache-retention.test.ts` | | pending |
| `test/cloudflare-gateway-binding.test.ts` | `tests/ai_cloudflare_gateway_binding.rs` | done (Request/signal-specific cases documented as N/A in the test header) |
| `test/cloudflare-stream.test.ts` | `tests/ai_cloudflare_stream.rs` | done (third case covers the TS `??` placeholder-fallback branch) |
| `test/cloudflare-utils.ts` | | pending (live-credential helper; lands with the live cloudflare provider tests) |
| `test/codex-websocket-cached-probe.ts` | | pending |
| `test/compat-env.test.ts` | | pending |
| `test/constrained-sampling.test.ts` | | pending |
| `test/context-estimate.test.ts` | | pending |
| `test/context-overflow.test.ts` | | pending |
| `test/cross-provider-handoff.test.ts` | | pending |
| `test/deferred-tools.test.ts` | | pending |
| `test/empty.test.ts` | | pending |
| `test/env-api-keys.test.ts` | | pending |
| `test/error-body.test.ts` | | pending |
| `test/faux-provider.test.ts` | | pending |
| `test/fetch-option.test.ts` | | pending |
| `test/fireworks-models.test.ts` | | pending |
| `test/generate-models-strict.test.ts` | | pending |
| `test/github-copilot-anthropic.test.ts` | | pending |
| `test/github-copilot-oauth.test.ts` | `tests/ai_oauth_github_copilot.rs` | done (Models getAvailable/store halves and the login-budget case included; the budget case rebuilds the provider with a scripted flow, mirroring the TS global-fetch stub) |
| `test/google-raw-stop-reason.test.ts` | | pending |
| `test/google-shared-convert-tools.test.ts` | | pending |
| `test/google-shared-gemini3-unsigned-tool-call.test.ts` | | pending |
| `test/google-shared-image-tool-result-routing.test.ts` | | pending |
| `test/google-shared-retry.test.ts` | | pending |
| `test/google-shared-signed-empty-blocks.test.ts` | | pending |
| `test/google-thinking-disable.test.ts` | | pending |
| `test/google-thinking-level-map.test.ts` | | pending |
| `test/google-thinking-signature.test.ts` | | pending |
| `test/google-vertex-api-key-resolution.test.ts` | | pending |
| `test/image-model-data.test.ts` | | n/a — tests the TS oracle generator script (`scripts/generate-image-models.ts`); the generated catalog it produces is committed via `scripts/oracle/export-model-catalog.mts` |
| `test/image-tool-result.test.ts` | | pending |
| `test/images-models.test.ts` | `tests/ai_images_models.rs` (+ `tests/ai_providers.rs` for the builtinImagesModels case) | done |
| `test/images.test.ts` | | pending (live E2E, gated on OPENROUTER_API_KEY; offline surface covered by `tests/ai_openrouter_images.rs`) |
| `test/interleaved-thinking.test.ts` | | pending |
| `test/kimi-coding-oauth.test.ts` | `tests/ai_oauth_kimi_coding.rs` | done |
| `test/lax-message-content.test.ts` | | pending |
| `test/lazy-module-load.test.ts` | | pending |
| `test/max-thinking.test.ts` | | pending |
| `test/mistral-http-transport.test.ts` | | pending |
| `test/mistral-raw-stop-reason.test.ts` | | pending |
| `test/mistral-reasoning-mode.test.ts` | | pending |
| `test/mistral-tool-schema.test.ts` | | pending |
| `test/model-catalog-types.test.ts` | | pending |
| `test/model-data-validation.test.ts` | | pending |
| `test/models-runtime.test.ts` | | pending |
| `test/node-http-proxy.test.ts` | | pending |
| `test/oauth-auth.test.ts` | `tests/ai_oauth_auth.rs` | done (Models.getAuth lazy-chain cases included; module-barrel introspection is a TypeScript namespace concern) |
| `test/oauth-device-code.test.ts` | `tests/ai_oauth_device_code.rs` | done |
| `test/oauth.ts` | | pending |
| `test/openai-codex-cache-affinity-e2e.test.ts` | | pending |
| `test/openai-codex-oauth.test.ts` | `tests/ai_oauth_openai_codex.rs` | done |
| `test/openai-codex-stream.test.ts` | | pending |
| `test/openai-completions-cache-control-format.test.ts` | | pending |
| `test/openai-completions-empty-tools.test.ts` | | pending |
| `test/openai-completions-prompt-cache.test.ts` | | pending |
| `test/openai-completions-raw-stop-reason.test.ts` | | pending |
| `test/openai-completions-reasoning-details.test.ts` | | pending |
| `test/openai-completions-response-model.test.ts` | | pending |
| `test/openai-completions-retry.test.ts` | | pending |
| `test/openai-completions-thinking-as-text.test.ts` | | pending |
| `test/openai-completions-thinking-token-budget.test.ts` | | pending |
| `test/openai-completions-tool-choice.test.ts` | | pending |
| `test/openai-completions-tool-result-images.test.ts` | | pending |
| `test/openai-responses-cache-affinity-e2e.test.ts` | | pending |
| `test/openai-responses-compat.test.ts` | | pending |
| `test/openai-responses-empty-tool-result.test.ts` | | pending |
| `test/openai-responses-foreign-toolcall-id.test.ts` | | pending |
| `test/openai-responses-message-id.test.ts` | | pending |
| `test/openai-responses-namespace.test.ts` | | pending |
| `test/openai-responses-partial-json-cleanup.test.ts` | | pending |
| `test/openai-responses-reasoning-replay-e2e.test.ts` | | pending |
| `test/openai-responses-terminal-event.test.ts` | | pending |
| `test/openai-responses-tool-result-images.test.ts` | | pending |
| `test/openrouter-cache-control-models.test.ts` | | pending |
| `test/openrouter-cache-write-repro.test.ts` | | pending |
| `test/openrouter-images.test.ts` | `tests/ai_openrouter_images.rs` | done (mock at the `HttpFetch` transport replaces the OpenAI-SDK mock) |
| `test/openrouter-oauth.test.ts` | `tests/ai_oauth_openrouter.rs` | done |
| `test/openrouter-reasoning-options.test.ts` | | pending |
| `test/overflow.test.ts` | | pending |
| `test/pi-messages.test.ts` | | pending |
| `test/provider-error-body-passthrough.test.ts` | | pending |
| `test/provider-error-body-regression.test.ts` | | pending |
| `test/provider-retry.test.ts` | | pending |
| `test/providers.test.ts` | `tests/ai_providers.rs` (+ `tests/ai_models.rs` for the dispatch-error case) | partial (all non-bedrock cases ported: catalog, auth precedence, getAuth, dispatch, env merge, deferred options, faux streams, dynamic-refresh supersede; the two bedrock cases land with the bedrock module) |
| `test/qwen-token-plan-models.test.ts` | | pending |
| `test/radius-oauth.test.ts` | `tests/ai_oauth_radius.rs` | done |
| `test/reasoning-options.test.ts` | | pending |
| `test/responseid.test.ts` | | pending |
| `test/retry.test.ts` | | pending |
| `test/sampling-options.test.ts` | | pending |
| `test/scratch.ts` | | exception: scratch file, not a test |
| `test/stream.test.ts` | | pending |
| `test/supports-xhigh.test.ts` | | pending |
| `test/telemetry-options.test.ts` | | pending |
| `test/text.test.ts` | | pending |
| `test/together-models.test.ts` | | pending |
| `test/tokens.test.ts` | | pending |
| `test/tool-call-id-normalization.test.ts` | | pending |
| `test/tool-call-without-result.test.ts` | | pending |
| `test/total-tokens.test.ts` | | pending |
| `test/transform-messages-copilot-openai-to-anthropic.test.ts` | | pending |
| `test/unicode-surrogate.test.ts` | | pending |
| `test/uuid.test.ts` | | pending |
| `test/validation.test.ts` | | pending |
| `test/xai-oauth.test.ts` | `tests/ai_oauth_xai.rs` | done |
| `test/xai-responses.test.ts` | | pending |
| `test/xhigh.test.ts` | | pending |
| `test/xiaomi-models.test.ts` | | pending |
| `test/xiaomi-token-plan-ams-anthropic-empty-signature-smoke.test.ts` | | pending |
| `test/zai-coding-plan-models.test.ts` | | pending |
| `test/zen.test.ts` | | pending |

### Documented deviations and deferrals

- `src/api/openai-codex-responses.ts` is ported over the SSE transport
  (request shape, URL resolution, retry policy, Codex event mapping and
  error taxonomy are 1:1). The optional WebSocket transport
  (`responses_websockets=2026-02-06` beta) with session cache/SSE fallback,
  and zstd request-body compression via `node:zlib`, are runtime transport
  optimizations without a Rust counterpart yet; the SSE path is the
  protocol-correct fallback the TS adapter also uses.

- `src/cli.ts` (Node CLI for OAuth login) and `src/bun-oauth.ts` (Bun
  credential export) are runtime entry points for the Node/Bun
  environments; they depend on the OAuth provider flows and are ported with
  them (M3). `bun-oauth.ts`'s Bun-specific export has no Rust counterpart.
- `src/compat.ts` and `src/legacy-api-aliases.ts` are re-export shims over
  the API implementations; they land with the providers (M3).
- `index.ts` public re-exports are mirrored as they land module by module.
- The HTTP proxy resolver returns the proxy URL string; TypeScript returns a
  `URL` object (whose `toString` adds a trailing slash). Transport
  construction consumes the string form.

### Additional Rust modules

| Rust module | Purpose | Status |
|---|---|---|
| `src/ai/utils/http.rs` | Transport abstraction behind the `fetch` option (`FetchFunction`) | done (trait) |
| `src/ai/utils/reqwest_fetch.rs` | reqwest-backed default transport with node-http-proxy resolution | done |
| `src/ai/utils/sse.rs` | SSE decoder + async event stream (ported from the shared reader in anthropic-messages.ts) | done |

## pi-agent-core

### Source modules

| TypeScript source | Rust module | Status |
|---|---|---|
| `src/agent-loop.ts` | | pending |
| `src/agent.ts` | | pending |
| `src/harness/agent-harness.ts` | | pending |
| `src/harness/compaction/branch-summarization.ts` | | pending |
| `src/harness/compaction/compaction.ts` | | pending |
| `src/harness/compaction/utils.ts` | | pending |
| `src/harness/env/nodejs.ts` | | pending |
| `src/harness/events.ts` | | pending |
| `src/harness/messages.ts` | | pending |
| `src/harness/prompt-templates.ts` | | pending |
| `src/harness/reducer.ts` | | pending |
| `src/harness/result.ts` | | pending |
| `src/harness/session/context.ts` | | pending |
| `src/harness/session/index.ts` | | pending |
| `src/harness/session/jsonl.ts` | | pending |
| `src/harness/session/jsonl/codec.ts` | | pending |
| `src/harness/session/jsonl/errors.ts` | | pending |
| `src/harness/session/jsonl/repo.ts` | | pending |
| `src/harness/session/jsonl/storage.ts` | | pending |
| `src/harness/session/jsonl/types.ts` | | pending |
| `src/harness/session/memory.ts` | | pending |
| `src/harness/session/session.ts` | | pending |
| `src/harness/session/state.ts` | | pending |
| `src/harness/session/testing/conformance.ts` | | pending |
| `src/harness/session/testing/index.ts` | | pending |
| `src/harness/session/testing/types.ts` | | pending |
| `src/harness/session/types.ts` | | pending |
| `src/harness/skills.ts` | | pending |
| `src/harness/system-prompt.ts` | | pending |
| `src/harness/telemetry.ts` | | pending |
| `src/harness/tools/bash.ts` | | pending |
| `src/harness/tools/edit-diff.ts` | | pending |
| `src/harness/tools/edit.ts` | | pending |
| `src/harness/tools/file-mutation-queue.ts` | | pending |
| `src/harness/tools/image.ts` | | pending |
| `src/harness/tools/index.ts` | | pending |
| `src/harness/tools/path-utils.ts` | | pending |
| `src/harness/tools/read.ts` | | pending |
| `src/harness/tools/tool-context.ts` | | pending |
| `src/harness/tools/write.ts` | | pending |
| `src/harness/types.ts` | | pending |
| `src/harness/utils/shell-output.ts` | | pending |
| `src/harness/utils/truncate.ts` | | pending |
| `src/index.ts` | | pending |
| `src/node.ts` | | pending |
| `src/proxy.ts` | | pending |
| `src/search/index.ts` | | pending |
| `src/search/scanning.ts` | | pending |
| `src/stream-fn.ts` | | pending |
| `src/types.ts` | `src/ai/types.rs` | done |

### Tests

| TypeScript test | Rust test | Status |
|---|---|---|
| `test/agent-loop.test.ts` | | pending |
| `test/agent.test.ts` | | pending |
| `test/e2e.test.ts` | | pending |
| `test/proxy.test.ts` | | pending |

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
| `src/api/anthropic-messages.lazy.ts` | | pending |
| `src/api/anthropic-messages.ts` | | pending |
| `src/api/azure-openai-responses.lazy.ts` | | pending |
| `src/api/azure-openai-responses.ts` | | pending |
| `src/api/bedrock-converse-stream.lazy.ts` | | pending |
| `src/api/bedrock-converse-stream.ts` | | pending |
| `src/api/cloudflare-gateway-binding.ts` | | pending |
| `src/api/cloudflare.ts` | | pending |
| `src/api/constrained-sampling.ts` | | pending |
| `src/api/github-copilot-headers.ts` | | pending |
| `src/api/google-generative-ai.lazy.ts` | | pending |
| `src/api/google-generative-ai.ts` | | pending |
| `src/api/google-shared.ts` | | pending |
| `src/api/google-vertex.lazy.ts` | | pending |
| `src/api/google-vertex.ts` | | pending |
| `src/api/lazy.ts` | | pending |
| `src/api/mistral-conversations.lazy.ts` | | pending |
| `src/api/mistral-conversations.ts` | | pending |
| `src/api/openai-codex-responses.lazy.ts` | | pending |
| `src/api/openai-codex-responses.ts` | | pending |
| `src/api/openai-completions.lazy.ts` | | pending |
| `src/api/openai-completions.ts` | | pending |
| `src/api/openai-prompt-cache.ts` | | pending |
| `src/api/openai-responses-shared.ts` | | pending |
| `src/api/openai-responses.lazy.ts` | | pending |
| `src/api/openai-responses.ts` | | pending |
| `src/api/openrouter-images.lazy.ts` | | pending |
| `src/api/openrouter-images.ts` | | pending |
| `src/api/pi-messages.lazy.ts` | | pending |
| `src/api/pi-messages.ts` | | pending |
| `src/api/simple-options.ts` | | pending |
| `src/api/transform-messages.ts` | | pending |
| `src/auth/context.ts` | | pending |
| `src/auth/credential-store.ts` | | pending |
| `src/auth/helpers.ts` | | pending |
| `src/auth/oauth/anthropic.ts` | | pending |
| `src/auth/oauth/device-code.ts` | | pending |
| `src/auth/oauth/github-copilot.ts` | | pending |
| `src/auth/oauth/kimi-coding.ts` | | pending |
| `src/auth/oauth/load.ts` | | pending |
| `src/auth/oauth/oauth-page.ts` | | pending |
| `src/auth/oauth/openai-codex.ts` | | pending |
| `src/auth/oauth/openrouter.ts` | | pending |
| `src/auth/oauth/pkce.ts` | | pending |
| `src/auth/oauth/radius.ts` | | pending |
| `src/auth/oauth/xai.ts` | | pending |
| `src/auth/resolve.ts` | | pending |
| `src/auth/types.ts` | | pending |
| `src/bedrock-provider.ts` | | pending |
| `src/bun-oauth.ts` | | pending |
| `src/cli.ts` | | pending |
| `src/compat.ts` | | pending |
| `src/compat/extension-oauth-types.ts` | | pending |
| `src/env-api-keys.ts` | | pending |
| `src/image-models.generated.ts` | | pending |
| `src/image-models.ts` | | pending |
| `src/images-api-registry.ts` | | pending |
| `src/images-models.ts` | | pending |
| `src/images.ts` | | pending |
| `src/index.ts` | | pending |
| `src/legacy-api-aliases.ts` | | pending |
| `src/model-catalog.ts` | | pending |
| `src/models-store.ts` | | pending |
| `src/models.generated.ts` | | pending |
| `src/models.ts` | | pending |
| `src/oauth.ts` | | pending |
| `src/providers/all.ts` | | pending |
| `src/providers/amazon-bedrock.models.ts` | | pending |
| `src/providers/amazon-bedrock.ts` | | pending |
| `src/providers/ant-ling.models.ts` | | pending |
| `src/providers/ant-ling.ts` | | pending |
| `src/providers/anthropic.models.ts` | | pending |
| `src/providers/anthropic.ts` | | pending |
| `src/providers/azure-openai-responses.models.ts` | | pending |
| `src/providers/azure-openai-responses.ts` | | pending |
| `src/providers/baseten.models.ts` | | pending |
| `src/providers/baseten.ts` | | pending |
| `src/providers/cerebras.models.ts` | | pending |
| `src/providers/cerebras.ts` | | pending |
| `src/providers/cloudflare-ai-gateway.models.ts` | | pending |
| `src/providers/cloudflare-ai-gateway.ts` | | pending |
| `src/providers/cloudflare-auth.ts` | | pending |
| `src/providers/cloudflare-stream.ts` | | pending |
| `src/providers/cloudflare-workers-ai.models.ts` | | pending |
| `src/providers/cloudflare-workers-ai.ts` | | pending |
| `src/providers/deepseek.models.ts` | | pending |
| `src/providers/deepseek.ts` | | pending |
| `src/providers/faux.ts` | | pending |
| `src/providers/fireworks.models.ts` | | pending |
| `src/providers/fireworks.ts` | | pending |
| `src/providers/github-copilot.models.ts` | | pending |
| `src/providers/github-copilot.ts` | | pending |
| `src/providers/google-vertex.models.ts` | | pending |
| `src/providers/google-vertex.ts` | | pending |
| `src/providers/google.models.ts` | | pending |
| `src/providers/google.ts` | | pending |
| `src/providers/groq.models.ts` | | pending |
| `src/providers/groq.ts` | | pending |
| `src/providers/huggingface.models.ts` | | pending |
| `src/providers/huggingface.ts` | | pending |
| `src/providers/images/register-builtins.ts` | | pending |
| `src/providers/kimi-coding.models.ts` | | pending |
| `src/providers/kimi-coding.ts` | | pending |
| `src/providers/minimax-cn.models.ts` | | pending |
| `src/providers/minimax-cn.ts` | | pending |
| `src/providers/minimax.models.ts` | | pending |
| `src/providers/minimax.ts` | | pending |
| `src/providers/mistral.models.ts` | | pending |
| `src/providers/mistral.ts` | | pending |
| `src/providers/moonshotai-cn.models.ts` | | pending |
| `src/providers/moonshotai-cn.ts` | | pending |
| `src/providers/moonshotai.models.ts` | | pending |
| `src/providers/moonshotai.ts` | | pending |
| `src/providers/nvidia.models.ts` | | pending |
| `src/providers/nvidia.ts` | | pending |
| `src/providers/openai-codex.models.ts` | | pending |
| `src/providers/openai-codex.ts` | | pending |
| `src/providers/openai.models.ts` | | pending |
| `src/providers/openai.ts` | | pending |
| `src/providers/opencode-go.models.ts` | | pending |
| `src/providers/opencode-go.ts` | | pending |
| `src/providers/opencode.models.ts` | | pending |
| `src/providers/opencode.ts` | | pending |
| `src/providers/openrouter-images.ts` | | pending |
| `src/providers/openrouter.models.ts` | | pending |
| `src/providers/openrouter.ts` | | pending |
| `src/providers/qwen-token-plan-cn.models.ts` | | pending |
| `src/providers/qwen-token-plan-cn.ts` | | pending |
| `src/providers/qwen-token-plan-individual.models.ts` | | pending |
| `src/providers/qwen-token-plan-individual.ts` | | pending |
| `src/providers/qwen-token-plan.models.ts` | | pending |
| `src/providers/qwen-token-plan.ts` | | pending |
| `src/providers/radius-config.ts` | | pending |
| `src/providers/radius.ts` | | pending |
| `src/providers/together.models.ts` | | pending |
| `src/providers/together.ts` | | pending |
| `src/providers/vercel-ai-gateway.models.ts` | | pending |
| `src/providers/vercel-ai-gateway.ts` | | pending |
| `src/providers/xai.models.ts` | | pending |
| `src/providers/xai.ts` | | pending |
| `src/providers/xiaomi-token-plan-ams.models.ts` | | pending |
| `src/providers/xiaomi-token-plan-ams.ts` | | pending |
| `src/providers/xiaomi-token-plan-cn.models.ts` | | pending |
| `src/providers/xiaomi-token-plan-cn.ts` | | pending |
| `src/providers/xiaomi-token-plan-sgp.models.ts` | | pending |
| `src/providers/xiaomi-token-plan-sgp.ts` | | pending |
| `src/providers/xiaomi.models.ts` | | pending |
| `src/providers/xiaomi.ts` | | pending |
| `src/providers/zai-coding-cn.models.ts` | | pending |
| `src/providers/zai-coding-cn.ts` | | pending |
| `src/providers/zai.models.ts` | | pending |
| `src/providers/zai.ts` | | pending |
| `src/session-resources.ts` | | pending |
| `src/types.ts` | `src/ai/types.rs` | done |
| `src/utils/abort-signals.ts` | `src/ai/utils/abort.rs` | done |
| `src/utils/abort.ts` | `src/ai/utils/abort.rs` | done |
| `src/utils/deferred-tools.ts` | `src/ai/utils/deferred_tools.rs` | done |
| `src/utils/diagnostics.ts` | `src/ai/utils/diagnostics.rs` | done |
| `src/utils/error-body.ts` | | pending |
| `src/utils/estimate.ts` | `src/ai/utils/estimate.rs` | done |
| `src/utils/event-stream.ts` | `src/ai/utils/event_stream.rs` | done |
| `src/utils/hash.ts` | `src/ai/utils/text.rs (short_hash)` | done |
| `src/utils/headers.ts` | `src/ai/utils/headers.rs` | done |
| `src/utils/json-parse.ts` | `src/ai/utils/json_parse.rs` | done |
| `src/utils/node-http-proxy.ts` | | pending |
| `src/utils/overflow.ts` | `src/ai/utils/overflow.rs` | done |
| `src/utils/pi-user-agent.ts` | | pending |
| `src/utils/provider-env.ts` | `src/ai/utils/provider_env.rs` | done |
| `src/utils/provider-retry.ts` | | pending |
| `src/utils/retry.ts` | | pending |
| `src/utils/sanitize-unicode.ts` | `src/ai/utils/sanitize_unicode.rs` | done |
| `src/utils/sleep.ts` | `src/ai/utils/abort.rs (abortable_sleep)` | done |
| `src/utils/text.ts` | `src/ai/utils/text.rs` | done |
| `src/utils/typebox-helpers.ts` | | pending |
| `src/utils/uuid.ts` | `src/ai/utils/uuid.rs` | done |
| `src/utils/validation.ts` | | pending |

### Tests

| TypeScript test | Rust test | Status |
|---|---|---|
| `test/abort.test.ts` | | pending |
| `test/anthropic-adaptive-thinking-models.test.ts` | | pending |
| `test/anthropic-auth-token.test.ts` | | pending |
| `test/anthropic-cache-write-1h-cost.test.ts` | | pending |
| `test/anthropic-eager-tool-input-compat.test.ts` | | pending |
| `test/anthropic-eager-tool-input-e2e.test.ts` | | pending |
| `test/anthropic-empty-thinking-signature-compat.test.ts` | | pending |
| `test/anthropic-force-adaptive-thinking.test.ts` | | pending |
| `test/anthropic-long-cache-retention-e2e.test.ts` | | pending |
| `test/anthropic-oauth.test.ts` | | pending |
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
| `test/bedrock-convert-messages.test.ts` | | pending |
| `test/bedrock-credentials.test.ts` | | pending |
| `test/bedrock-custom-headers.test.ts` | | pending |
| `test/bedrock-endpoint-resolution.test.ts` | | pending |
| `test/bedrock-error-metadata.test.ts` | | pending |
| `test/bedrock-models.test.ts` | | pending |
| `test/bedrock-raw-stop-reason.test.ts` | | pending |
| `test/bedrock-redacted-reasoning.test.ts` | | pending |
| `test/bedrock-response-headers.test.ts` | | pending |
| `test/bedrock-thinking-payload.test.ts` | | pending |
| `test/bedrock-utils.ts` | | pending |
| `test/cache-retention.test.ts` | | pending |
| `test/cloudflare-gateway-binding.test.ts` | | pending |
| `test/cloudflare-stream.test.ts` | | pending |
| `test/cloudflare-utils.ts` | | pending |
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
| `test/github-copilot-oauth.test.ts` | | pending |
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
| `test/image-model-data.test.ts` | | pending |
| `test/image-tool-result.test.ts` | | pending |
| `test/images-models.test.ts` | | pending |
| `test/images.test.ts` | | pending |
| `test/interleaved-thinking.test.ts` | | pending |
| `test/kimi-coding-oauth.test.ts` | | pending |
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
| `test/oauth-auth.test.ts` | | pending |
| `test/oauth-device-code.test.ts` | | pending |
| `test/oauth.ts` | | pending |
| `test/openai-codex-cache-affinity-e2e.test.ts` | | pending |
| `test/openai-codex-oauth.test.ts` | | pending |
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
| `test/openrouter-images.test.ts` | | pending |
| `test/openrouter-oauth.test.ts` | | pending |
| `test/openrouter-reasoning-options.test.ts` | | pending |
| `test/overflow.test.ts` | | pending |
| `test/pi-messages.test.ts` | | pending |
| `test/provider-error-body-passthrough.test.ts` | | pending |
| `test/provider-error-body-regression.test.ts` | | pending |
| `test/provider-retry.test.ts` | | pending |
| `test/providers.test.ts` | | pending |
| `test/qwen-token-plan-models.test.ts` | | pending |
| `test/radius-oauth.test.ts` | | pending |
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
| `test/xai-oauth.test.ts` | | pending |
| `test/xai-responses.test.ts` | | pending |
| `test/xhigh.test.ts` | | pending |
| `test/xiaomi-models.test.ts` | | pending |
| `test/xiaomi-token-plan-ams-anthropic-empty-signature-smoke.test.ts` | | pending |
| `test/zai-coding-plan-models.test.ts` | | pending |
| `test/zen.test.ts` | | pending |

### Additional Rust modules

| Rust module | Purpose | Status |
|---|---|---|
| `src/ai/utils/http.rs` | Transport abstraction behind the `fetch` option (`FetchFunction`); reqwest-backed default lands with the providers | done (trait) |

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

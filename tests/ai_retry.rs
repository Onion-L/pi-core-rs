//! Port of `pi-core/ai/test/retry.test.ts` and
//! `pi-core/ai/test/provider-retry.test.ts`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use pi_core::ai::providers::faux::{FauxContent, FauxMessageOptions, faux_assistant_message};
use pi_core::ai::types::{AssistantContent, StopReason, TextContent, Usage};
use pi_core::ai::utils::provider_retry::{
    ProviderHttpError, ProviderRetryOptions, retry_provider_request,
};
use pi_core::ai::utils::retry::{
    RetryCallbacks, RetryPolicy, is_retryable_assistant_error, retry_assistant_call,
};
use tokio_util::sync::CancellationToken;

fn error_message(message: &str) -> pi_core::ai::types::AssistantMessage {
    faux_assistant_message(
        "",
        FauxMessageOptions {
            stop_reason: Some(StopReason::Error),
            error_message: Some(message.to_string()),
            ..Default::default()
        },
    )
}

const OPENAI_EXPLICIT_RETRY_MESSAGE: &str = "An error occurred while processing your request. You can retry your request, or contact us through our help center at help.openai.com if the error persists. Please include the request ID req_******** in your message.";
const BEDROCK_EXPLICIT_RETRY_MESSAGE: &str = "{\"message\":\"The system encountered an unexpected error during processing. Try your request again.\"}";
const NVIDIA_NIM_RESOURCE_EXHAUSTED_MESSAGE: &str =
    "ResourceExhausted: Worker local total request limit reached (288/48)";
const BUN_FETCH_SOCKET_CLOSED_MESSAGE: &str = "The socket connection was closed unexpectedly. For more information, pass `verbose: true` in the second argument to fetch()";
const OPENAI_RESPONSES_EARLY_EOF_MESSAGE: &str =
    "OpenAI Responses stream ended before a terminal response event";
const WRAPPED_DNS_LOOKUP_ERROR: &str = "The pending stream has been canceled (caused by: getaddrinfo ENOTFOUND bedrock-runtime.us-east-1.amazonaws.com)";

/// Port of the "provider retry classification" describe block.
#[test]
fn provider_retry_classification() {
    // matches explicit provider retry guidance
    for message in [
        OPENAI_EXPLICIT_RETRY_MESSAGE,
        BEDROCK_EXPLICIT_RETRY_MESSAGE,
        NVIDIA_NIM_RESOURCE_EXHAUSTED_MESSAGE,
    ] {
        assert!(
            is_retryable_assistant_error(&error_message(message)),
            "{message}"
        );
    }

    // matches Bun fetch socket drop wording
    assert!(is_retryable_assistant_error(&error_message(
        BUN_FETCH_SOCKET_CLOSED_MESSAGE
    )));

    // matches upstream request buffer exhaustion wording
    assert!(is_retryable_assistant_error(&error_message(
        "Error: exceeded request buffer limit while retrying upstream"
    )));

    // matches DNS transport failure wording
    for message in [
        WRAPPED_DNS_LOOKUP_ERROR,
        "connect ENOTFOUND api.example.com",
        "EAI_AGAIN api.example.com",
        "getaddrinfo failed for api.example.com",
    ] {
        assert!(
            is_retryable_assistant_error(&error_message(message)),
            "{message}"
        );
    }

    // matches OpenAI Responses streams that end before terminal events
    assert!(is_retryable_assistant_error(&error_message(
        OPENAI_RESPONSES_EARLY_EOF_MESSAGE
    )));

    // keeps provider limit errors non-retryable
    assert!(!is_retryable_assistant_error(&error_message(
        "429 quota exceeded"
    )));

    // classifies assistant error messages
    assert!(is_retryable_assistant_error(&error_message(
        "overloaded_error"
    )));
    assert!(is_retryable_assistant_error(&error_message(
        "524 status code (no body)"
    )));
    // a non-error message is never retryable
    assert!(!is_retryable_assistant_error(&faux_assistant_message(
        "not an error",
        FauxMessageOptions::default()
    )));
}

fn faux_text_message(text: &str) -> pi_core::ai::types::AssistantMessage {
    faux_assistant_message(
        FauxContent::Blocks(vec![AssistantContent::Text(TextContent {
            text: text.to_string(),
            ..Default::default()
        })]),
        FauxMessageOptions::default(),
    )
}

/// Shared async callback recorder standing in for the vitest `vi.fn` mocks.
#[derive(Clone, Default)]
struct EventLog {
    events: Arc<Mutex<Vec<String>>>,
}

impl EventLog {
    fn push(&self, event: String) {
        self.events.lock().unwrap().push(event);
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn retry_assistant_call_returns_successful_response_immediately() {
    let calls = Arc::new(AtomicU32::new(0));
    let enabled = RetryPolicy {
        enabled: true,
        max_retries: 3,
        base_delay_ms: 0,
    };
    let result = retry_assistant_call(
        || {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                faux_text_message("ok")
            }
        },
        Some(&enabled),
        None,
        None,
    )
    .await;

    assert_eq!(
        result.content,
        vec![AssistantContent::Text(TextContent {
            text: "ok".to_string(),
            ..Default::default()
        })]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retry_assistant_call_does_not_retry_an_aborted_message() {
    let calls = Arc::new(AtomicU32::new(0));
    let enabled = RetryPolicy {
        enabled: true,
        max_retries: 3,
        base_delay_ms: 0,
    };
    let scheduled = EventLog::default();
    let log = scheduled.clone();
    let result = retry_assistant_call(
        || {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                faux_assistant_message(
                    "",
                    FauxMessageOptions {
                        stop_reason: Some(StopReason::Aborted),
                        ..Default::default()
                    },
                )
            }
        },
        Some(&enabled),
        None,
        Some(&RetryCallbacks {
            on_retry_scheduled: Some(Box::new(move |attempt, _, _, _| {
                let log = log.clone();
                Box::pin(async move { log.push(format!("retry:{attempt}")) })
            })),
            ..Default::default()
        }),
    )
    .await;

    assert_eq!(result.stop_reason, StopReason::Aborted);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(scheduled.events().is_empty());
}

#[tokio::test]
async fn retry_assistant_call_does_not_retry_non_retryable_errors() {
    let calls = Arc::new(AtomicU32::new(0));
    let enabled = RetryPolicy {
        enabled: true,
        max_retries: 3,
        base_delay_ms: 0,
    };
    let result = retry_assistant_call(
        || {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                error_message("insufficient_quota")
            }
        },
        Some(&enabled),
        None,
        None,
    )
    .await;

    assert_eq!(result.stop_reason, StopReason::Error);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retry_assistant_call_retries_transient_errors_up_to_max_retries() {
    let calls = Arc::new(AtomicU32::new(0));
    let enabled = RetryPolicy {
        enabled: true,
        max_retries: 3,
        base_delay_ms: 0,
    };
    let finished = EventLog::default();
    let log = finished.clone();
    let result = retry_assistant_call(
        || {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                error_message("terminated")
            }
        },
        Some(&enabled),
        None,
        Some(&RetryCallbacks {
            on_retry_finished: Some(Box::new(move |success, attempt, error| {
                let log = log.clone();
                let error = error.map(str::to_string);
                Box::pin(async move {
                    let error = error.as_deref().unwrap_or("").to_string();
                    log.push(format!("finished:{success}:{attempt}:{error}"));
                })
            })),
            ..Default::default()
        }),
    )
    .await;

    assert_eq!(result.stop_reason, StopReason::Error);
    assert_eq!(calls.load(Ordering::SeqCst), 4, "1 initial + 3 retries");
    assert_eq!(
        finished.events(),
        vec!["finished:false:3:terminated".to_string()]
    );
}

#[tokio::test]
async fn retry_assistant_call_stops_retrying_once_a_call_succeeds() {
    let count = Arc::new(AtomicU32::new(0));
    let enabled = RetryPolicy {
        enabled: true,
        max_retries: 3,
        base_delay_ms: 0,
    };
    let finished = EventLog::default();
    let log = finished.clone();
    let result = retry_assistant_call(
        || {
            let count = Arc::clone(&count);
            async move {
                let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                if n < 3 {
                    error_message("terminated")
                } else {
                    faux_text_message("recovered")
                }
            }
        },
        Some(&enabled),
        None,
        Some(&RetryCallbacks {
            on_retry_finished: Some(Box::new(move |success, attempt, error| {
                let log = log.clone();
                let error = error.map(str::to_string);
                Box::pin(async move {
                    let error = error.as_deref().unwrap_or("").to_string();
                    log.push(format!("finished:{success}:{attempt}:{error}"));
                })
            })),
            ..Default::default()
        }),
    )
    .await;

    assert_eq!(
        result.content,
        vec![AssistantContent::Text(TextContent {
            text: "recovered".to_string(),
            ..Default::default()
        })]
    );
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert_eq!(finished.events(), vec!["finished:true:2:".to_string()]);
}

#[tokio::test]
async fn retry_assistant_call_reports_aborted_retried_call_as_unsuccessful() {
    let count = Arc::new(AtomicU32::new(0));
    let enabled = RetryPolicy {
        enabled: true,
        max_retries: 3,
        base_delay_ms: 0,
    };
    let finished = EventLog::default();
    let log = finished.clone();
    let result = retry_assistant_call(
        || {
            let count = Arc::clone(&count);
            async move {
                let n = count.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 1 {
                    error_message("terminated")
                } else {
                    faux_assistant_message(
                        "",
                        FauxMessageOptions {
                            stop_reason: Some(StopReason::Aborted),
                            ..Default::default()
                        },
                    )
                }
            }
        },
        Some(&enabled),
        None,
        Some(&RetryCallbacks {
            on_retry_finished: Some(Box::new(move |success, attempt, error| {
                let log = log.clone();
                let error = error.map(str::to_string);
                Box::pin(async move {
                    let error = error.as_deref().unwrap_or("").to_string();
                    log.push(format!("finished:{success}:{attempt}:{error}"));
                })
            })),
            ..Default::default()
        }),
    )
    .await;

    assert_eq!(result.stop_reason, StopReason::Aborted);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(finished.events(), vec!["finished:false:1:".to_string()]);
}

#[tokio::test]
async fn retry_assistant_call_does_not_retry_when_policy_is_disabled() {
    let calls = Arc::new(AtomicU32::new(0));
    let disabled = RetryPolicy {
        enabled: false,
        max_retries: 3,
        base_delay_ms: 0,
    };
    let result = retry_assistant_call(
        || {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                error_message("terminated")
            }
        },
        Some(&disabled),
        None,
        None,
    )
    .await;

    assert_eq!(result.stop_reason, StopReason::Error);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn retry_assistant_call_emits_attempt_start_after_backoff() {
    let count = Arc::new(AtomicU32::new(0));
    let enabled = RetryPolicy {
        enabled: true,
        max_retries: 3,
        base_delay_ms: 0,
    };
    let log = EventLog::default();
    let produce_log = log.clone();
    let scheduled_log = log.clone();
    let attempt_log = log.clone();

    let result = retry_assistant_call(
        || {
            let produce_log = produce_log.clone();
            let count = Arc::clone(&count);
            async move {
                let n = count.fetch_add(1, Ordering::SeqCst);
                produce_log.push(format!("produce:{n}"));
                if n < 2 {
                    error_message("terminated")
                } else {
                    faux_text_message("recovered")
                }
            }
        },
        Some(&enabled),
        None,
        Some(&RetryCallbacks {
            on_retry_scheduled: Some(Box::new(move |attempt, _, _, _| {
                let scheduled_log = scheduled_log.clone();
                Box::pin(async move { scheduled_log.push(format!("retry:{attempt}")) })
            })),
            on_retry_attempt_start: Some(Box::new(move || {
                let attempt_log = attempt_log.clone();
                Box::pin(async move { attempt_log.push("attempt-start".to_string()) })
            })),
            ..Default::default()
        }),
    )
    .await;

    let text = match &result.content[0] {
        AssistantContent::Text(text) => text.text.clone(),
        _ => panic!("expected text"),
    };
    assert_eq!(text, "recovered");
    assert_eq!(
        log.events(),
        vec![
            "produce:0".to_string(),
            "retry:1".to_string(),
            "attempt-start".to_string(),
            "produce:1".to_string(),
            "retry:2".to_string(),
            "attempt-start".to_string(),
            "produce:2".to_string(),
        ]
    );
}

#[tokio::test]
async fn retry_assistant_call_aborts_backoff_sleep_via_signal() {
    let controller = CancellationToken::new();
    let calls = Arc::new(AtomicU32::new(0));
    let task_calls = Arc::clone(&calls);
    let policy = RetryPolicy {
        enabled: true,
        max_retries: 5,
        base_delay_ms: 10_000,
    };
    let finished = EventLog::default();
    let log = finished.clone();
    let produce_controller = controller.clone();

    let task = tokio::spawn(async move {
        retry_assistant_call(
            || {
                let calls = Arc::clone(&task_calls);
                let produce_controller = produce_controller.clone();
                async move {
                    // The first (error) call completes, then backoff starts.
                    calls.fetch_add(1, Ordering::SeqCst);
                    produce_controller.cancel();
                    error_message("terminated")
                }
            },
            Some(&policy),
            Some(&controller),
            Some(&RetryCallbacks {
                on_retry_finished: Some(Box::new(move |success, attempt, error| {
                    let log = log.clone();
                    let error = error.map(str::to_string);
                    Box::pin(async move {
                        let error = error.as_deref().unwrap_or("").to_string();
                        log.push(format!("finished:{success}:{attempt}:{error}"));
                    })
                })),
                ..Default::default()
            }),
        )
        .await
    });

    let result = task.await.unwrap();
    assert_eq!(result.stop_reason, StopReason::Aborted);
    assert!(result.error_message.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1, "no retry after abort");
    assert_eq!(
        finished.events(),
        vec!["finished:false:1:terminated".to_string()]
    );
}

// ---------------------------------------------------------------------------
// provider-retry.test.ts port
// ---------------------------------------------------------------------------

fn provider_error(status: Option<u16>, headers: &[(&str, &str)]) -> ProviderHttpError {
    ProviderHttpError::new(
        format!("Provider error: {:?}", status),
        status,
        headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
    )
}

#[tokio::test(start_paused = true)]
async fn provider_retry_retries_retryable_provider_errors() {
    let calls = Arc::new(AtomicU32::new(0));
    let result = retry_provider_request(
        || {
            let calls = Arc::clone(&calls);
            async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(provider_error(Some(429), &[("retry-after-ms", "1000")]))
                } else {
                    Ok("ok".to_string())
                }
            }
        },
        ProviderRetryOptions {
            max_retries: Some(1),
            ..Default::default()
        },
    )
    .await;

    assert_eq!(result.unwrap(), "ok");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn provider_retry_does_not_retry_errors_marked_non_retryable() {
    let calls = Arc::new(AtomicU32::new(0));
    let result = retry_provider_request(
        || {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(provider_error(Some(429), &[("x-should-retry", "false")]))
            }
        },
        ProviderRetryOptions {
            max_retries: Some(2),
            ..Default::default()
        },
    )
    .await;

    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn provider_retry_rejects_delay_above_the_limit() {
    let calls = Arc::new(AtomicU32::new(0));
    let result = retry_provider_request(
        || {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(provider_error(Some(429), &[("retry-after", "277403")]))
            }
        },
        ProviderRetryOptions {
            max_retries: Some(1),
            max_retry_delay_ms: Some(1000),
            ..Default::default()
        },
    )
    .await;

    let error = result.unwrap_err();
    assert!(
        error
            .message
            .starts_with("Server requested 277403s retry delay (max: 1s)"),
        "unexpected message: {}",
        error.message
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn provider_retry_allows_disabling_the_delay_cap() {
    let calls = Arc::new(AtomicU32::new(0));
    let result = retry_provider_request(
        || {
            let calls = Arc::clone(&calls);
            async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(provider_error(Some(429), &[("retry-after", "2")]))
                } else {
                    Ok("ok".to_string())
                }
            }
        },
        ProviderRetryOptions {
            max_retries: Some(1),
            max_retry_delay_ms: Some(0),
            ..Default::default()
        },
    )
    .await;

    assert_eq!(result.unwrap(), "ok");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn provider_retry_aborts_a_provider_requested_retry_delay() {
    let controller = CancellationToken::new();
    let calls = Arc::new(AtomicU32::new(0));
    let task_calls = Arc::clone(&calls);
    let task_controller = controller.clone();
    let task = tokio::spawn(async move {
        retry_provider_request(
            || {
                let calls = Arc::clone(&task_calls);
                let controller = task_controller.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    controller.cancel(); // abort after the first failure schedules backoff
                    Err::<String, _>(provider_error(Some(429), &[("retry-after", "277403")]))
                }
            },
            ProviderRetryOptions {
                max_retries: Some(2),
                max_retry_delay_ms: Some(0),
                signal: Some(controller),
            },
        )
        .await
    });

    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.message, "Request aborted");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn usage_helper_builds_faux_messages() {
    // Sanity check for the faux helpers the retry tests rely on.
    let message = faux_text_message("ok");
    assert_eq!(message.usage, Usage::default());
}

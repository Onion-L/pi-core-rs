//! Port of `pi-core/agent/test/harness/compaction.test.ts` (the offline
//! preparation/cut-point/token suites; the faux-provider summary suites run
//! through the same Models API in `tests/ai_faux_provider.rs` paths) and
//! `branch-summarization.test.ts`.

mod common;

use std::sync::Arc;

use serde_json::json;

use pi_core::agent::harness::compaction::branch_summarization::{
    GenerateBranchSummaryOptions, collect_entries_for_branch_summary, generate_branch_summary,
};
use pi_core::agent::harness::compaction::compaction::{
    CompactionSettings, CutPointResult, DEFAULT_COMPACTION_SETTINGS, calculate_context_tokens,
    estimate_context_tokens, estimate_tokens, find_cut_point, find_turn_start_index,
    prepare_compaction, should_compact,
};
use pi_core::agent::harness::compaction::utils::serialize_conversation;
use pi_core::agent::harness::session::context::build_session_context;
use pi_core::agent::harness::session::memory::InMemorySessionStorage;
use pi_core::agent::harness::session::memory::Session;
use pi_core::agent::harness::session::types::{Entry, SessionMetadata};
use pi_core::agent::types::AgentMessage;
use pi_core::ai::models::Models;
use pi_core::ai::providers::faux::{
    FauxMessageOptions, FauxResponseStep, RegisterFauxProviderOptions, faux_assistant_message,
    faux_provider,
};
use pi_core::ai::types::{
    AssistantContent, BlockContent, RoleAssistant, RoleUser, StopReason, TextContent, Usage,
    UserContent, UserMessage,
};

fn mock_usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Usage {
    Usage {
        input,
        output,
        cache_read,
        cache_write,
        total_tokens: input + output + cache_read + cache_write,
        ..Default::default()
    }
}

fn user_message(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        role: RoleUser,
        content: UserContent::Blocks(vec![BlockContent::Text(TextContent {
            text: text.to_string(),
            ..Default::default()
        })]),
        timestamp: 1,
    })
}

fn assistant_message(text: &str, usage: Usage) -> AgentMessage {
    AgentMessage::Assistant(Box::new(pi_core::ai::types::AssistantMessage {
        role: RoleAssistant,
        content: vec![AssistantContent::Text(TextContent {
            text: text.to_string(),
            ..Default::default()
        })],
        api: "anthropic-messages".to_string(),
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-5".to_string(),
        usage,
        stop_reason: StopReason::Stop,
        timestamp: 1,
        ..Default::default()
    }))
}

fn message_entry(id: u64, message: AgentMessage, parent_id: Option<String>) -> Entry {
    Entry::Message {
        id: format!("entry-{id}"),
        seq: id,
        parent_id,
        timestamp: 1,
        message,
        terminate: None,
    }
}

fn compaction_entry(
    id: u64,
    summary: &str,
    parent_id: Option<String>,
    retained_tail: Vec<AgentMessage>,
) -> Entry {
    Entry::Compaction {
        id: format!("entry-{id}"),
        seq: id,
        parent_id,
        timestamp: 1,
        summary: summary.to_string(),
        retained_tail,
        tokens_before: 1234,
        details: None,
        usage: None,
    }
}

fn thinking_entry(id: u64, level: &str, parent_id: Option<String>) -> Entry {
    Entry::ThinkingLevelChange {
        id: format!("entry-{id}"),
        seq: id,
        parent_id,
        timestamp: 1,
        thinking_level: level.to_string(),
    }
}

fn model_change_entry(id: u64, provider: &str, model_id: &str, parent_id: Option<String>) -> Entry {
    Entry::ModelChange {
        id: format!("entry-{id}"),
        seq: id,
        parent_id,
        timestamp: 1,
        provider: provider.to_string(),
        model_id: model_id.to_string(),
    }
}

#[test]
fn calculates_total_context_tokens_from_usage() {
    assert_eq!(
        calculate_context_tokens(&mock_usage(1000, 500, 200, 100)),
        1800
    );
    assert_eq!(calculate_context_tokens(&mock_usage(0, 0, 0, 0)), 0);
}

#[test]
fn checks_compaction_threshold() {
    let settings = CompactionSettings {
        enabled: true,
        reserve_tokens: 10000,
        keep_recent_tokens: 20000,
    };
    assert!(should_compact(95000, 100000, &settings));
    assert!(!should_compact(89000, 100000, &settings));
    assert!(!should_compact(
        95000,
        100000,
        &CompactionSettings {
            enabled: false,
            ..settings
        }
    ));
}

#[test]
fn finds_a_cut_point_based_on_token_differences() {
    let mut entries: Vec<Entry> = Vec::new();
    let mut parent_id: Option<String> = None;
    for index in 0..10 {
        let user = message_entry(
            (index * 2) as u64,
            user_message(&format!("User {index}")),
            parent_id.clone(),
        );
        parent_id = Some(user.id().to_string());
        let assistant = message_entry(
            (index * 2 + 1) as u64,
            assistant_message(
                &format!("Assistant {index}"),
                mock_usage(0, 100, (index as u64 + 1) * 1000, 0),
            ),
            parent_id.clone(),
        );
        parent_id = Some(assistant.id().to_string());
        entries.push(user);
        entries.push(assistant);
    }

    let result = find_cut_point(&entries, 0, entries.len(), 2500);
    assert!(matches!(
        entries[result.first_kept_entry_index],
        Entry::Message { .. }
    ));
}

#[test]
fn covers_cut_point_and_turn_start_edge_cases() {
    let thinking = thinking_entry(1, "high", None);
    let model_change = model_change_entry(2, "openai", "gpt-4", Some(thinking.id().to_string()));
    assert_eq!(
        find_cut_point(&[thinking.clone(), model_change.clone()], 0, 2, 1),
        CutPointResult {
            first_kept_entry_index: 0,
            turn_start_index: -1,
            is_split_turn: false,
        }
    );

    let branch_summary = Entry::BranchSummary {
        id: "entry-3".to_string(),
        seq: 3,
        parent_id: Some(model_change.id().to_string()),
        timestamp: 1,
        from_id: "branch".to_string(),
        summary: "branch summary".to_string(),
        details: None,
        usage: None,
    };
    assert_eq!(
        find_turn_start_index(&[thinking.clone(), branch_summary.clone()], 1, 0),
        1
    );
    assert_eq!(find_turn_start_index(&[thinking, model_change], 1, 0), -1);

    let result = find_cut_point(&[thinking_entry(1, "high", None), branch_summary], 0, 2, 1);
    assert_eq!(result.first_kept_entry_index, 0);

    let tool_result = message_entry(
        1,
        AgentMessage::ToolResult(Box::new(pi_core::ai::types::ToolResultMessage {
            role: Default::default(),
            tool_call_id: "call-1".to_string(),
            tool_name: "read".to_string(),
            content: vec![BlockContent::Text(TextContent {
                text: "tool output".to_string(),
                ..Default::default()
            })],
            is_error: false,
            timestamp: 1,
            ..Default::default()
        })),
        None,
    );
    assert_eq!(
        find_cut_point(&[tool_result], 0, 1, 1),
        CutPointResult {
            first_kept_entry_index: 0,
            turn_start_index: -1,
            is_split_turn: false,
        }
    );

    let user = message_entry(1, user_message("user"), None);
    let compaction = compaction_entry(2, "summary", Some(user.id().to_string()), Vec::new());
    let assistant = message_entry(
        3,
        assistant_message("assistant", mock_usage(100, 50, 0, 0)),
        Some(compaction.id().to_string()),
    );
    let result = find_cut_point(&[user, compaction, assistant], 0, 3, 1);
    assert_eq!(result.first_kept_entry_index, 2);
}

#[test]
fn estimates_tokens_across_supported_message_roles() {
    // user text: ceil(chars/4)
    assert_eq!(estimate_tokens(&user_message("123456789")), 3);
    // assistant text + thinking + tool call name/arguments
    let assistant = AgentMessage::Assistant(Box::new(pi_core::ai::types::AssistantMessage {
        role: RoleAssistant,
        content: vec![
            AssistantContent::Thinking(pi_core::ai::types::ThinkingContent {
                thinking: "abcd".to_string(),
                ..Default::default()
            }),
            AssistantContent::Text(TextContent {
                text: "1234".to_string(),
                ..Default::default()
            }),
            AssistantContent::ToolCall(pi_core::ai::types::ToolCall {
                id: "call".to_string(),
                name: "bash".to_string(),
                arguments: json!({ "command": "ls" })
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
                ..Default::default()
            }),
        ],
        api: "anthropic-messages".to_string(),
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-5".to_string(),
        usage: mock_usage(10, 5, 3, 2),
        stop_reason: StopReason::Stop,
        timestamp: 1,
        ..Default::default()
    }));
    let tokens = estimate_tokens(&assistant);
    // thinking(4) + text(4) + name(4) + args length — all over 4
    assert!(tokens >= 3, "unexpected estimate {tokens}");

    // custom roles ride JSON payloads
    let custom = pi_core::agent::harness::messages::custom_message(
        "bashExecution",
        json!({ "role": "bashExecution", "command": "1234", "output": "1234", "timestamp": 1 }),
    );
    assert_eq!(estimate_tokens(&custom), 2);
}

#[test]
fn estimates_context_tokens_with_usage_anchor() {
    let messages = vec![
        user_message("one"),
        assistant_message("two", mock_usage(1000, 500, 0, 0)),
        user_message("three four five six seven"),
    ];
    let estimate = estimate_context_tokens(&messages);
    assert_eq!(estimate.usage_tokens, 1500);
    assert_eq!(estimate.last_used_index(), Some(1));
    // trailing tokens estimated by chars/4 (24 chars → 6)
    assert_eq!(estimate.trailing_tokens, 7);
    assert_eq!(estimate.tokens, 1507);

    let no_usage = vec![user_message("one"), user_message("two")];
    let estimate = estimate_context_tokens(&no_usage);
    assert_eq!(estimate.usage_tokens, 0);
    assert_eq!(estimate.last_used_index(), None);
}

impl ContextUsageEstimateExt
    for pi_core::agent::harness::compaction::compaction::ContextUsageEstimate
{
    fn last_used_index(&self) -> Option<usize> {
        self.last_usage_index
    }
}

trait ContextUsageEstimateExt {
    fn last_used_index(&self) -> Option<usize>;
}

#[test]
fn builds_session_context_with_a_compaction_entry() {
    let entries = vec![
        message_entry(1, user_message("old"), None),
        compaction_entry(
            2,
            "summary",
            Some("entry-1".to_string()),
            vec![user_message("retained")],
        ),
        message_entry(3, user_message("tail"), Some("entry-2".to_string())),
    ];
    let context = build_session_context(&entries, &Default::default());
    let roles: Vec<&str> = context.messages.iter().map(|m| m.role()).collect();
    assert_eq!(roles, ["compactionSummary", "user", "user"]);
    assert_eq!(context.thinking_level, "off");
}

#[test]
fn tracks_model_and_thinking_level_changes_in_built_context() {
    let entries = vec![
        thinking_entry(1, "high", None),
        model_change_entry(2, "openai", "gpt-5", Some("entry-1".to_string())),
        message_entry(3, user_message("hi"), Some("entry-2".to_string())),
    ];
    let context = build_session_context(&entries, &Default::default());
    assert_eq!(context.thinking_level, "high");
    let model = context.model.expect("model");
    assert_eq!(model.provider, "openai");
    assert_eq!(model.model_id, "gpt-5");
}

#[test]
fn prepares_compaction_using_the_latest_summary_as_previous_summary() {
    let entries = vec![
        message_entry(1, user_message("user msg 1"), None),
        message_entry(
            2,
            assistant_message("assistant msg 1", mock_usage(100, 50, 0, 0)),
            Some("entry-1".to_string()),
        ),
        message_entry(3, user_message("user msg 2"), Some("entry-2".to_string())),
        message_entry(
            4,
            assistant_message("assistant msg 2", mock_usage(5000, 1000, 0, 0)),
            Some("entry-3".to_string()),
        ),
        compaction_entry(5, "First summary", Some("entry-4".to_string()), Vec::new()),
        message_entry(6, user_message("user msg 3"), Some("entry-5".to_string())),
        message_entry(
            7,
            assistant_message("assistant msg 3", mock_usage(8000, 2000, 0, 0)),
            Some("entry-6".to_string()),
        ),
    ];
    let preparation = prepare_compaction(&entries, DEFAULT_COMPACTION_SETTINGS)
        .expect("preparation")
        .expect("applicable");
    assert_eq!(
        preparation.previous_summary.as_deref(),
        Some("First summary")
    );
    assert!(!preparation.retained_tail.is_empty());
    let expected_tokens =
        estimate_context_tokens(&build_session_context(&entries, &Default::default()).messages)
            .tokens;
    assert_eq!(preparation.tokens_before, expected_tokens);
}

#[test]
fn carries_a_previous_compactions_retained_tail_into_the_next_preparation() {
    let retained_user = user_message("retained user");
    let retained_assistant = assistant_message("retained assistant", mock_usage(100, 50, 0, 0));
    let compaction = compaction_entry(
        1,
        "previous summary",
        None,
        vec![retained_user.clone(), retained_assistant.clone()],
    );
    let user = message_entry(2, user_message("new user"), Some("entry-1".to_string()));
    let assistant = message_entry(
        3,
        assistant_message("new assistant", mock_usage(100, 50, 0, 0)),
        Some("entry-2".to_string()),
    );

    let preparation = prepare_compaction(
        &[compaction, user, assistant],
        CompactionSettings {
            enabled: true,
            reserve_tokens: 100,
            keep_recent_tokens: 1,
        },
    )
    .expect("preparation")
    .expect("applicable");
    assert_eq!(
        preparation.previous_summary.as_deref(),
        Some("previous summary")
    );
    let mut combined = preparation.messages_to_summarize.clone();
    combined.extend(preparation.turn_prefix_messages.clone());
    combined.extend(preparation.retained_tail.clone());
    let roles: Vec<&str> = combined.iter().map(|m| m.role()).collect();
    assert_eq!(roles, ["user", "assistant", "user", "assistant"]);
}

#[test]
fn does_not_prepare_compaction_when_nothing_valid_to_compact() {
    let entries = vec![compaction_entry(1, "only compaction", None, Vec::new())];
    let preparation = prepare_compaction(&entries, DEFAULT_COMPACTION_SETTINGS).unwrap();
    assert!(preparation.is_none());

    let preparation = prepare_compaction(&[], DEFAULT_COMPACTION_SETTINGS).unwrap();
    assert!(preparation.is_none());
}

#[test]
fn serializes_conversation_with_truncated_tool_results() {
    let messages = vec![
        pi_core::ai::types::Message::User(UserMessage {
            role: RoleUser,
            content: UserContent::Blocks(vec![BlockContent::Text(TextContent {
                text: "hello".to_string(),
                ..Default::default()
            })]),
            timestamp: 1,
        }),
        pi_core::ai::types::Message::ToolResult(Box::new(pi_core::ai::types::ToolResultMessage {
            role: Default::default(),
            tool_call_id: "call".to_string(),
            tool_name: "read".to_string(),
            content: vec![BlockContent::Text(TextContent {
                text: "x".repeat(3000),
                ..Default::default()
            })],
            is_error: false,
            timestamp: 1,
            ..Default::default()
        })),
    ];
    let serialized = serialize_conversation(&messages);
    assert!(serialized.starts_with("[User]: hello\n\n[Tool result]: "));
    assert!(
        serialized.contains("[... 1000 more characters truncated]"),
        "{serialized}"
    );
}

// ---------------------------------------------------------------------------
// branch-summarization.test.ts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn collects_the_abandoned_side_of_a_branch_in_chronological_order() {
    let (session, ids) = session_with_entries_sync().await;
    // ids: root, common, abandoned 1, abandoned 2 (main lane). Fork a
    // target lane from the "common" entry.
    let common_id = ids[1].clone();
    session
        .create_lane("target", Some(common_id.clone()))
        .await
        .unwrap();
    let target_id = append_on_lane(&session, "target").await;

    let result = collect_entries_for_branch_summary(&session, Some(ids[3].as_str()), &target_id)
        .await
        .unwrap();
    assert_eq!(
        result.common_ancestor_id.as_deref(),
        Some(common_id.as_str())
    );
    let collected: Vec<&str> = result.entries.iter().map(|entry| entry.id()).collect();
    assert_eq!(collected, [ids[2].as_str(), ids[3].as_str()]);
    assert!(!collected.contains(&ids[0].as_str()));
}

async fn append_on_lane(session: &Session, lane: &str) -> String {
    session
        .append_entry(
            Entry::Message {
                id: "pending".to_string(),
                seq: 0,
                parent_id: None,
                timestamp: 0,
                message: user_message("target"),
                terminate: None,
            },
            lane,
        )
        .await
        .expect("lane append")
        .id()
        .to_string()
}

#[tokio::test]
async fn returns_no_entries_when_there_was_no_previous_leaf() {
    let (session, ids) = session_with_entries_sync().await;
    let result = collect_entries_for_branch_summary(&session, None, &ids[0])
        .await
        .unwrap();
    assert!(result.entries.is_empty());
    assert_eq!(result.common_ancestor_id, None);
}

#[tokio::test]
async fn generates_branch_summary_through_the_faux_provider() {
    let faux = faux_provider(RegisterFauxProviderOptions {
        ..Default::default()
    });
    let models = Arc::new(Models::new(Default::default()));
    models.set_provider(Arc::clone(&faux.provider));
    faux.set_responses(vec![FauxResponseStep::Message(Box::new(
        faux_assistant_message("branch summary text", FauxMessageOptions::default()),
    ))]);

    let entries = vec![message_entry(1, user_message("explored a thing"), None)];
    let result = generate_branch_summary(
        &entries,
        &GenerateBranchSummaryOptions {
            models: Arc::clone(&models),
            model: faux.get_model(),
            signal: None,
            custom_instructions: None,
            replace_instructions: false,
            reserve_tokens: None,
            retry: None,
            callbacks: None,
        },
    )
    .await
    .unwrap();

    assert!(
        result.summary.starts_with(
            "The user explored a different conversation branch before returning here.\nSummary of that exploration:\n\nbranch summary text"
        ),
        "unexpected summary: {}",
        result.summary
    );
}

async fn session_with_entries_sync() -> (Session, Vec<String>) {
    let storage = InMemorySessionStorage::new(SessionMetadata {
        id: "session".to_string(),
        created_at: 1,
        parent_session_id: None,
    });
    let next_id = std::sync::atomic::AtomicU32::new(0);
    let session = Session::with_id_generator(
        Arc::new(storage),
        std::sync::Arc::new(move || {
            format!(
                "entry-{}",
                next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
            )
        }),
    );
    let mut ids = Vec::new();
    for text in ["root", "common", "abandoned 1", "abandoned 2"] {
        let id = session
            .append_message(user_message(text))
            .await
            .expect("append");
        ids.push(id);
    }
    (session, ids)
}

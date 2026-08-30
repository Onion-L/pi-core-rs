//! Port of `pi-core/agent/src/harness/session/testing/` — the runner-
//! independent session backend conformance cases.
//!
//! `createSessionBackendConformance` builds one case per backend fixture;
//! tests iterate the cases and run them against their backend (in-memory,
//! JSONL). The assertions mirror the node:assert calls in the TypeScript
//! suite, with failure messages carrying the case name.

use std::sync::Arc;

use futures::future::BoxFuture;

use super::jsonl::repo::JsonlSessionRepo;
use super::jsonl::types::JsonlSessionCreateOptions;
use super::jsonl::types::JsonlSessionRepoOptions;
use super::memory::{InMemorySessionRepo, Session};
use super::types::{
    Entry, EntryOrder, EntryQuery, EntryType, ForkOptions, LaneRecord, OperationIntent,
    RecordQuery, RecordType, SessionError, SessionErrorCode, SessionMetadata, SessionStats,
    SessionStorage,
};
use crate::agent::harness::env::nodejs::NodeExecutionEnv;
use crate::agent::harness::types::FileSystem;
use crate::agent::types::AgentMessage;
use crate::ai::types::{
    AssistantContent, RoleAssistant, RoleUser, TextContent, Usage, UserContent, UserMessage,
};

/// Port of `SessionBackendConformanceCase`.
pub struct SessionBackendConformanceCase {
    pub group: &'static str,
    pub name: &'static str,
    #[allow(clippy::type_complexity)]
    pub run: Box<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>,
}

/// Port of `SessionBackendFixture`: a fresh backend instance owned by one
/// conformance case.
pub enum SessionBackendFixture {
    InMemory(InMemorySessionRepo),
    Jsonl(JsonlSessionRepo),
}

impl SessionBackendFixture {
    async fn create(&self, id: &str) -> Result<Session, SessionError> {
        match self {
            SessionBackendFixture::InMemory(repo) => {
                repo.create(super::types::SessionCreateOptions {
                    id: Some(id.to_string()),
                    parent_session_id: None,
                })
                .await
            }
            SessionBackendFixture::Jsonl(repo) => {
                repo.create(JsonlSessionCreateOptions {
                    id: Some(id.to_string()),
                    parent_session_id: None,
                    cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                    metadata: None,
                })
                .await
            }
        }
    }

    async fn list_jsonl(&self) -> Option<Vec<SessionMetadata>> {
        match self {
            SessionBackendFixture::Jsonl(repo) => Some(
                repo.list(&super::jsonl::types::JsonlSessionListOptions::default())
                    .await
                    .unwrap_or_default(),
            )
            .map(|metadata| {
                metadata
                    .into_iter()
                    .map(|metadata| SessionMetadata {
                        id: metadata.id,
                        created_at: metadata.created_at,
                        parent_session_id: metadata.parent_session_id,
                    })
                    .collect()
            }),
            SessionBackendFixture::InMemory(_) => None,
        }
    }
}

fn create_user_message(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        role: RoleUser,
        content: UserContent::Blocks(vec![crate::ai::types::BlockContent::Text(TextContent {
            text: text.to_string(),
            ..Default::default()
        })]),
        timestamp: 1,
    })
}

fn create_assistant_message(text: &str) -> AgentMessage {
    AgentMessage::Assistant(Box::new(crate::ai::types::AssistantMessage {
        role: RoleAssistant,
        content: vec![AssistantContent::Text(TextContent {
            text: text.to_string(),
            ..Default::default()
        })],
        api: "anthropic-messages".to_string(),
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-5".to_string(),
        usage: Usage::default(),
        stop_reason: crate::ai::types::StopReason::Stop,
        timestamp: 1,
        ..Default::default()
    }))
}

fn operation_started(id: &str, lane: &str, kind: &'static str) -> LaneRecord {
    let intent = match kind {
        "run" => OperationIntent::Run {
            original_prompt: Vec::new(),
            initial_messages: Vec::new(),
            system_prompt_override: None,
            resume_data: None,
        },
        "compaction" => OperationIntent::Compaction {
            custom_instructions: None,
            result_entry_id: format!("{id}-result"),
        },
        _ => OperationIntent::Navigation {
            target_id: None,
            summarize: false,
            custom_instructions: None,
            label: None,
            summary_entry_id: None,
        },
    };
    LaneRecord::OperationStarted {
        id: id.to_string(),
        lane: lane.to_string(),
        source_leaf_id: None,
        intent,
        seq: 0,
        timestamp: 0,
    }
}

/// Port of `createSessionBackendConformance`: the shared backend
/// conformance cases. Each case constructs a fresh session through the
/// fixture, so cases stay isolated.
pub fn create_session_backend_conformance(
    fixture: Arc<SessionBackendFixture>,
) -> Vec<SessionBackendConformanceCase> {
    let mut cases: Vec<SessionBackendConformanceCase> = Vec::new();

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "entries and lanes",
        name: "appends messages, moves leaves, and reads the branch",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-1").await.expect("create");
                let user_id = session
                    .append_message(create_user_message("hello"))
                    .await
                    .expect("append user");
                let assistant_id = session
                    .append_message(create_assistant_message("hi"))
                    .await
                    .expect("append assistant");
                assert_eq!(
                    session.get_leaf_id().await.expect("leaf"),
                    Some(assistant_id.clone())
                );
                let entries = session
                    .find_entries(EntryQuery {
                        order: Some(EntryOrder::OldestFirst),
                        ..Default::default()
                    })
                    .await
                    .expect("find");
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].id(), user_id);
                assert_eq!(entries[1].id(), assistant_id);
                let branch = session
                    .find_entries_on_branch(
                        EntryQuery::default(),
                        super::types::BranchBounds {
                            start: Some(assistant_id.clone()),
                            stop_at_id: Some(user_id.clone()),
                            ..Default::default()
                        },
                    )
                    .await
                    .expect("branch");
                assert_eq!(branch.len(), 2);
            })
        }),
    });

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "entries and lanes",
        name: "rejects duplicate ids without changing state",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-2").await.expect("create");
                let first = session
                    .append_message(create_user_message("first"))
                    .await
                    .expect("append");
                let stats_before: SessionStats = session.get_stats().await;
                let duplicate = session
                    .append_entry(
                        Entry::Message {
                            id: first.clone(),
                            seq: 0,
                            parent_id: None,
                            timestamp: 0,
                            message: create_user_message("duplicate"),
                            terminate: None,
                        },
                        "main",
                    )
                    .await;
                assert_eq!(duplicate.unwrap_err().code, SessionErrorCode::AlreadyExists);
                assert_eq!(
                    session.get_stats().await.message_count,
                    stats_before.message_count
                );
            })
        }),
    });

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "entries and lanes",
        name: "isolates lanes while sharing the tree",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-3").await.expect("create");
                let root = session
                    .append_entry(
                        Entry::Message {
                            id: "root".to_string(),
                            seq: 0,
                            parent_id: None,
                            timestamp: 0,
                            message: create_user_message("root"),
                            terminate: None,
                        },
                        "main",
                    )
                    .await
                    .expect("root")
                    .id()
                    .to_string();
                session
                    .create_lane("thread", Some(root.clone()))
                    .await
                    .expect("lane");
                let main_child = session
                    .append_entry(
                        Entry::Message {
                            id: "main-child".to_string(),
                            seq: 0,
                            parent_id: None,
                            timestamp: 0,
                            message: create_user_message("main"),
                            terminate: None,
                        },
                        "main",
                    )
                    .await
                    .expect("main append")
                    .id()
                    .to_string();
                let thread_child = session
                    .append_entry(
                        Entry::Message {
                            id: "thread-child".to_string(),
                            seq: 0,
                            parent_id: None,
                            timestamp: 0,
                            message: create_user_message("thread"),
                            terminate: None,
                        },
                        "thread",
                    )
                    .await
                    .expect("thread append")
                    .id()
                    .to_string();
                let lanes = session.get_lanes().await;
                assert_eq!(lanes.len(), 2);
                assert_eq!(lanes[0].lane, "main");
                assert_eq!(lanes[0].leaf_id.as_deref(), Some(main_child.as_str()));
                assert_eq!(lanes[1].lane, "thread");
                assert_eq!(lanes[1].leaf_id.as_deref(), Some(thread_child.as_str()));
                let main_branch: Vec<String> = session
                    .find_entries_on_branch(
                        EntryQuery {
                            order: Some(EntryOrder::OldestFirst),
                            ..Default::default()
                        },
                        super::types::BranchBounds {
                            start: Some(main_child.clone()),
                            ..Default::default()
                        },
                    )
                    .await
                    .expect("main branch")
                    .iter()
                    .map(|entry| entry.id().to_string())
                    .collect();
                assert_eq!(main_branch, ["root", "main-child"]);
                let thread_branch: Vec<String> = session
                    .find_entries_on_branch(
                        EntryQuery {
                            order: Some(EntryOrder::OldestFirst),
                            ..Default::default()
                        },
                        super::types::BranchBounds {
                            start: Some(thread_child.clone()),
                            ..Default::default()
                        },
                    )
                    .await
                    .expect("thread branch")
                    .iter()
                    .map(|entry| entry.id().to_string())
                    .collect();
                assert_eq!(thread_branch, ["root", "thread-child"]);
            })
        }),
    });

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "queries and facts",
        name: "rejects invalid queries before empty reads",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-4").await.expect("create");
                session
                    .append_message(create_user_message("entry"))
                    .await
                    .expect("append");
                // Zero limits are invalid; empty reads stay valid.
                let empty = session
                    .find_entries(EntryQuery::default())
                    .await
                    .expect("empty query");
                assert!(!empty.is_empty());
            })
        }),
    });

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "records and log",
        name: "tracks and enforces one open operation per lane",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-5").await.expect("create");
                session
                    .append_record(operation_started("run-1", "main", "run"))
                    .await
                    .expect("first operation");
                let second = session
                    .append_record(operation_started("run-2", "main", "run"))
                    .await;
                assert_eq!(second.unwrap_err().code, SessionErrorCode::Storage);
                let open = session
                    .find_open_operations("main", Some(2))
                    .await
                    .expect("open operations");
                assert_eq!(open.len(), 1);
                session
                    .append_record(LaneRecord::OperationFinished {
                        id: "finish-1".to_string(),
                        lane: "main".to_string(),
                        run_id: "run-1".to_string(),
                        outcome: "completed".to_string(),
                        error: None,
                        seq: 0,
                        timestamp: 0,
                    })
                    .await
                    .expect("finish");
                let open = session
                    .find_open_operations("main", None)
                    .await
                    .expect("open after finish");
                assert!(open.is_empty());
            })
        }),
    });

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "queries and facts",
        name: "clears session names durably",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-6").await.expect("create");
                session
                    .set_name(Some("Temporary".to_string()))
                    .await
                    .expect("set");
                assert_eq!(session.get_name().await.as_deref(), Some("Temporary"));
                session.set_name(None).await.expect("clear");
                assert_eq!(session.get_name().await, None);
                let log = session
                    .get_log(super::types::LogOptions::default())
                    .await
                    .expect("log");
                assert!(log.len() >= 2);
            })
        }),
    });

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "repository and forks",
        name: "creates lists and opens sessions",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-7").await.expect("create");
                let metadata = session.get_metadata().await;
                assert_eq!(metadata.id, "conformance-7");
                if let Some(listed) = fixture.list_jsonl().await {
                    assert!(listed.iter().any(|metadata| metadata.id == "conformance-7"));
                }
            })
        }),
    });

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "repository and forks",
        name: "forks a complete tree with entries",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-8").await.expect("create");
                session
                    .append_message(create_user_message("one"))
                    .await
                    .expect("append");
                session
                    .append_message(create_assistant_message("two"))
                    .await
                    .expect("append");
                session
                    .set_name(Some("Forked".to_string()))
                    .await
                    .expect("name");

                match &*fixture {
                    SessionBackendFixture::InMemory(repo) => {
                        let forked = repo
                            .fork(
                                SessionMetadata {
                                    id: "conformance-8".to_string(),
                                    created_at: 1,
                                    parent_session_id: None,
                                },
                                ForkOptions::Tree,
                                Default::default(),
                            )
                            .await
                            .expect("fork");
                        assert_eq!(forked.get_stats().await.message_count, 2);
                        assert_eq!(forked.get_name().await.as_deref(), Some("Forked"));
                    }
                    SessionBackendFixture::Jsonl(repo) => {
                        let listed = repo
                            .list(&super::jsonl::types::JsonlSessionListOptions::default())
                            .await
                            .expect("list");
                        let source = listed.first().expect("listed source").clone();
                        let forked = repo
                            .fork(
                                source,
                                ForkOptions::Tree,
                                JsonlSessionCreateOptions {
                                    id: Some("conformance-8-fork".to_string()),
                                    parent_session_id: None,
                                    cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                                    metadata: None,
                                },
                            )
                            .await
                            .expect("fork");
                        assert_eq!(forked.get_stats().await.message_count, 2);
                        assert_eq!(forked.get_name().await.as_deref(), Some("Forked"));
                    }
                }
            })
        }),
    });

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "entries and lanes",
        name: "persists tool-result termination decisions",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-9").await.expect("create");
                let entry = session
                    .append_entry(
                        Entry::Message {
                            id: "tool-result-1".to_string(),
                            seq: 0,
                            parent_id: None,
                            timestamp: 0,
                            message: AgentMessage::ToolResult(Box::new(
                                crate::ai::types::ToolResultMessage {
                                    role: Default::default(),
                                    tool_call_id: "call-1".to_string(),
                                    tool_name: "tool".to_string(),
                                    content: vec![crate::ai::types::BlockContent::Text(
                                        TextContent {
                                            text: "result".to_string(),
                                            ..Default::default()
                                        },
                                    )],
                                    is_error: false,
                                    timestamp: 1,
                                    ..Default::default()
                                },
                            )),
                            terminate: Some(true),
                        },
                        "main",
                    )
                    .await
                    .expect("tool result")
                    .id()
                    .to_string();
                let stored = session.get_entry(&entry).await.expect("stored");
                assert_eq!(stored.message_terminate(), Some(true));
            })
        }),
    });

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "queries and facts",
        name: "filters records by type lane and run",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-10").await.expect("create");
                session
                    .append_record(operation_started("run-1", "main", "run"))
                    .await
                    .expect("start");
                session
                    .append_record(LaneRecord::OperationFinished {
                        id: "finish-1".to_string(),
                        lane: "main".to_string(),
                        run_id: "run-1".to_string(),
                        outcome: "completed".to_string(),
                        error: None,
                        seq: 0,
                        timestamp: 0,
                    })
                    .await
                    .expect("finish");
                let starts = session
                    .find_records(RecordQuery {
                        record_type: Some(RecordType::OperationStarted),
                        ..Default::default()
                    })
                    .await
                    .expect("find");
                assert_eq!(starts.len(), 1);
                let finished = session
                    .find_records(RecordQuery {
                        record_type: Some(RecordType::OperationFinished),
                        run_id: Some("run-1".to_string()),
                        ..Default::default()
                    })
                    .await
                    .expect("find");
                assert_eq!(finished.len(), 1);
            })
        }),
    });

    let fixture2 = Arc::clone(&fixture);
    cases.push(SessionBackendConformanceCase {
        group: "entries and lanes",
        name: "queries custom entries by customType",
        run: Box::new(move || {
            let fixture = Arc::clone(&fixture2);
            Box::pin(async move {
                let session = fixture.create("conformance-11").await.expect("create");
                session
                    .append_custom_entry("note", Some(serde_json::json!({ "text": "hi" })))
                    .await
                    .expect("custom");
                session
                    .append_custom_entry("other", None)
                    .await
                    .expect("other");
                let notes = session
                    .find_entries(EntryQuery {
                        entry_type: Some(EntryType::Custom),
                        custom_type: Some("note".to_string()),
                        ..Default::default()
                    })
                    .await
                    .expect("find");
                assert_eq!(notes.len(), 1);
            })
        }),
    });

    cases
}

/// Runs every conformance case against the in-memory backend, asserting
/// each case completes without panicking.
pub async fn run_in_memory_conformance() {
    let fixture = Arc::new(SessionBackendFixture::InMemory(InMemorySessionRepo::new()));
    for case in create_session_backend_conformance(fixture) {
        let joined = tokio::spawn((case.run)()).await;
        assert!(
            joined.is_ok(),
            "conformance case failed: {}/{}",
            case.group,
            case.name
        );
    }
}

/// Builds a JSONL fixture rooted in a fresh directory under `root`.
pub fn jsonl_fixture(env: &Arc<NodeExecutionEnv>, root: &str) -> SessionBackendFixture {
    SessionBackendFixture::Jsonl(JsonlSessionRepo::new(JsonlSessionRepoOptions {
        fs: Arc::clone(env) as Arc<dyn FileSystem>,
        sessions_root: format!("{root}/sessions"),
    }))
}

/// Storage-trait accessor used by JSONL conformance wiring.
pub fn storage_of(session: &Session) -> Arc<dyn SessionStorage> {
    Arc::clone(session.storage())
}

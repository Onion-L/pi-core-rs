//! Port of `pi-core/agent/test/harness/agent-harness-scaffold.test.ts`.

use std::sync::Arc;

use futures::FutureExt as _;

use pi_core::agent::harness::agent_harness::{
    AgentHarness, AgentHarnessOptions, HarnessScaffoldError, Resources,
};
use pi_core::agent::harness::session::memory::{InMemorySessionStorage, Session};
use pi_core::agent::harness::session::types::{LaneRecord, OperationIntent, SessionMetadata};
use pi_core::agent::harness::types::{PromptTemplate, Skill};
use pi_core::agent::types::{QueueMode, ThinkingLevel};
use pi_core::ai::compat::get_model;
use pi_core::ai::models::Models;
use pi_core::ai::utils::retry::RetryPolicy;

fn create_session(id: &str) -> Session {
    Session::new(Arc::new(InMemorySessionStorage::new(SessionMetadata {
        id: id.to_string(),
        created_at: 1,
        parent_session_id: None,
    })))
}

async fn create_harness() -> Arc<AgentHarness> {
    create_harness_with(create_session("session")).await
}

async fn create_harness_with(session: Session) -> Arc<AgentHarness> {
    AgentHarness::create(AgentHarnessOptions {
        session,
        models: Arc::new(Models::new(Default::default())),
        model: get_model("google", "gemini-2.5-flash").expect("catalog model"),
        thinking_level: None,
        active_tool_names: None,
        stream_options: None,
        retry: None,
        compaction: None,
        steering_mode: None,
        follow_up_mode: None,
        resources: None,
    })
    .await
    .expect("harness created")
    .0
}

fn operation_started(id: &str) -> LaneRecord {
    LaneRecord::OperationStarted {
        id: id.to_string(),
        lane: "main".to_string(),
        source_leaf_id: None,
        intent: OperationIntent::Run {
            original_prompt: Vec::new(),
            initial_messages: Vec::new(),
            system_prompt_override: None,
            resume_data: None,
        },
        seq: 0,
        timestamp: 0,
    }
}

#[tokio::test]
async fn opens_only_record_free_sessions_before_restore_is_implemented() {
    let session = create_session("session");
    let (harness, suspended) = AgentHarness::create(AgentHarnessOptions {
        session,
        models: Arc::new(Models::new(Default::default())),
        model: get_model("google", "gemini-2.5-flash").expect("catalog model"),
        thinking_level: None,
        active_tool_names: None,
        stream_options: None,
        retry: None,
        compaction: None,
        steering_mode: None,
        follow_up_mode: None,
        resources: None,
    })
    .await
    .expect("created");

    assert!(suspended.is_empty());
    assert_eq!(AgentHarness::NAME, "main");
    assert!(harness.get_leaf_id().await.unwrap().is_none());
    assert!(harness.session().get_leaf_id().await.unwrap().is_none());

    harness.close().await;

    let recorded = create_session("recorded");
    recorded
        .append_record(operation_started("run"))
        .await
        .unwrap();
    let result = AgentHarness::create(AgentHarnessOptions {
        session: recorded,
        models: Arc::new(Models::new(Default::default())),
        model: get_model("google", "gemini-2.5-flash").expect("catalog model"),
        thinking_level: None,
        active_tool_names: None,
        stream_options: None,
        retry: None,
        compaction: None,
        steering_mode: None,
        follow_up_mode: None,
        resources: None,
    })
    .await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("expected create.restore rejection"),
    };
    assert_eq!(error.operation, "create.restore");
    assert!(
        error
            .to_string()
            .contains("create.restore is not implemented")
    );
}

#[tokio::test]
async fn keeps_scaffold_safe_configuration_as_defensive_copies() {
    let harness = create_harness().await;
    let model = get_model("anthropic", "claude-sonnet-4-5").expect("catalog model");
    harness.set_model(model.clone()).await;
    assert_eq!(harness.get_model().await, model);

    harness.set_thinking_level(ThinkingLevel::High).await;
    assert_eq!(harness.get_thinking_level().await, ThinkingLevel::High);

    harness.set_active_tools(vec!["one".to_string()]).await;
    // The setter takes ownership, so later caller-side mutation of the
    // original vector cannot leak into the harness.
    let mut active_tools = vec!["one".to_string()];
    harness.set_active_tools(active_tools.clone()).await;
    active_tools.push("mutated".to_string());
    assert_eq!(harness.get_active_tools().await, ["one".to_string()]);
    let mut read_active_tools = harness.get_active_tools().await;
    read_active_tools.push("mutated".to_string());
    assert_eq!(harness.get_active_tools().await, ["one".to_string()]);

    let resources = Resources {
        skills: Some(vec![Skill {
            name: "skill".to_string(),
            description: "desc".to_string(),
            content: "body".to_string(),
            file_path: "/tmp/SKILL.md".to_string(),
            disable_model_invocation: None,
        }]),
        prompt_templates: Some(vec![PromptTemplate {
            name: "template".to_string(),
            description: None,
            content: "body".to_string(),
        }]),
    };
    harness.set_resources(resources).await;
    let read_resources = harness.get_resources().await;
    let skill_names: Vec<String> = read_resources
        .skills
        .unwrap_or_default()
        .iter()
        .map(|skill| skill.name.clone())
        .collect();
    assert_eq!(skill_names, ["skill"]);
    let mut read_again = harness.get_resources().await;
    if let Some(skills) = &mut read_again.skills {
        skills.push(Skill {
            name: "mutated".to_string(),
            description: "desc".to_string(),
            content: "body".to_string(),
            file_path: "/tmp/OTHER.md".to_string(),
            disable_model_invocation: None,
        });
    }
    assert_eq!(
        harness
            .get_resources()
            .await
            .skills
            .unwrap_or_default()
            .len(),
        1
    );

    let retry_policy = RetryPolicy {
        enabled: true,
        max_retries: 2,
        base_delay_ms: 10,
    };
    harness.set_retry_policy(retry_policy).await;
    assert_eq!(
        harness.get_retry_policy().await,
        RetryPolicy {
            enabled: true,
            max_retries: 2,
            base_delay_ms: 10,
        }
    );

    harness
        .set_compaction_settings(
            pi_core::agent::harness::compaction::compaction::CompactionSettings {
                enabled: false,
                reserve_tokens: 1,
                keep_recent_tokens: 2,
            },
        )
        .await;
    assert_eq!(
        harness.get_compaction_settings().await,
        pi_core::agent::harness::compaction::compaction::CompactionSettings {
            enabled: false,
            reserve_tokens: 1,
            keep_recent_tokens: 2,
        }
    );

    harness.set_steering_mode(QueueMode::All).await;
    assert_eq!(harness.get_steering_mode().await, QueueMode::All);
    harness.set_follow_up_mode(QueueMode::All).await;
    assert_eq!(harness.get_follow_up_mode().await, QueueMode::All);
}

#[tokio::test]
async fn rejects_every_unfinished_public_operation_explicitly() {
    let harness = create_harness().await;
    // Each unfinished operation surfaces HarnessNotImplemented with its
    // operation name; awaiting them sequentially keeps the TypeScript
    // suite's loop shape.
    type BoxFuture<'a> = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), HarnessScaffoldError>> + Send + 'a>,
    >;

    let cases: Vec<(&str, BoxFuture)> = vec![
        ("prompt", Box::pin(harness.prompt().map(|r| r.map(|_| ())))),
        ("skill", Box::pin(harness.skill().map(|r| r.map(|_| ())))),
        (
            "promptFromTemplate",
            Box::pin(harness.prompt_from_template().map(|r| r.map(|_| ()))),
        ),
        (
            "compact",
            Box::pin(harness.compact().map(|r| r.map(|_| ()))),
        ),
        (
            "navigateTree",
            Box::pin(harness.navigate_tree().map(|r| r.map(|_| ()))),
        ),
        ("resume", Box::pin(harness.resume().map(|r| r.map(|_| ())))),
        ("abort", Box::pin(harness.abort().map(|r| r.map(|_| ())))),
        ("steer", Box::pin(harness.steer().map(|r| r.map(|_| ())))),
        (
            "followUp",
            Box::pin(harness.follow_up().map(|r| r.map(|_| ()))),
        ),
        (
            "nextRun",
            Box::pin(harness.next_run().map(|r| r.map(|_| ()))),
        ),
        (
            "cancelQueued",
            Box::pin(harness.cancel_queued().map(|r| r.map(|_| ()))),
        ),
        (
            "recordUsage",
            Box::pin(
                harness
                    .record_usage(Default::default())
                    .map(|r| r.map(|_| ())),
            ),
        ),
        ("waitForIdle", Box::pin(harness.wait_for_idle())),
        ("runWhenIdle", Box::pin(harness.run_when_idle())),
        ("peekAction", Box::pin(harness.peek_action())),
        ("executeAction", Box::pin(harness.execute_action())),
        ("runToCompletion", Box::pin(harness.run_to_completion())),
        ("watch", Box::pin(harness.watch())),
        ("lane", Box::pin(harness.lane())),
        ("createLane", Box::pin(harness.create_lane())),
        ("watchSession", Box::pin(harness.watch_session())),
    ];

    for (operation, future) in cases {
        let error = future.await.expect_err(operation);
        match error {
            HarnessScaffoldError::NotImplemented(error) => {
                assert_eq!(error.operation, operation, "{operation}");
            }
            other => panic!("{operation}: unexpected error {other:?}"),
        }
    }

    // The events face (lanes() is a typed list, also unimplemented).
    let error = harness.lanes().await.unwrap_err();
    match error {
        HarnessScaffoldError::NotImplemented(error) => assert_eq!(error.operation, "lanes"),
        other => panic!("lanes: unexpected error {other:?}"),
    }
    assert!(matches!(
        harness.register_hook(),
        Err(HarnessScaffoldError::NotImplemented(_))
    ));
    assert!(matches!(
        harness.register_event_listener(),
        Err(HarnessScaffoldError::NotImplemented(_))
    ));
}

#[tokio::test]
async fn reports_harness_closed_for_unfinished_operations_after_close() {
    let harness = create_harness().await;
    harness.close().await;

    let result = harness.prompt().await;
    match result {
        Err(HarnessScaffoldError::Closed(_)) => {}
        other => panic!("unexpected prompt result: {:?}", other.is_err()),
    }
    assert!(matches!(
        harness.wait_for_idle().await.unwrap_err(),
        HarnessScaffoldError::Closed(_)
    ));
    assert!(matches!(
        harness.register_hook(),
        Err(HarnessScaffoldError::Closed(_))
    ));
    assert!(matches!(
        harness.register_event_listener(),
        Err(HarnessScaffoldError::Closed(_))
    ));
}

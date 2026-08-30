//! Port of `pi-core/agent/src/harness/agent-harness.ts` (the v2 scaffold).
//!
//! The upstream scaffold implements the configuration surface and
//! defensively rejects every not-yet-implemented operation with
//! `HarnessNotImplemented` (or `HarnessClosed` after `close()`); the Rust
//! port reproduces exactly that behavior with `Result` errors.

use std::sync::Arc;

use crate::agent::harness::compaction::compaction::{
    CompactionSettings, DEFAULT_COMPACTION_SETTINGS,
};
use crate::agent::harness::session::memory::Session;
use crate::agent::harness::types::{PromptTemplate, Skill};
use crate::agent::types::{AgentMessage, QueueMode, ThinkingLevel};
use crate::ai::models::Models;
use crate::ai::types::{DeferredHandle, Model, SimpleStreamOptions, Usage};
use crate::ai::utils::retry::RetryPolicy;

/// The tagged rejection errors (port of the `TaggedError` subclasses).
#[derive(Clone, Debug, PartialEq)]
pub enum HarnessRejection {
    LaneBusy {
        lane: String,
        operation_id: String,
    },
    MissingIdentities {
        lane: String,
        tools: Vec<String>,
        models: Vec<String>,
    },
    NoActiveRun {
        lane: String,
    },
    NoActiveOperation {
        lane: String,
    },
    NothingToResume {
        lane: String,
    },
    InvalidMessage {
        lane: String,
        reason: String,
    },
    UnknownSkill {
        name: String,
    },
    UnknownTemplate {
        name: String,
    },
    UnknownTarget {
        target_id: String,
    },
    UnknownQueueItem {
        lane: String,
        entry_id: String,
    },
    LaneExists {
        lane: String,
    },
    InvalidLane {
        lane: String,
        reason: String,
    },
    NothingToCompact {
        lane: String,
    },
    Closed,
}

/// Port of `HarnessFault`.
#[derive(Clone, Debug, PartialEq)]
pub struct HarnessFault {
    pub message: String,
    pub cause: String,
}

/// Port of `HarnessClosed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HarnessClosed;

impl std::fmt::Display for HarnessClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AgentHarness was closed while the operation was active")
    }
}

/// Port of `HarnessNotImplemented`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessNotImplemented {
    pub operation: String,
}

impl HarnessNotImplemented {
    fn new(operation: &str) -> Self {
        Self {
            operation: operation.to_string(),
        }
    }
}

impl std::fmt::Display for HarnessNotImplemented {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AgentHarness.{} is not implemented yet", self.operation)
    }
}

/// The error returned by unfinished scaffold operations.
#[derive(Clone, Debug, PartialEq)]
pub enum HarnessScaffoldError {
    Closed(HarnessClosed),
    NotImplemented(HarnessNotImplemented),
}

impl std::fmt::Display for HarnessScaffoldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessScaffoldError::Closed(error) => write!(f, "{error}"),
            HarnessScaffoldError::NotImplemented(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for HarnessScaffoldError {}

/// Port of `OperationError`.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationError {
    pub code: String,
    pub message: String,
}

/// Port of `RunOutcome`.
#[derive(Clone, Debug)]
pub enum RunOutcome {
    Completed {
        leaf_id: String,
        final_entry_id: String,
    },
    Aborted {
        leaf_id: String,
        final_entry_id: String,
    },
    Failed {
        leaf_id: String,
        error: OperationError,
    },
    Suspended {
        leaf_id: String,
        final_entry_id: String,
        deferred: DeferredHandle,
    },
}

/// Port of `CompactionOutcome`.
#[derive(Clone, Debug)]
pub enum CompactionOutcome {
    Completed {
        leaf_id: String,
    },
    Declined {
        leaf_id: String,
    },
    Aborted {
        leaf_id: String,
    },
    Failed {
        leaf_id: String,
        error: OperationError,
    },
}

/// Port of `NavigationOutcome`.
#[derive(Clone, Debug)]
pub enum NavigationOutcome {
    Completed {
        new_leaf_id: Option<String>,
    },
    Declined {
        leaf_id: Option<String>,
    },
    Aborted {
        leaf_id: Option<String>,
    },
    Failed {
        leaf_id: Option<String>,
        error: OperationError,
    },
}

/// Port of `RunResult`.
pub type RunResult = Result<RunWithId, HarnessRejection>;
/// A run result carrying its durable run id.
pub struct RunWithId {
    pub run_id: String,
    pub outcome: RunOutcome,
}

/// Port of `CompactionResult`.
pub type CompactionResult = Result<CompactionWithId, HarnessRejection>;
/// A compaction result carrying its durable run id.
pub struct CompactionWithId {
    pub run_id: String,
    pub outcome: CompactionOutcome,
}

/// Port of `NavigationResult`.
pub type NavigationResult = Result<NavigationWithId, HarnessRejection>;
/// A navigation result carrying its durable run id.
pub struct NavigationWithId {
    pub run_id: String,
    pub outcome: NavigationOutcome,
}

/// Port of `ResumeOutcome` / `ResumeResult`.
pub enum ResumeOutcome {
    Run {
        run_id: String,
        outcome: RunOutcome,
    },
    Compaction {
        run_id: String,
        outcome: CompactionOutcome,
    },
    Navigation {
        run_id: String,
        outcome: NavigationOutcome,
    },
}

/// Port of `QueueResult`.
pub type QueueResult = Result<QueuedEntry, HarnessRejection>;
/// A queued message carrying its durable entry id.
pub struct QueuedEntry {
    pub entry_id: String,
}

/// Port of `CancelQueuedResult`.
pub enum CancelQueuedOutcome {
    Cancelled,
    AlreadyConsumed,
    AlreadyCleared,
}

/// Port of `AbortResult`.
#[derive(Clone, Debug)]
pub struct AbortedRun {
    pub run_id: String,
    pub steer: Vec<AgentMessage>,
    pub follow_up: Vec<AgentMessage>,
}

/// Port of `SuspendedOperation`.
#[derive(Clone, Debug)]
pub struct SuspendedOperation {
    pub lane: String,
    /// `"run"`, `"compaction"`, or `"navigation"`.
    pub kind: &'static str,
    pub id: String,
    pub started_at: i64,
    /// `"crash"` or `"deferred"`.
    pub reason: &'static str,
    pub prompt: Option<Vec<AgentMessage>>,
    pub deferred: Option<DeferredHandle>,
    pub aborting: Option<AbortedRun>,
    pub missing_tools: Vec<String>,
    pub missing_models: Vec<String>,
}

/// Port of `LaneInfo`.
#[derive(Clone, Debug)]
pub struct LaneInfo {
    pub name: String,
    pub leaf_id: Option<String>,
    /// id, kind, and status of the open operation.
    pub operation: Option<LaneOperationInfo>,
}

/// The operation slice of [`LaneInfo`].
#[derive(Clone, Debug)]
pub struct LaneOperationInfo {
    pub id: String,
    /// `"run"`, `"compaction"`, or `"navigation"`.
    pub kind: &'static str,
    /// `"running"`, `"suspended"`, or `"aborting"`.
    pub status: &'static str,
}

/// Port of `HookName`.
pub const HOOK_NAMES: &[&str] = &[
    "before_run",
    "before_resume",
    "before_run_end",
    "transform_context",
    "before_request",
    "before_payload",
    "after_response",
    "before_tool",
    "after_tool",
    "before_compaction",
    "before_navigation",
];

/// Port of `Resources`.
#[derive(Clone, Debug, Default)]
pub struct Resources {
    pub skills: Option<Vec<Skill>>,
    pub prompt_templates: Option<Vec<PromptTemplate>>,
}

/// Port of `AgentHarnessOptions`.
pub struct AgentHarnessOptions {
    pub session: Session,
    pub models: Arc<Models>,
    pub model: Model,
    pub thinking_level: Option<ThinkingLevel>,
    pub active_tool_names: Option<Vec<String>>,
    pub stream_options: Option<SimpleStreamOptions>,
    pub retry: Option<RetryPolicy>,
    pub compaction: Option<CompactionSettings>,
    pub steering_mode: Option<QueueMode>,
    pub follow_up_mode: Option<QueueMode>,
    pub resources: Option<Resources>,
}

/// Port of the `AgentHarness` v2 scaffold.
pub struct AgentHarness {
    session: Session,
    model: std::sync::Mutex<Model>,
    thinking_level: std::sync::Mutex<ThinkingLevel>,
    active_tool_names: std::sync::Mutex<Vec<String>>,
    resources: std::sync::Mutex<Resources>,
    stream_options: std::sync::Mutex<SimpleStreamOptions>,
    retry_policy: std::sync::Mutex<RetryPolicy>,
    compaction_settings: std::sync::Mutex<CompactionSettings>,
    steering_mode: std::sync::Mutex<QueueMode>,
    follow_up_mode: std::sync::Mutex<QueueMode>,
    closed: std::sync::atomic::AtomicBool,
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl AgentHarness {
    /// The lane name this root harness mirrors.
    pub const NAME: &'static str = "main";

    /// Port of `AgentHarness.create`: opens only record-free sessions
    /// before restore is implemented.
    pub async fn create(
        options: AgentHarnessOptions,
    ) -> Result<(Arc<Self>, Vec<SuspendedOperation>), HarnessNotImplemented> {
        let records = options
            .session
            .find_records(Default::default())
            .await
            .unwrap_or_default();
        if !records.is_empty() {
            return Err(HarnessNotImplemented::new("create.restore"));
        }
        let harness = Arc::new(Self {
            session: options.session,
            model: std::sync::Mutex::new(options.model),
            thinking_level: std::sync::Mutex::new(
                options.thinking_level.unwrap_or(ThinkingLevel::Off),
            ),
            active_tool_names: std::sync::Mutex::new(options.active_tool_names.unwrap_or_default()),
            resources: std::sync::Mutex::new(options.resources.unwrap_or_default()),
            stream_options: std::sync::Mutex::new(options.stream_options.unwrap_or_default()),
            retry_policy: std::sync::Mutex::new(options.retry.unwrap_or(RetryPolicy {
                enabled: false,
                max_retries: 0,
                base_delay_ms: 1000,
            })),
            compaction_settings: std::sync::Mutex::new(
                options.compaction.unwrap_or(DEFAULT_COMPACTION_SETTINGS),
            ),
            steering_mode: std::sync::Mutex::new(
                options.steering_mode.unwrap_or(QueueMode::OneAtATime),
            ),
            follow_up_mode: std::sync::Mutex::new(
                options.follow_up_mode.unwrap_or(QueueMode::OneAtATime),
            ),
            closed: std::sync::atomic::AtomicBool::new(false),
        });
        Ok((harness, Vec::new()))
    }

    /// The durable session tree this harness drives.
    pub fn session(&self) -> &Session {
        &self.session
    }

    fn unavailable<T>(&self, operation: &str) -> Result<T, HarnessScaffoldError> {
        if self.is_closed() {
            Err(HarnessScaffoldError::Closed(HarnessClosed))
        } else {
            Err(HarnessScaffoldError::NotImplemented(
                HarnessNotImplemented::new(operation),
            ))
        }
    }

    /// Whether `close` has been called.
    pub fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Port of `hooks.on` / `events.on`: always rejected by the scaffold.
    pub fn register_hook(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("hooks.on")
    }

    /// The events face (also always rejected by the scaffold).
    pub fn register_event_listener(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("events.on")
    }

    /// Port of `getLeafId`.
    pub async fn get_leaf_id(
        &self,
    ) -> Result<Option<String>, crate::agent::harness::session::types::SessionError> {
        self.session.get_leaf_id().await
    }

    // Unimplemented operation surface.
    pub async fn prompt(&self) -> Result<RunResult, HarnessScaffoldError> {
        self.unavailable("prompt")
    }
    pub async fn skill(&self) -> Result<RunResult, HarnessScaffoldError> {
        self.unavailable("skill")
    }
    pub async fn prompt_from_template(&self) -> Result<RunResult, HarnessScaffoldError> {
        self.unavailable("promptFromTemplate")
    }
    pub async fn compact(&self) -> Result<CompactionResult, HarnessScaffoldError> {
        self.unavailable("compact")
    }
    pub async fn navigate_tree(&self) -> Result<NavigationResult, HarnessScaffoldError> {
        self.unavailable("navigateTree")
    }
    pub async fn resume(
        &self,
    ) -> Result<Result<ResumeOutcome, HarnessRejection>, HarnessScaffoldError> {
        self.unavailable("resume")
    }
    pub async fn abort(
        &self,
    ) -> Result<Result<AbortedRun, HarnessRejection>, HarnessScaffoldError> {
        self.unavailable("abort")
    }
    pub async fn steer(&self) -> Result<QueueResult, HarnessScaffoldError> {
        self.unavailable("steer")
    }
    pub async fn follow_up(&self) -> Result<QueueResult, HarnessScaffoldError> {
        self.unavailable("followUp")
    }
    pub async fn next_run(&self) -> Result<QueueResult, HarnessScaffoldError> {
        self.unavailable("nextRun")
    }
    pub async fn cancel_queued(
        &self,
    ) -> Result<Result<CancelQueuedOutcome, HarnessRejection>, HarnessScaffoldError> {
        self.unavailable("cancelQueued")
    }
    pub async fn record_usage(
        &self,
        _usage: Usage,
    ) -> Result<Result<(), HarnessRejection>, HarnessScaffoldError> {
        self.unavailable("recordUsage")
    }
    pub async fn wait_for_idle(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("waitForIdle")
    }
    pub async fn run_when_idle(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("runWhenIdle")
    }
    pub async fn peek_action(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("peekAction")
    }
    pub async fn execute_action(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("executeAction")
    }
    pub async fn run_to_completion(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("runToCompletion")
    }
    pub async fn watch(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("watch")
    }
    pub async fn lane(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("lane")
    }
    pub async fn create_lane(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("createLane")
    }
    pub async fn lanes(&self) -> Result<Vec<LaneInfo>, HarnessScaffoldError> {
        self.unavailable("lanes")
    }
    pub async fn watch_session(&self) -> Result<(), HarnessScaffoldError> {
        self.unavailable("watchSession")
    }

    // Configuration surface (defensive copies like the TypeScript getter
    // and setter pairs).
    pub async fn get_model(&self) -> Model {
        lock(&self.model).clone()
    }
    pub async fn set_model(&self, model: Model) {
        *lock(&self.model) = model;
    }
    pub async fn get_thinking_level(&self) -> ThinkingLevel {
        *lock(&self.thinking_level)
    }
    pub async fn set_thinking_level(&self, level: ThinkingLevel) {
        *lock(&self.thinking_level) = level;
    }
    pub async fn get_active_tools(&self) -> Vec<String> {
        lock(&self.active_tool_names).clone()
    }
    pub async fn set_active_tools(&self, names: Vec<String>) {
        *lock(&self.active_tool_names) = names;
    }
    pub async fn get_resources(&self) -> Resources {
        lock(&self.resources).clone()
    }
    pub async fn set_resources(&self, resources: Resources) {
        *lock(&self.resources) = resources;
    }
    pub async fn get_stream_options(&self) -> SimpleStreamOptions {
        clone_stream_options(&lock(&self.stream_options))
    }
    pub async fn set_stream_options(&self, options: SimpleStreamOptions) {
        *lock(&self.stream_options) = clone_stream_options(&options);
    }
    pub async fn get_retry_policy(&self) -> RetryPolicy {
        *lock(&self.retry_policy)
    }
    pub async fn set_retry_policy(&self, policy: RetryPolicy) {
        *lock(&self.retry_policy) = policy;
    }
    pub async fn get_compaction_settings(&self) -> CompactionSettings {
        *lock(&self.compaction_settings)
    }
    pub async fn set_compaction_settings(&self, settings: CompactionSettings) {
        *lock(&self.compaction_settings) = settings;
    }
    pub async fn get_steering_mode(&self) -> QueueMode {
        *lock(&self.steering_mode)
    }
    pub async fn set_steering_mode(&self, mode: QueueMode) {
        *lock(&self.steering_mode) = mode;
    }
    pub async fn get_follow_up_mode(&self) -> QueueMode {
        *lock(&self.follow_up_mode)
    }
    pub async fn set_follow_up_mode(&self, mode: QueueMode) {
        *lock(&self.follow_up_mode) = mode;
    }

    /// Port of `close`.
    pub async fn close(&self) {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Clones the stream options the way the TypeScript spread does.
fn clone_stream_options(options: &SimpleStreamOptions) -> SimpleStreamOptions {
    options.clone()
}

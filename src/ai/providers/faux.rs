//! Port of `pi-core/ai/src/providers/faux.ts` (message helpers).
//!
//! The faux provider is the package's test double; its message constructors
//! are shared by the retry classification tests and by downstream test
//! suites. The full provider registration lands with the provider runtime.

use crate::ai::types::{
    AssistantContent, AssistantMessage, DeferredHandle, RoleAssistant, StopReason, ToolCall,
    ToolCallArguments, Usage, UsageCost,
};

/// Port of the faux provider's zeroed usage.
pub fn faux_usage() -> Usage {
    Usage {
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        cache_write_1h: None,
        reasoning: None,
        total_tokens: 0,
        cost: UsageCost::default(),
    }
}

/// Port of `fauxText`.
pub fn faux_text(text: impl Into<String>) -> AssistantContent {
    AssistantContent::Text(crate::ai::types::TextContent {
        text: text.into(),
        ..Default::default()
    })
}

/// Port of `fauxThinking`.
pub fn faux_thinking(thinking: impl Into<String>) -> AssistantContent {
    AssistantContent::Thinking(crate::ai::types::ThinkingContent {
        thinking: thinking.into(),
        ..Default::default()
    })
}

/// Port of `fauxToolCall`.
pub fn faux_tool_call(
    name: impl Into<String>,
    arguments: ToolCallArguments,
    id: Option<String>,
) -> AssistantContent {
    AssistantContent::ToolCall(ToolCall {
        content_type: Default::default(),
        id: id.unwrap_or_else(|| format!("tool:{}", crate::ai::utils::uuid::uuidv7())),
        name: name.into(),
        arguments,
        ..Default::default()
    })
}

/// Content accepted by `fauxAssistantMessage`.
pub enum FauxContent {
    Text(String),
    Blocks(Vec<AssistantContent>),
}

impl From<&str> for FauxContent {
    fn from(value: &str) -> Self {
        FauxContent::Text(value.to_string())
    }
}

impl From<String> for FauxContent {
    fn from(value: String) -> Self {
        FauxContent::Text(value)
    }
}

impl From<Vec<AssistantContent>> for FauxContent {
    fn from(value: Vec<AssistantContent>) -> Self {
        FauxContent::Blocks(value)
    }
}

impl From<AssistantContent> for FauxContent {
    fn from(value: AssistantContent) -> Self {
        FauxContent::Blocks(vec![value])
    }
}

/// Options accepted by `fauxAssistantMessage`.
#[derive(Default)]
pub struct FauxMessageOptions {
    pub stop_reason: Option<StopReason>,
    pub deferred: Option<DeferredHandle>,
    pub error_message: Option<String>,
    pub response_id: Option<String>,
    pub timestamp: Option<i64>,
}

/// Port of `fauxAssistantMessage`.
pub fn faux_assistant_message(
    content: impl Into<FauxContent>,
    options: FauxMessageOptions,
) -> AssistantMessage {
    let content = match content.into() {
        FauxContent::Text(text) => vec![faux_text(text)],
        FauxContent::Blocks(blocks) => blocks,
    };
    AssistantMessage {
        role: RoleAssistant,
        content,
        api: "faux".to_string(),
        provider: "faux".to_string(),
        model: "faux-1".to_string(),
        usage: faux_usage(),
        stop_reason: options.stop_reason.unwrap_or(StopReason::Stop),
        deferred: options.deferred,
        error_message: options.error_message,
        response_id: options.response_id,
        timestamp: options.timestamp.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or_default()
        }),
        ..Default::default()
    }
}

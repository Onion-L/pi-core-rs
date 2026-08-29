//! Port of `pi-core/ai/src/utils/deferred-tools.ts`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ai::types::{AssistantContent, Context, Message, Tool};

/// Port of `splitDeferredTools`: splits current tools into prefix and
/// transcript-loaded definitions.
pub fn split_deferred_tools(
    context: &Context,
    enabled: bool,
    normalize_name: impl Fn(&str) -> String,
) -> DeferredToolSplit {
    let mut unique_tools: BTreeMap<String, Tool> = BTreeMap::new();
    for tool in context.tools.iter().flatten() {
        unique_tools.insert(normalize_name(&tool.name), tool.clone());
    }
    if !enabled {
        return DeferredToolSplit {
            immediate: unique_tools.into_values().collect(),
            deferred: HashMap::new(),
        };
    }

    let mut deferred_names = BTreeSet::new();
    let mut used_names = BTreeSet::new();
    for message in &context.messages {
        match message {
            Message::Assistant(assistant) => {
                for block in &assistant.content {
                    if let AssistantContent::ToolCall(tool_call) = block {
                        used_names.insert(normalize_name(&tool_call.name));
                    }
                }
            }
            Message::ToolResult(result) => {
                for name in result.added_tool_names.iter().flatten() {
                    let normalized_name = normalize_name(name);
                    if !used_names.contains(&normalized_name) {
                        deferred_names.insert(normalized_name);
                    }
                }
            }
            Message::User(_) => {}
        }
    }

    let mut immediate = Vec::new();
    let mut deferred = HashMap::new();
    for (name, tool) in unique_tools {
        if deferred_names.contains(&name) {
            deferred.insert(name, tool);
        } else {
            immediate.push(tool);
        }
    }
    DeferredToolSplit {
        immediate,
        deferred,
    }
}

/// Port of the `{ immediate, deferred }` result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeferredToolSplit {
    pub immediate: Vec<Tool>,
    /// Deferred tools keyed by normalized tool name.
    pub deferred: HashMap<String, Tool>,
}

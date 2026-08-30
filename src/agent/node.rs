//! Port of `pi-core/agent/src/node.ts`: the Node-facing entry point
//! re-exports the Node execution environment plus the crate root face.

pub use super::harness::env::nodejs::NodeExecutionEnv;

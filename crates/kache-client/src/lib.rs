//! Compiler invocation capture and synchronous handoff to a running Kache daemon.

mod execution;
pub use execution::Execution;

mod stdio;
pub use stdio::Stdio;
mod creation_mask;
pub use creation_mask::creation_mask;
mod frontend;
pub use frontend::run;

/// Internal argv envelope passed from the frontend to its companion engine.
pub const FORWARDED_ARGV_FLAG: &str = "--kache-forwarded-argv-v1";

mod entry;
pub use entry::{EngineEntry, engine_entry};

include!(concat!(env!("OUT_DIR"), "/commands.rs"));

//! Recovery subsystem: journal, manager and restoration contracts.

pub mod journal;
pub mod manager;

pub use manager::{CapturedState, RecoveryManager, RestorationExecutor, StrictNoOpExecutor};

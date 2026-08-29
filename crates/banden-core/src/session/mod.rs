//! Session subsystem: the pure state machine plus its orchestrating manager.

pub mod machine;
pub mod manager;

pub use machine::{SessionState, SessionStateMachine, Transition};
pub use manager::{Session, SessionManager, StartSessionRequest, StopReason};

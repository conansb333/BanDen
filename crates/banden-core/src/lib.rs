//! BanDen domain core.
//!
//! This crate contains the stateful, dangerous logic of BanDen: the session
//! lifecycle state machine, the recovery manager, the emergency-stop
//! orchestration and the traffic aggregation engine.
//!
//! Everything here is transport-agnostic. The crate depends only on ports
//! (traits) that the application layer implements: persistence, event
//! emission and the actual control backend. This keeps the critical state
//! transitions fully unit-testable without hardware.

pub mod error;
pub mod events;
pub mod models;
pub mod ports;
pub mod recovery;
pub mod runtime;
pub mod session;
pub mod simulation;
pub mod traffic;

pub use error::CoreError;
pub use events::CoreEvent;
pub use models::*;
pub use ports::{ActivityLog, ControlBackend, EventSink, JournalStore, SystemClock};
pub use recovery::journal::{JournalEntry, JournalEntryState, RestorationAction};
pub use recovery::manager::{RecoveryManager, RestorationExecutor};
pub use runtime::{CoreRuntime, EmergencyStopOutcome, EmergencyStopStage, StatusInputs};
pub use session::machine::{SessionState, SessionStateMachine, Transition};
pub use session::manager::{Session, SessionManager, StopReason};
pub use simulation::{FlakyControlBackend, SimulatedControlBackend};
pub use traffic::TrafficAggregator;

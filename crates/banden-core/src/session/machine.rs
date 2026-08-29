//! The session lifecycle state machine.
//!
//! A network-control operation is never a boolean flag. It is an explicit
//! state machine whose legal transitions are defined in exactly one place
//! and fully covered by unit tests.
//!
//! ```text
//!                 ┌──────────┐
//!    prepare ok   │          │  prepare failed
//!   ┌────────────►│  Active  │────────────┐
//!   │             │          │            ▼
//! ┌─┴────────┐    └────┬─────┘       ┌─────────┐
//! │Preparing │         │ stop        │  Failed │ (terminal)
//! └──┬───────┘         ▼             └─────────┘
//!    │ cancel   ┌──────────┐
//!    └─────────►│ Stopping │
//!               └────┬─────┘
//!                    ▼
//!               ┌───────────┐   exhausted
//!               │ Restoring │──retries──► Failed
//!               └────┬──────┘
//!                    ▼
//!               ┌───────────┐
//!               │ Verifying │
//!               └────┬──────┘
//!            verify ok│  verify failed
//!                    ▼                ▼
//!              ┌──────────┐    ┌─────────────────┐
//!              │Completed │    │RecoveryRequired │──► Restoring
//!              └──────────┘    └─────────────────┘
//! ```
//!
//! `Active -> RecoveryRequired` models an unexpected fault detected while the
//! session is running; restoration must then be driven by the recovery
//! manager, not by the normal stop path.

use crate::error::{CoreError, CoreResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Preparing,
    Active,
    Stopping,
    Restoring,
    Verifying,
    Completed,
    RecoveryRequired,
    Failed,
}

impl SessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionState::Preparing => "preparing",
            SessionState::Active => "active",
            SessionState::Stopping => "stopping",
            SessionState::Restoring => "restoring",
            SessionState::Verifying => "verifying",
            SessionState::Completed => "completed",
            SessionState::RecoveryRequired => "recovery_required",
            SessionState::Failed => "failed",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, SessionState::Completed | SessionState::Failed)
    }

    /// The full transition table. Single source of truth.
    pub fn allowed_targets(self) -> &'static [SessionState] {
        use SessionState::*;
        match self {
            Preparing => &[Active, Stopping, Failed],
            Active => &[Stopping, RecoveryRequired, Failed],
            Stopping => &[Restoring, Failed],
            Restoring => &[Verifying, Failed],
            Verifying => &[Completed, RecoveryRequired, Failed],
            RecoveryRequired => &[Restoring, Failed],
            Completed => &[],
            Failed => &[],
        }
    }
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A recorded transition, used for events and history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Transition {
    pub from: SessionState,
    pub to: SessionState,
    pub at: DateTime<Utc>,
    pub reason: Option<String>,
}

/// Pure state machine guard. Holds no I/O, no locks, no side effects.
#[derive(Debug, Clone)]
pub struct SessionStateMachine {
    state: SessionState,
    history: Vec<Transition>,
}

impl SessionStateMachine {
    pub fn new() -> Self {
        Self {
            state: SessionState::Preparing,
            history: Vec::new(),
        }
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn history(&self) -> &[Transition] {
        &self.history
    }

    pub fn can_transition(&self, to: SessionState) -> bool {
        self.state.allowed_targets().contains(&to)
    }

    /// Apply a transition if legal; error otherwise. Never silently coerces.
    pub fn transition(
        &mut self,
        to: SessionState,
        at: DateTime<Utc>,
        reason: Option<String>,
    ) -> CoreResult<Transition> {
        if !self.can_transition(to) {
            return Err(CoreError::InvalidTransition {
                from: self.state.as_str(),
                to: to.as_str(),
            });
        }
        let t = Transition {
            from: self.state,
            to,
            at,
            reason,
        };
        self.state = to;
        self.history.push(t.clone());
        Ok(t)
    }
}

impl Default for SessionStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    fn assert_illegal(from: SessionState, path: &[SessionState], to: SessionState) {
        let mut m = SessionStateMachine::new();
        for step in path {
            m.transition(*step, t(), None).unwrap();
        }
        assert_eq!(m.state(), from);
        let err = m.transition(to, t(), None).unwrap_err();
        match err {
            CoreError::InvalidTransition {
                from: f,
                to: target,
            } => {
                assert_eq!(f, from.as_str());
                assert_eq!(target, to.as_str());
            }
            other => panic!("expected InvalidTransition, got {other:?}"),
        }
    }

    #[test]
    fn happy_path() {
        use SessionState::*;
        let mut m = SessionStateMachine::new();
        for step in [Active, Stopping, Restoring, Verifying, Completed] {
            m.transition(step, t(), None)
                .unwrap_or_else(|e| panic!("{e}"));
        }
        assert!(m.state().is_terminal());
        assert_eq!(m.history().len(), 5);
    }

    #[test]
    fn cancel_while_preparing() {
        use SessionState::*;
        let mut m = SessionStateMachine::new();
        m.transition(Stopping, t(), Some("cancelled".into()))
            .unwrap();
        m.transition(Restoring, t(), None).unwrap();
        m.transition(Verifying, t(), None).unwrap();
        m.transition(Completed, t(), None).unwrap();
    }

    #[test]
    fn active_to_recovery_required_then_restored() {
        use SessionState::*;
        let mut m = SessionStateMachine::new();
        m.transition(Active, t(), None).unwrap();
        m.transition(RecoveryRequired, t(), Some("fault".into()))
            .unwrap();
        m.transition(Restoring, t(), None).unwrap();
        m.transition(Verifying, t(), None).unwrap();
        m.transition(Completed, t(), None).unwrap();
    }

    #[test]
    fn verification_failure_forces_recovery() {
        use SessionState::*;
        let mut m = SessionStateMachine::new();
        for step in [Active, Stopping, Restoring, Verifying] {
            m.transition(step, t(), None).unwrap();
        }
        m.transition(RecoveryRequired, t(), Some("verify failed".into()))
            .unwrap();
    }

    #[test]
    fn terminal_states_are_absorbing() {
        use SessionState::*;
        assert_illegal(
            SessionState::Completed,
            &[Active, Stopping, Restoring, Verifying, Completed],
            SessionState::Active,
        );
        assert_illegal(SessionState::Failed, &[Failed], SessionState::Active);
    }

    #[test]
    fn illegal_skips_rejected() {
        use SessionState::*;
        // Preparing must not jump to Completed/Verifying/Restoring.
        assert_illegal(Preparing, &[], Completed);
        assert_illegal(Preparing, &[], Verifying);
        assert_illegal(Preparing, &[], Restoring);
        assert_illegal(Preparing, &[], RecoveryRequired);
        // Active cannot directly complete: restoration is mandatory.
        assert_illegal(Active, &[Active], Completed);
        assert_illegal(Active, &[Active], Verifying);
        assert_illegal(Active, &[Active], Restoring);
        // Stopping cannot skip restoration.
        assert_illegal(Stopping, &[Active, Stopping], Completed);
    }

    #[test]
    fn serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&SessionState::RecoveryRequired).unwrap(),
            "\"recovery_required\""
        );
    }
}

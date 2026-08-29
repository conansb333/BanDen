//! Simulated control backend.
//!
//! This is the default backend when BanDen runs in *lab mode* (the default
//! safety setting). It performs no system mutation at all: the entire
//! session lifecycle — capture, journal registration, restoration and
//! verification — is exercised against in-memory state.
//!
//! This keeps the safety architecture honest and testable, and it is the
//! recommended mode for learning, demos and development. Backends that
//! mutate real system state are provided by the network layer and are only
//! enabled after an explicit authorization step.

use crate::error::{CoreError, CoreResult};
use crate::ports::ControlBackend;
use crate::recovery::manager::CapturedState;
use crate::session::Session;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Default)]
pub struct SimulatedControlBackend {
    pub applied: Mutex<HashSet<uuid::Uuid>>,
}

impl SimulatedControlBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_applied(&self, id: uuid::Uuid) -> bool {
        self.applied.lock().unwrap().contains(&id)
    }
}

#[async_trait]
impl ControlBackend for SimulatedControlBackend {
    fn name(&self) -> &'static str {
        "simulation"
    }

    async fn prepare(&self, _session: &Session) -> CoreResult<CapturedState> {
        // Nothing is mutated, so the only restoration needed is our own
        // bookkeeping.
        Ok(CapturedState {
            description: "simulated session (no system state modified)".into(),
            actions: vec![],
        })
    }

    async fn apply(&self, session: &Session) -> CoreResult<()> {
        self.applied.lock().unwrap().insert(session.id);
        tracing::debug!(session = %session.id, "simulated control applied");
        Ok(())
    }

    async fn teardown(&self, session: &Session) -> CoreResult<()> {
        self.applied.lock().unwrap().remove(&session.id);
        Ok(())
    }

    async fn verify_restoration(
        &self,
        session_id: uuid::Uuid,
        _captured: &CapturedState,
    ) -> CoreResult<()> {
        if self.applied.lock().unwrap().contains(&session_id) {
            return Err(CoreError::VerificationFailed(
                "simulated control still applied".into(),
            ));
        }
        Ok(())
    }
}

/// A backend that fails at a chosen phase; used to test failure recovery.
#[derive(Default)]
pub struct FlakyControlBackend {
    pub fail_prepare: bool,
    pub fail_apply: bool,
    pub fail_teardown: bool,
    pub fail_verify: bool,
    pub applied: Mutex<HashSet<uuid::Uuid>>,
}

#[async_trait]
impl ControlBackend for FlakyControlBackend {
    fn name(&self) -> &'static str {
        "flaky-test"
    }

    async fn prepare(&self, _s: &Session) -> CoreResult<CapturedState> {
        if self.fail_prepare {
            return Err(CoreError::Backend("injected prepare failure".into()));
        }
        Ok(CapturedState {
            description: "flaky".into(),
            actions: vec![],
        })
    }

    async fn apply(&self, s: &Session) -> CoreResult<()> {
        if self.fail_apply {
            return Err(CoreError::Backend("injected apply failure".into()));
        }
        self.applied.lock().unwrap().insert(s.id);
        Ok(())
    }

    async fn teardown(&self, _s: &Session) -> CoreResult<()> {
        if self.fail_teardown {
            return Err(CoreError::Backend("injected teardown failure".into()));
        }
        self.applied.lock().unwrap().clear();
        Ok(())
    }

    async fn verify_restoration(&self, _id: uuid::Uuid, _c: &CapturedState) -> CoreResult<()> {
        if self.fail_verify {
            return Err(CoreError::VerificationFailed(
                "injected verify failure".into(),
            ));
        }
        Ok(())
    }
}

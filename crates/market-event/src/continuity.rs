//! Sequence continuity with evidence-gated recovery.
//!
//! A stream never becomes clean again from a normal-looking sequence.
//! Only an authoritative snapshot or reset evidence token may clear a gap.

use serde::{
    Deserialize, //
    Serialize,
};

use crate::error::ContractError;

/// Stream continuity state. `Clean` is the only risk-increasing state.
///
/// `Unverified` is the fail-closed state for a source that declares no
/// sequence scope. Receipt order may still be recorded, but exchange-level
/// packet completeness is not claimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamState {
    Unverified,
    Clean,
    Gap,
    Recovering,
    Recovered,
}

impl StreamState {
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }

    /// Whether a normal in-order observation is sufficient to advance.
    #[must_use]
    pub const fn accepts_sequence_observation(&self) -> bool {
        matches!(self, Self::Clean | Self::Recovering)
    }
}

/// Result of observing a sequence under a connection epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationOutcome {
    Initial,
    InOrder,
    Duplicate,
    OutOfOrder,
    Gap,
    ConnectionReset,
    Recovered,
}

/// Per-epoch sequence tracker with a fail-closed state machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContinuityMachine {
    state: StreamState,
    connection_epoch: u64,
    highest: Option<u64>,
    recovery_epoch: Option<u64>,
    recovery_authority: Option<crate::identity::ContentId>,
}

impl Default for ContinuityMachine {
    fn default() -> Self {
        Self {
            state: StreamState::Clean,
            connection_epoch: 0,
            highest: None,
            recovery_epoch: None,
            recovery_authority: None,
        }
    }
}

impl ContinuityMachine {
    #[must_use]
    pub const fn state(&self) -> StreamState {
        self.state
    }

    #[must_use]
    pub const fn highest(&self) -> Option<u64> {
        self.highest
    }

    /// A decreasing sequence under the same epoch is out-of-order, not a gap
    /// and not an identity change. A new epoch is a connection boundary and
    /// resets sequence state but not continuity evidence.
    pub fn observe(
        &mut self,
        connection_epoch: u64,
        sequence: u64,
    ) -> Result<ObservationOutcome, ContractError> {
        if connection_epoch != self.connection_epoch {
            self.connection_epoch = connection_epoch;
            self.highest = Some(sequence);
            self.recovery_epoch = None;
            self.recovery_authority = None;
            if self.state == StreamState::Clean {
                return Ok(ObservationOutcome::ConnectionReset);
            }
            // A reconnect does not by itself prove recovery.
            self.state = StreamState::Recovering;
            return Ok(ObservationOutcome::ConnectionReset);
        }

        let Some(highest) = self.highest else {
            self.highest = Some(sequence);
            return Ok(ObservationOutcome::Initial);
        };

        if sequence == highest {
            return Ok(ObservationOutcome::Duplicate);
        }
        if sequence < highest {
            return Ok(ObservationOutcome::OutOfOrder);
        }
        if sequence > highest + 1 {
            self.highest = Some(sequence);
            self.state = StreamState::Gap;
            self.recovery_epoch = None;
            self.recovery_authority = None;
            return Ok(ObservationOutcome::Gap);
        }

        self.highest = Some(sequence);
        match self.state {
            StreamState::Clean => Ok(ObservationOutcome::InOrder),
            StreamState::Recovering
            | StreamState::Gap
            | StreamState::Recovered
            | StreamState::Unverified => {
                // Sequence order alone cannot clear an unknown state.
                Ok(ObservationOutcome::InOrder)
            }
        }
    }

    /// Accept an authoritative snapshot or reset as recovery evidence.
    pub fn recover(
        &mut self,
        connection_epoch: u64,
        authority: crate::identity::ContentId,
    ) -> Result<(), ContractError> {
        if connection_epoch == 0 || connection_epoch != self.connection_epoch {
            return Err(ContractError::RecoveryWithoutEvidence);
        }
        self.recovery_authority = Some(authority);
        self.state = StreamState::Recovered;
        self.recovery_epoch = Some(connection_epoch);
        Ok(())
    }

    /// Only after recovery evidence and an in-order observation in the same
    /// epoch may the stream return to clean.
    pub fn clear_recovery(&mut self) -> Result<(), ContractError> {
        if self.state != StreamState::Recovered
            || self.recovery_epoch != Some(self.connection_epoch)
            || self.recovery_authority.is_none()
        {
            return Err(ContractError::GapToClean);
        }
        self.state = StreamState::Clean;
        self.recovery_epoch = None;
        self.recovery_authority = None;
        Ok(())
    }
}

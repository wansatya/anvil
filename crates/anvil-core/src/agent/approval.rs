//! Approval plumbing: the loop pauses on `ask` tools until the UI
//! resolves the decision (SPEC §12). Cancellation also flows through here.

use tokio::sync::{mpsc, oneshot, watch};

use crate::events::{ApprovalDecision, ApprovalRequest};
use crate::agent::AgentError;

/// One paused approval: the request for the UI + the reply channel.
pub struct ApprovalJob {
    pub request: ApprovalRequest,
    pub reply: oneshot::Sender<ApprovalDecision>,
}

#[derive(Clone)]
pub struct ApprovalGate {
    tx: mpsc::UnboundedSender<ApprovalJob>,
}

impl ApprovalGate {
    pub fn new(tx: mpsc::UnboundedSender<ApprovalJob>) -> Self {
        Self { tx }
    }

    /// Ask the UI; resolves to the user's decision or `Cancelled`.
    pub async fn ask(
        &self,
        request: ApprovalRequest,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<ApprovalDecision, AgentError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ApprovalJob { request, reply: reply_tx })
            .map_err(|_| AgentError::Approval("UI went away".into()))?;
        tokio::select! {
            biased;
            _ = cancel.changed() => Err(AgentError::Cancelled),
            r = reply_rx => r.map_err(|_| AgentError::Approval("approval unanswered".into())),
        }
    }
}

use crate::sandbox::NetworkPolicy;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);
const TICKET_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    WorkspaceRead,
    WorkspaceWrite,
    ProcessExecute,
    JobControl,
    GitRead,
    GitLocalWrite,
    GitRemoteWrite,
    NetworkOutbound,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceRead => "workspace.read",
            Self::WorkspaceWrite => "workspace.write",
            Self::ProcessExecute => "process.execute",
            Self::JobControl => "job.control",
            Self::GitRead => "git.read",
            Self::GitLocalWrite => "git.local.write",
            Self::GitRemoteWrite => "git.remote.write",
            Self::NetworkOutbound => "network.outbound",
        }
    }

    pub fn requires_approval(self) -> bool {
        matches!(
            self,
            Self::ProcessExecute
                | Self::GitLocalWrite
                | Self::GitRemoteWrite
                | Self::NetworkOutbound
        )
    }
}

#[derive(Debug, Clone)]
pub struct ExecAuthorization {
    pub capability: Capability,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub background: bool,
    pub network: NetworkPolicy,
}

#[derive(Debug, Serialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub summary: String,
    pub reason: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Error)]
pub enum PermissionError {
    #[error("approval ticket not found")]
    NotFound,
    #[error("approval ticket expired")]
    Expired,
    #[error("approval ticket does not match this execution request")]
    Mismatch,
    #[error("approval ticket has not been approved")]
    NotApproved,
    #[error("capability requires explicit approval: {0}")]
    ApprovalRequired(&'static str),
    #[error("failed to initialize approval secret")]
    Random,
}

struct Ticket {
    digest: [u8; 32],
    expires: Instant,
    approved: bool,
}

pub struct PermissionEngine {
    secret: [u8; 32],
    tickets: HashMap<String, Ticket>,
}

impl PermissionEngine {
    pub fn new() -> Result<Self, PermissionError> {
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(|_| PermissionError::Random)?;
        Ok(Self {
            secret,
            tickets: HashMap::new(),
        })
    }

    pub fn authorize(&self, capability: Capability) -> Result<(), PermissionError> {
        if capability.requires_approval() {
            return Err(PermissionError::ApprovalRequired(capability.as_str()));
        }
        Ok(())
    }

    pub fn request_action(
        &mut self,
        request: &ExecAuthorization,
        summary: String,
        reason: String,
    ) -> ApprovalRequest {
        self.cleanup();
        let id = format!(
            "apr_{}_{}",
            std::process::id(),
            NEXT_TICKET.fetch_add(1, Ordering::Relaxed)
        );
        self.tickets.insert(
            id.clone(),
            Ticket {
                digest: self.digest(request),
                expires: Instant::now() + TICKET_TTL,
                approved: false,
            },
        );
        ApprovalRequest {
            id,
            summary,
            reason,
            expires_in_seconds: TICKET_TTL.as_secs(),
        }
    }

    pub fn approve(&mut self, id: &str) -> Result<(), PermissionError> {
        self.cleanup();
        let ticket = self.tickets.get_mut(id).ok_or(PermissionError::NotFound)?;
        ticket.approved = true;
        Ok(())
    }

    pub fn deny(&mut self, id: &str) -> Result<(), PermissionError> {
        self.cleanup();
        self.tickets.remove(id).ok_or(PermissionError::NotFound)?;
        Ok(())
    }

    pub fn consume_exec(
        &mut self,
        id: &str,
        request: &ExecAuthorization,
    ) -> Result<(), PermissionError> {
        self.cleanup();
        let ticket = self.tickets.get(id).ok_or(PermissionError::NotFound)?;
        if ticket.expires <= Instant::now() {
            self.tickets.remove(id);
            return Err(PermissionError::Expired);
        }
        if !ticket.approved {
            return Err(PermissionError::NotApproved);
        }
        if ticket.digest != self.digest(request) {
            return Err(PermissionError::Mismatch);
        }
        self.tickets.remove(id);
        Ok(())
    }

    fn digest(&self, request: &ExecAuthorization) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.secret);
        hasher.update(request.capability.as_str().as_bytes());
        for arg in &request.argv {
            hasher.update((arg.len() as u64).to_le_bytes());
            hasher.update(arg.as_bytes());
        }
        match &request.cwd {
            Some(cwd) => {
                hasher.update([1]);
                hasher.update((cwd.len() as u64).to_le_bytes());
                hasher.update(cwd.as_bytes());
            }
            None => hasher.update([0]),
        }
        hasher.update([request.background as u8]);
        hasher.update(request.network.as_str().as_bytes());
        hasher.finalize().into()
    }

    fn cleanup(&mut self) {
        let now = Instant::now();
        self.tickets.retain(|_, ticket| ticket.expires > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(argv: &[&str]) -> ExecAuthorization {
        ExecAuthorization {
            capability: Capability::ProcessExecute,
            argv: argv.iter().map(|value| value.to_string()).collect(),
            cwd: Some(".".into()),
            background: false,
            network: NetworkPolicy::Deny,
        }
    }

    fn request_ticket(
        engine: &mut PermissionEngine,
        request: &ExecAuthorization,
    ) -> ApprovalRequest {
        engine.request_action(request, "test".into(), "test".into())
    }

    #[test]
    fn approval_is_bound_to_exact_request_and_single_use() {
        let mut engine = PermissionEngine::new().unwrap();
        let first = request(&["cargo", "test"]);
        let ticket = request_ticket(&mut engine, &first);
        engine.approve(&ticket.id).unwrap();
        assert!(matches!(
            engine.consume_exec(&ticket.id, &request(&["cargo", "check"])),
            Err(PermissionError::Mismatch)
        ));
    }

    #[test]
    fn approval_is_bound_to_capability() {
        let mut engine = PermissionEngine::new().unwrap();
        let request = request(&["git", "push"]);
        let ticket = engine.request_action(&request, "test".into(), "test".into());
        engine.approve(&ticket.id).unwrap();
        let mut changed = request.clone();
        changed.capability = Capability::GitRemoteWrite;
        assert!(matches!(
            engine.consume_exec(&ticket.id, &changed),
            Err(PermissionError::Mismatch)
        ));
    }

    #[test]
    fn approved_ticket_can_be_consumed_once() {
        let mut engine = PermissionEngine::new().unwrap();
        let request = request(&["cargo", "test"]);
        let ticket = request_ticket(&mut engine, &request);
        engine.approve(&ticket.id).unwrap();
        engine.consume_exec(&ticket.id, &request).unwrap();
        assert!(matches!(
            engine.consume_exec(&ticket.id, &request),
            Err(PermissionError::NotFound)
        ));
    }

    #[test]
    fn mismatch_does_not_consume_ticket() {
        let mut engine = PermissionEngine::new().unwrap();
        let approved = request(&["cargo", "test"]);
        let ticket = request_ticket(&mut engine, &approved);
        engine.approve(&ticket.id).unwrap();

        // A mismatching consume must NOT invalidate the ticket.
        assert!(matches!(
            engine.consume_exec(&ticket.id, &request(&["cargo", "check"])),
            Err(PermissionError::Mismatch)
        ));

        // The original approved request is still usable.
        engine.consume_exec(&ticket.id, &approved).unwrap();

        // But only once.
        assert!(matches!(
            engine.consume_exec(&ticket.id, &approved),
            Err(PermissionError::NotFound)
        ));
    }

    #[test]
    fn capability_mismatch_does_not_consume_ticket() {
        let mut engine = PermissionEngine::new().unwrap();
        let mut approved = request(&["git", "push"]);
        approved.capability = Capability::GitLocalWrite;
        let ticket = engine.request_action(&approved, "test".into(), "test".into());
        engine.approve(&ticket.id).unwrap();

        let mut wrong_capability = approved.clone();
        wrong_capability.capability = Capability::GitRemoteWrite;
        assert!(matches!(
            engine.consume_exec(&ticket.id, &wrong_capability),
            Err(PermissionError::Mismatch)
        ));

        engine.consume_exec(&ticket.id, &approved).unwrap();
        assert!(matches!(
            engine.consume_exec(&ticket.id, &approved),
            Err(PermissionError::NotFound)
        ));
    }

    #[test]
    fn network_policy_mismatch_does_not_consume_ticket() {
        let mut engine = PermissionEngine::new().unwrap();
        let mut approved = request(&["cargo", "fetch"]);
        approved.capability = Capability::NetworkOutbound;
        approved.network = NetworkPolicy::Outbound;
        let ticket = request_ticket(&mut engine, &approved);
        engine.approve(&ticket.id).unwrap();

        let mut denied_network = approved.clone();
        denied_network.network = NetworkPolicy::Deny;
        assert!(matches!(
            engine.consume_exec(&ticket.id, &denied_network),
            Err(PermissionError::Mismatch)
        ));

        engine.consume_exec(&ticket.id, &approved).unwrap();
    }

    #[test]
    fn not_approved_ticket_is_preserved_until_approval() {
        let mut engine = PermissionEngine::new().unwrap();
        let req = request(&["cargo", "test"]);
        let ticket = request_ticket(&mut engine, &req);

        // Consuming before approval must not destroy the ticket.
        assert!(matches!(
            engine.consume_exec(&ticket.id, &req),
            Err(PermissionError::NotApproved)
        ));

        engine.approve(&ticket.id).unwrap();
        engine.consume_exec(&ticket.id, &req).unwrap();
    }
}

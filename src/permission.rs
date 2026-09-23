use crate::approval_store::{ApprovalRecord, ApprovalStore, MAX_PENDING_TICKETS};
use crate::sandbox::NetworkPolicy;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);
const TICKET_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    WorkspaceRead,
    WorkspaceSensitiveRead,
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
            Self::WorkspaceSensitiveRead => "workspace.sensitive.read",
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
            Self::WorkspaceSensitiveRead
                | Self::ProcessExecute
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
    pub expected_head: Option<String>,
    pub stdin: Option<String>,
    pub protected_read: bool,
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
    #[error("approval ticket capacity reached")]
    Limit,
    #[error("approval control store failed: {0}")]
    Store(#[from] std::io::Error),
}

struct Ticket {
    digest: [u8; 32],
    expires: Instant,
    approved: bool,
}

pub struct PermissionEngine {
    secret: [u8; 32],
    tickets: HashMap<String, Ticket>,
    approval_store: Option<ApprovalStore>,
    workspace_root: Option<PathBuf>,
}

impl PermissionEngine {
    pub fn for_workspace(workspace: &crate::workspace::Workspace) -> Result<Self, PermissionError> {
        Self::with_workspace(Some(workspace.root().to_path_buf()))
    }

    #[cfg(test)]
    pub fn new() -> Result<Self, PermissionError> {
        Self::with_workspace(None)
    }

    fn with_workspace(workspace_root: Option<PathBuf>) -> Result<Self, PermissionError> {
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(|_| PermissionError::Random)?;
        Ok(Self {
            secret,
            tickets: HashMap::new(),
            approval_store: None,
            workspace_root,
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
    ) -> Result<ApprovalRequest, PermissionError> {
        self.cleanup();
        if self.tickets.len() >= MAX_PENDING_TICKETS {
            return Err(PermissionError::Limit);
        }
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| PermissionError::Random)?;
        let nonce = format!("{:032x}", u128::from_ne_bytes(nonce));
        let id = format!(
            "apr_{}_{}_{}",
            std::process::id(),
            NEXT_TICKET.fetch_add(1, Ordering::Relaxed),
            nonce
        );
        let expires_at = Instant::now() + TICKET_TTL;
        let wall_now = std::time::SystemTime::now();
        let expires_unix_s = wall_now
            .checked_add(TICKET_TTL)
            .unwrap_or(wall_now)
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if self.approval_store.is_none() {
            self.approval_store = Some(ApprovalStore::new(self.workspace_root.as_deref())?);
        }
        let store = self
            .approval_store
            .as_ref()
            .ok_or_else(|| std::io::Error::other("approval store unavailable"))?;
        store.publish(&ApprovalRecord {
            ticket_id: id.clone(),
            capability: request.capability.as_str().into(),
            summary: summary.clone(),
            expires_unix_s,
        })?;
        self.tickets.insert(
            id.clone(),
            Ticket {
                digest: self.digest(request),
                expires: expires_at,
                approved: false,
            },
        );
        Ok(ApprovalRequest {
            id,
            summary,
            reason,
            expires_in_seconds: TICKET_TTL.as_secs(),
        })
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
        if let Some(store) = &self.approval_store {
            store.consume(id);
        }
        Ok(())
    }

    pub fn consume_exec(
        &mut self,
        id: &str,
        request: &ExecAuthorization,
    ) -> Result<(), PermissionError> {
        self.validate_exec(id, request)?;
        self.tickets.remove(id);
        if let Some(store) = &self.approval_store {
            store.consume(id);
        }
        Ok(())
    }

    pub fn consume_execs(
        &mut self,
        requests: &[(String, ExecAuthorization)],
    ) -> Result<(), PermissionError> {
        for (id, request) in requests {
            self.validate_exec(id, request)?;
        }
        for (id, _) in requests {
            self.tickets.remove(id);
            if let Some(store) = &self.approval_store {
                store.consume(id);
            }
        }
        Ok(())
    }

    pub fn validate_exec(
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
        let local_approval = match &self.approval_store {
            Some(store) => store.is_approved(id)?,
            None => false,
        };
        if !ticket.approved && !local_approval {
            return Err(PermissionError::NotApproved);
        }
        if ticket.digest != self.digest(request) {
            return Err(PermissionError::Mismatch);
        }
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
        match &request.expected_head {
            Some(head) => {
                hasher.update([1]);
                hasher.update((head.len() as u64).to_le_bytes());
                hasher.update(head.as_bytes());
            }
            None => hasher.update([0]),
        }
        match &request.stdin {
            Some(stdin) => {
                hasher.update([1]);
                hasher.update((stdin.len() as u64).to_le_bytes());
                hasher.update(stdin.as_bytes());
            }
            None => hasher.update([0]),
        }
        hasher.update([request.protected_read as u8]);
        hasher.finalize().into()
    }

    fn cleanup(&mut self) {
        let now = Instant::now();
        let expired = self
            .tickets
            .iter()
            .filter(|(_, ticket)| ticket.expires <= now)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        self.tickets.retain(|_, ticket| ticket.expires > now);
        if let Some(store) = &self.approval_store {
            for id in expired {
                store.consume(&id);
            }
        }
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
            expected_head: None,
            stdin: None,
            protected_read: false,
        }
    }

    fn request_ticket(
        engine: &mut PermissionEngine,
        request: &ExecAuthorization,
    ) -> ApprovalRequest {
        engine
            .request_action(request, "test".into(), "test".into())
            .unwrap()
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
        let ticket = engine
            .request_action(&request, "test".into(), "test".into())
            .unwrap();
        engine.approve(&ticket.id).unwrap();
        let mut changed = request.clone();
        changed.capability = Capability::GitRemoteWrite;
        assert!(matches!(
            engine.consume_exec(&ticket.id, &changed),
            Err(PermissionError::Mismatch)
        ));
    }

    #[test]
    fn approval_is_bound_to_expected_head() {
        let mut engine = PermissionEngine::new().unwrap();
        let mut request = request(&["git", "commit", "--", "file.txt"]);
        request.capability = Capability::GitLocalWrite;
        request.expected_head = Some("abc123".into());
        let ticket = request_ticket(&mut engine, &request);
        engine.approve(&ticket.id).unwrap();

        let mut changed = request.clone();
        changed.expected_head = Some("def456".into());
        assert!(matches!(
            engine.consume_exec(&ticket.id, &changed),
            Err(PermissionError::Mismatch)
        ));
    }

    #[test]
    fn approval_is_bound_to_stdin() {
        let mut engine = PermissionEngine::new().unwrap();
        let mut request = request(&["cat"]);
        request.stdin = Some("first".into());
        let ticket = request_ticket(&mut engine, &request);
        engine.approve(&ticket.id).unwrap();

        let mut changed = request.clone();
        changed.stdin = Some("second".into());
        assert!(matches!(
            engine.consume_exec(&ticket.id, &changed),
            Err(PermissionError::Mismatch)
        ));
    }

    #[test]
    fn approval_is_bound_to_protected_read_state() {
        let mut engine = PermissionEngine::new().unwrap();
        let mut request = request(&["cat", ".env"]);
        request.protected_read = true;
        let ticket = request_ticket(&mut engine, &request);
        engine.approve(&ticket.id).unwrap();
        let mut changed = request.clone();
        changed.protected_read = false;
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
    fn local_cli_approval_uses_the_same_bound_one_shot_ticket() {
        let mut engine = PermissionEngine::new().unwrap();
        let approved_request = request(&["cargo", "test"]);
        let ticket = request_ticket(&mut engine, &approved_request);

        ApprovalStore::approve(&ticket.id).unwrap().unwrap();
        engine.consume_exec(&ticket.id, &approved_request).unwrap();
        assert!(matches!(
            engine.consume_exec(&ticket.id, &approved_request),
            Err(PermissionError::NotFound)
        ));

        let changed_request = request(&["cargo", "check"]);
        let ticket = request_ticket(&mut engine, &approved_request);
        ApprovalStore::approve(&ticket.id).unwrap().unwrap();
        assert!(matches!(
            engine.consume_exec(&ticket.id, &changed_request),
            Err(PermissionError::Mismatch)
        ));
    }

    #[test]
    fn pending_approval_tickets_have_a_hard_capacity() {
        let mut engine = PermissionEngine::new().unwrap();
        let request = request(&["cargo", "test"]);
        for _ in 0..MAX_PENDING_TICKETS {
            engine
                .request_action(&request, "test".into(), "test".into())
                .unwrap();
        }
        assert!(matches!(
            engine.request_action(&request, "overflow".into(), "test".into()),
            Err(PermissionError::Limit)
        ));
    }

    #[test]
    fn multi_capability_consumption_is_atomic() {
        let mut engine = PermissionEngine::new().unwrap();
        let mut git_request = request(&["git", "push"]);
        git_request.capability = Capability::GitRemoteWrite;
        git_request.network = NetworkPolicy::Outbound;
        let mut network_request = git_request.clone();
        network_request.capability = Capability::NetworkOutbound;
        let git_ticket = request_ticket(&mut engine, &git_request);
        let network_ticket = request_ticket(&mut engine, &network_request);
        engine.approve(&git_ticket.id).unwrap();

        assert!(matches!(
            engine.consume_execs(&[
                (git_ticket.id.clone(), git_request.clone()),
                (network_ticket.id.clone(), network_request.clone())
            ]),
            Err(PermissionError::NotApproved)
        ));
        engine.consume_exec(&git_ticket.id, &git_request).unwrap();
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
        let ticket = engine
            .request_action(&approved, "test".into(), "test".into())
            .unwrap();
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

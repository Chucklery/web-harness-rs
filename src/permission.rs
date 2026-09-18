use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;

static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);
const TICKET_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct ExecAuthorization {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub background: bool,
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

    pub fn request_exec(&mut self, request: &ExecAuthorization) -> ApprovalRequest {
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
            summary: format!(
                "Run {}",
                request.argv.first().map(String::as_str).unwrap_or("?")
            ),
            reason: "OS sandbox enforcement is not enabled; explicit approval is required".into(),
            expires_in_seconds: TICKET_TTL.as_secs(),
        }
    }

    pub fn authorize_workspace_patch(&self) -> Result<(), PermissionError> {
        Ok(())
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
        let ticket = self.tickets.remove(id).ok_or(PermissionError::NotFound)?;
        if ticket.expires <= Instant::now() {
            return Err(PermissionError::Expired);
        }
        if !ticket.approved {
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
            argv: argv.iter().map(|value| value.to_string()).collect(),
            cwd: Some(".".into()),
            background: false,
        }
    }

    #[test]
    fn approval_is_bound_to_exact_request_and_single_use() {
        let mut engine = PermissionEngine::new().unwrap();
        let first = request(&["cargo", "test"]);
        let ticket = engine.request_exec(&first);
        engine.approve(&ticket.id).unwrap();
        assert!(matches!(
            engine.consume_exec(&ticket.id, &request(&["cargo", "check"])),
            Err(PermissionError::Mismatch)
        ));
    }

    #[test]
    fn approved_ticket_can_be_consumed_once() {
        let mut engine = PermissionEngine::new().unwrap();
        let request = request(&["cargo", "test"]);
        let ticket = engine.request_exec(&request);
        engine.approve(&ticket.id).unwrap();
        engine.consume_exec(&ticket.id, &request).unwrap();
        assert!(matches!(
            engine.consume_exec(&ticket.id, &request),
            Err(PermissionError::NotFound)
        ));
    }
}

use crate::atomic_file;
use crate::config;
use crate::process;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LIVE_SESSIONS: usize = 32;
pub const MAX_PENDING_TICKETS: usize = 128;
pub const MAX_APPROVAL_LIST: usize = 128;
const MAX_RECORD_BYTES: u64 = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub ticket_id: String,
    pub capability: String,
    pub summary: String,
    pub expires_unix_s: u64,
}

pub struct ApprovalStore {
    session_dir: PathBuf,
}

impl ApprovalStore {
    pub fn new(workspace: Option<&Path>) -> std::io::Result<Self> {
        let root = config::state_dir()
            .map_err(|error| std::io::Error::other(error.to_string()))?
            .join("approvals");
        if crate::sandbox::SandboxBackend::detect().enforced() {
            let canonical_root = canonicalize_or_nearest_existing(&root)?;
            if is_writable_by_sandbox(&canonical_root, workspace)? {
                return Err(std::io::Error::other(
                    "approval state directory is inside a sandbox-writable path",
                ));
            }
        }
        fs::create_dir_all(&root)?;
        ensure_private_directory(&root)?;
        reap_dead_sessions(&root)?;

        let live_sessions = fs::read_dir(&root)?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .take(MAX_LIVE_SESSIONS + 1)
            .count();
        if live_sessions >= MAX_LIVE_SESSIONS {
            return Err(std::io::Error::other("too many active approval sessions"));
        }

        for _ in 0..16 {
            let nonce = random_u64()?;
            let session_dir = root.join(format!("session-{}-{nonce:016x}", std::process::id()));
            match fs::create_dir(&session_dir) {
                Ok(()) => {
                    ensure_private_directory(&session_dir)?;
                    return Ok(Self { session_dir });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::other(
            "could not allocate a unique approval session",
        ))
    }

    pub fn publish(&self, record: &ApprovalRecord) -> std::io::Result<()> {
        if !valid_ticket_id(&record.ticket_id) {
            return Err(std::io::Error::other("invalid approval ticket id"));
        }
        if record.summary.len() > 512 || record.capability.len() > 64 {
            return Err(std::io::Error::other("approval summary exceeds its limit"));
        }
        let bytes = serde_json::to_vec(record)?;
        if bytes.len() > MAX_RECORD_BYTES as usize {
            return Err(std::io::Error::other("approval record exceeds its limit"));
        }
        let path = self.session_dir.join(format!("{}.json", record.ticket_id));
        atomic_file::stage(&path, &bytes, true)?.commit()
    }

    pub fn is_approved(&self, ticket_id: &str) -> std::io::Result<bool> {
        if !valid_ticket_id(ticket_id) {
            return Ok(false);
        }
        let marker = self.session_dir.join(format!("{ticket_id}.approved"));
        match fs::symlink_metadata(marker) {
            Ok(metadata)
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() == b"approved\n".len() as u64 =>
            {
                Ok(
                    fs::read(self.session_dir.join(format!("{ticket_id}.approved")))?
                        == b"approved\n",
                )
            }
            Ok(_) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn consume(&self, ticket_id: &str) {
        self.remove_files(ticket_id);
    }

    pub fn approve(ticket_id: &str) -> std::io::Result<Option<ApprovalRecord>> {
        let root = approvals_root()?;
        for session in session_directories(&root)? {
            let record_path = session.join(format!("{ticket_id}.json"));
            if let Some(record) = read_record(&record_path)? {
                if record.expires_unix_s <= now_unix_s() {
                    return Ok(None);
                }
                let marker = session.join(format!("{ticket_id}.approved"));
                atomic_file::stage(&marker, b"approved\n", true)?.commit()?;
                return Ok(Some(record));
            }
        }
        Ok(None)
    }

    pub fn pending_any(ticket_id: &str) -> std::io::Result<Option<ApprovalRecord>> {
        if !valid_ticket_id(ticket_id) {
            return Ok(None);
        }
        let root = approvals_root()?;
        for session in session_directories(&root)? {
            let record = session.join(format!("{ticket_id}.json"));
            if let Some(record) = read_record(&record)? {
                if record.expires_unix_s > now_unix_s() {
                    return Ok(Some(record));
                }
            }
        }
        Ok(None)
    }

    pub fn list_pending() -> std::io::Result<Vec<ApprovalRecord>> {
        let root = approvals_root()?;
        let mut records = Vec::new();
        'sessions: for session in session_directories(&root)? {
            let entries = match fs::read_dir(&session) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.filter_map(Result::ok).take(MAX_PENDING_TICKETS * 2) {
                if records.len() >= MAX_APPROVAL_LIST {
                    break 'sessions;
                }
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("json") {
                    continue;
                }
                let record = match read_record(&path) {
                    Ok(record) => record,
                    Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => continue,
                    Err(error) => return Err(error),
                };
                if let Some(record) = record {
                    if record.expires_unix_s > now_unix_s()
                        && !session
                            .join(format!("{}.approved", record.ticket_id))
                            .exists()
                    {
                        records.push(record);
                    }
                }
            }
        }
        records.sort_by(|left, right| left.ticket_id.cmp(&right.ticket_id));
        Ok(records)
    }

    fn remove_files(&self, ticket_id: &str) {
        if !valid_ticket_id(ticket_id) {
            return;
        }
        let _ = fs::remove_file(self.session_dir.join(format!("{ticket_id}.json")));
        let _ = fs::remove_file(self.session_dir.join(format!("{ticket_id}.approved")));
    }
}

impl Drop for ApprovalStore {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.session_dir)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            let _ = fs::remove_dir_all(&self.session_dir);
        }
    }
}

fn approvals_root() -> std::io::Result<PathBuf> {
    let root = config::state_dir()
        .map_err(|error| std::io::Error::other(error.to_string()))?
        .join("approvals");
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(root),
        Ok(_) => Err(std::io::Error::other(
            "approval state path is not a real directory",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(root),
        Err(error) => Err(error),
    }
}

fn canonicalize_or_nearest_existing(path: &Path) -> std::io::Result<PathBuf> {
    match fs::canonicalize(path) {
        Ok(canonical) => Ok(canonical),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| std::io::Error::other("approval state path has no parent"))?;
            Ok(canonicalize_or_nearest_existing(parent)?.join(
                path.file_name()
                    .ok_or_else(|| std::io::Error::other("approval state path has no name"))?,
            ))
        }
        Err(error) => Err(error),
    }
}

fn session_directories(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(Vec::new());
    };
    let mut sessions = Vec::new();
    for entry in entries.filter_map(Result::ok).take(MAX_LIVE_SESSIONS * 2) {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && !kind.is_symlink() {
            sessions.push(entry.path());
        }
    }
    Ok(sessions)
}

fn read_record(path: &Path) -> std::io::Result<Option<ApprovalRecord>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_RECORD_BYTES
    {
        return Ok(None);
    }
    let Some(expected_name) = path.file_stem().and_then(|value| value.to_str()) else {
        return Ok(None);
    };
    if !valid_ticket_id(expected_name) {
        return Ok(None);
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Ok(None);
    }
    let record: ApprovalRecord = serde_json::from_slice(&bytes)?;
    if record.ticket_id != expected_name {
        return Ok(None);
    }
    Ok(Some(record))
}

fn reap_dead_sessions(root: &Path) -> std::io::Result<()> {
    for session in session_directories(root)? {
        let Some(name) = session.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(pid) = name
            .strip_prefix("session-")
            .and_then(|name| name.split_once('-'))
            .and_then(|(pid, _nonce)| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if !process::process_alive(pid) {
            fs::remove_dir_all(session)?;
        }
    }
    Ok(())
}

fn ensure_private_directory(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "approval state path must be a real directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn is_writable_by_sandbox(approval_root: &Path, workspace: Option<&Path>) -> std::io::Result<bool> {
    if workspace.is_some_and(|workspace| path_is_within(approval_root, workspace)) {
        return Ok(true);
    }
    let temporary_roots = [
        Some(std::env::temp_dir()),
        std::env::var_os("TMPDIR").map(PathBuf::from),
        Some(PathBuf::from("/tmp")),
        Some(PathBuf::from("/private/tmp")),
    ];
    for root in temporary_roots.into_iter().flatten() {
        let canonical = match fs::canonicalize(root) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if path_is_within(approval_root, &canonical) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn valid_ticket_id(ticket_id: &str) -> bool {
    let Some((pid, rest)) = ticket_id
        .strip_prefix("apr_")
        .and_then(|id| id.split_once('_'))
    else {
        return false;
    };
    let Some((serial, nonce)) = rest.split_once('_') else {
        return false;
    };
    !pid.is_empty()
        && !serial.is_empty()
        && nonce.len() == 32
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && serial.bytes().all(|byte| byte.is_ascii_digit())
        && nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn random_u64() -> std::io::Result<u64> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(u64::from_ne_bytes(bytes))
}

fn now_unix_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_ids_cannot_escape_the_approval_directory() {
        assert!(valid_ticket_id("apr_42_7_0123456789abcdef0123456789abcdef"));
        for invalid in [
            "../approval",
            "apr_42_7/../../x",
            "apr__7_0123456789abcdef0123456789abcdef",
            "apr_1_a_0123456789abcdef0123456789abcdef",
            "apr_42_7_0123",
        ] {
            assert!(!valid_ticket_id(invalid));
        }
    }

    #[test]
    fn sandbox_writable_ancestors_are_rejected() {
        assert!(path_is_within(Path::new("/tmp/state"), Path::new("/tmp")));
        assert!(path_is_within(
            Path::new("/workspace/.state"),
            Path::new("/workspace")
        ));
        assert!(!path_is_within(
            Path::new("/home/user/state"),
            Path::new("/tmp")
        ));
    }

    #[test]
    fn canonicalizes_missing_paths_without_losing_symlink_ancestors() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        fs::create_dir(&real).unwrap();
        let alias = temp.path().join("alias");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real, &alias).unwrap();
        let expected = fs::canonicalize(&real).unwrap().join("not-created");
        assert_eq!(
            canonicalize_or_nearest_existing(&alias.join("not-created")).unwrap(),
            expected
        );
    }

    #[test]
    fn approval_manifest_is_private_and_one_shot() {
        let store = ApprovalStore::new(None).unwrap();
        let record = ApprovalRecord {
            ticket_id: "apr_42_7_0123456789abcdef0123456789abcdef".into(),
            capability: "process.execute".into(),
            summary: "Run a bounded test".into(),
            expires_unix_s: now_unix_s() + 60,
        };
        store.publish(&record).unwrap();
        assert_eq!(
            ApprovalStore::pending_any(&record.ticket_id)
                .unwrap()
                .unwrap()
                .summary,
            record.summary
        );
        let approved = ApprovalStore::approve(&record.ticket_id).unwrap().unwrap();
        assert_eq!(approved.ticket_id, record.ticket_id);
        assert!(store.is_approved(&record.ticket_id).unwrap());
        store.consume(&record.ticket_id);
        assert!(!store.is_approved(&record.ticket_id).unwrap());
        assert!(ApprovalStore::pending_any(&record.ticket_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn publish_rejects_ticket_ids_outside_the_store_namespace() {
        let store = ApprovalStore::new(None).unwrap();
        let record = ApprovalRecord {
            ticket_id: "../outside".into(),
            capability: "process.execute".into(),
            summary: "Must not escape".into(),
            expires_unix_s: now_unix_s() + 60,
        };
        assert!(store.publish(&record).is_err());
        assert!(!store
            .session_dir
            .parent()
            .unwrap()
            .join("outside.json")
            .exists());
    }

    #[test]
    fn approved_requests_are_not_listed_as_pending() {
        let store = ApprovalStore::new(None).unwrap();
        let record = ApprovalRecord {
            ticket_id: "apr_42_8_0123456789abcdef0123456789abcdef".into(),
            capability: "process.execute".into(),
            summary: "Run a bounded test".into(),
            expires_unix_s: now_unix_s() + 60,
        };
        store.publish(&record).unwrap();
        assert!(ApprovalStore::list_pending()
            .unwrap()
            .iter()
            .any(|pending| pending.ticket_id == record.ticket_id));
        ApprovalStore::approve(&record.ticket_id).unwrap().unwrap();
        assert!(!ApprovalStore::list_pending()
            .unwrap()
            .iter()
            .any(|pending| pending.ticket_id == record.ticket_id));
    }

    #[test]
    fn expired_request_cannot_be_approved() {
        let store = ApprovalStore::new(None).unwrap();
        let record = ApprovalRecord {
            ticket_id: "apr_42_9_0123456789abcdef0123456789abcdef".into(),
            capability: "process.execute".into(),
            summary: "Expired test".into(),
            expires_unix_s: now_unix_s().saturating_sub(1),
        };
        store.publish(&record).unwrap();
        assert!(ApprovalStore::approve(&record.ticket_id).unwrap().is_none());
        assert!(!store.is_approved(&record.ticket_id).unwrap());
    }
}

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_EVIDENCE_BYTES: usize = 16 * 1024;
const MAX_TOOL_CALLS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceEvidence {
    pub schema_version: u8,
    pub client_version: String,
    pub protocol_version: String,
    pub stages: Vec<StageEvidence>,
    pub tool_calls: Vec<ToolCallEvidence>,
    pub failure_recovery: FailureRecovery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageEvidence {
    pub stage: AcceptanceStage,
    pub passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceStage {
    ConnectWorkspace,
    DiscoverProject,
    ReadInstructionsAndSource,
    SearchSymbol,
    ModifyWorkspace,
    RunValidation,
    ObserveBackgroundJob,
    InspectGitChanges,
    VerifyUserApproval,
    DisconnectCleanup,
}

impl AcceptanceStage {
    const REQUIRED: [Self; 10] = [
        Self::ConnectWorkspace,
        Self::DiscoverProject,
        Self::ReadInstructionsAndSource,
        Self::SearchSymbol,
        Self::ModifyWorkspace,
        Self::RunValidation,
        Self::ObserveBackgroundJob,
        Self::InspectGitChanges,
        Self::VerifyUserApproval,
        Self::DisconnectCleanup,
    ];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallEvidence {
    pub sequence: u16,
    pub tool: ToolName,
    pub outcome: ToolOutcome,
    pub error_code: Option<ToolErrorCode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    RuntimeStatus,
    WorkOnProject,
    ToolManifest,
    CallRuntimeTool,
    Permission,
    WorkspaceInfo,
    ListFiles,
    WorkspaceInstructions,
    ReadFiles,
    Search,
    Patch,
    Exec,
    JobPoll,
    JobWait,
    JobOutput,
    JobList,
    JobCancel,
    GitHead,
    GitStatus,
    GitDiff,
    GitLog,
    GitShow,
    GitShowFile,
    GitAdd,
    GitCommit,
    GitSwitch,
    GitCreateBranch,
    GitRestore,
    GitPush,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolOutcome {
    Succeeded,
    ApprovalRequired,
    Failed,
    Recovered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCode {
    InvalidArguments,
    NotFound,
    Denied,
    PermissionDenied,
    DependencyUnavailable,
    ExecutionFailed,
    Conflict,
    LimitExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureRecovery {
    NotNeeded,
    RetryAfterTransientFailure,
    RetryAfterHostReconnect,
    RetryAfterApprovalDecline,
    OtherRecovered,
}

impl AcceptanceEvidence {
    pub fn parse(stdout: &[u8], truncated: bool) -> Result<Self, &'static str> {
        if truncated || stdout.len() > MAX_EVIDENCE_BYTES {
            return Err("acceptance evidence exceeds its 16 KiB bound");
        }
        let evidence: Self = serde_json::from_slice(stdout)
            .map_err(|_| "stdout must contain one JSON acceptance evidence object")?;
        evidence.validate()?;
        Ok(evidence)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1 {
            return Err("unsupported acceptance evidence schema_version");
        }
        if !valid_version(&self.client_version) || !valid_version(&self.protocol_version) {
            return Err("client_version and protocol_version must be bounded version tokens");
        }
        if self.stages.len() != AcceptanceStage::REQUIRED.len() {
            return Err("acceptance evidence must include each required stage exactly once");
        }
        let mut stages = HashSet::with_capacity(AcceptanceStage::REQUIRED.len());
        if self
            .stages
            .iter()
            .any(|stage| !stage.passed || !stages.insert(stage.stage))
            || !self
                .stages
                .iter()
                .map(|stage| stage.stage)
                .eq(AcceptanceStage::REQUIRED)
        {
            return Err("all required acceptance stages must pass exactly once");
        }
        if self.tool_calls.is_empty() || self.tool_calls.len() > MAX_TOOL_CALLS {
            return Err("tool_calls must contain 1..=64 bounded call records");
        }
        if self
            .tool_calls
            .iter()
            .enumerate()
            .any(|(index, call)| usize::from(call.sequence) != index + 1)
        {
            return Err("tool call sequence numbers must be contiguous and start at one");
        }
        if self
            .tool_calls
            .iter()
            .any(|call| matches!(call.outcome, ToolOutcome::Failed) != call.error_code.is_some())
        {
            return Err("failed tool calls must include a machine-readable error_code only");
        }
        for required in [
            ToolName::WorkspaceInfo,
            ToolName::ListFiles,
            ToolName::WorkspaceInstructions,
            ToolName::ReadFiles,
            ToolName::Search,
            ToolName::Patch,
            ToolName::Exec,
            ToolName::JobWait,
            ToolName::GitStatus,
            ToolName::GitDiff,
        ] {
            if !self.tool_calls.iter().any(|call| {
                call.tool == required
                    && matches!(
                        call.outcome,
                        ToolOutcome::Succeeded | ToolOutcome::Recovered
                    )
            }) {
                return Err("tool_calls do not cover every required workspace operation");
            }
        }
        let has_approval_challenge = self
            .tool_calls
            .iter()
            .any(|call| call.outcome == ToolOutcome::ApprovalRequired);
        let has_recovered_approval = self.tool_calls.iter().any(|call| {
            call.outcome == ToolOutcome::Succeeded
                && self.tool_calls.iter().any(|challenge| {
                    challenge.tool == call.tool
                        && challenge.outcome == ToolOutcome::ApprovalRequired
                        && challenge.sequence < call.sequence
                })
        });
        if !has_approval_challenge || !has_recovered_approval {
            return Err(
                "evidence must show a Host approval challenge and a later successful retry",
            );
        }
        if self.failure_recovery == FailureRecovery::NotNeeded
            && self
                .tool_calls
                .iter()
                .any(|call| call.outcome == ToolOutcome::Recovered)
        {
            return Err("failure_recovery must describe recorded recovered tool failures");
        }
        if self.failure_recovery != FailureRecovery::NotNeeded
            && !self
                .tool_calls
                .iter()
                .any(|call| call.outcome == ToolOutcome::Recovered)
        {
            return Err("recovered tool failures require a failure_recovery path");
        }
        for failed in self
            .tool_calls
            .iter()
            .filter(|call| call.outcome == ToolOutcome::Failed)
        {
            let recovered = self.tool_calls.iter().any(|call| {
                call.tool == failed.tool
                    && call.sequence > failed.sequence
                    && call.outcome == ToolOutcome::Recovered
            });
            if self.failure_recovery == FailureRecovery::NotNeeded || !recovered {
                return Err(
                    "each failed tool call requires a later recovered result and recovery path",
                );
            }
        }
        Ok(())
    }
}

fn valid_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-/".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_evidence() -> Vec<u8> {
        let stages = AcceptanceStage::REQUIRED
            .iter()
            .map(|stage| json!({"stage": stage, "passed": true}))
            .collect::<Vec<_>>();
        let tools = [
            ("workspace_info", "succeeded"),
            ("list_files", "succeeded"),
            ("workspace_instructions", "succeeded"),
            ("read_files", "succeeded"),
            ("search", "succeeded"),
            ("patch", "succeeded"),
            ("exec", "approval_required"),
            ("exec", "succeeded"),
            ("job_wait", "succeeded"),
            ("git_status", "succeeded"),
            ("git_diff", "succeeded"),
        ];
        let tool_calls = tools
            .iter()
            .enumerate()
            .map(|(sequence, (tool, outcome))| {
                json!({
                    "sequence": sequence as u16 + 1,
                    "tool": tool,
                    "outcome": outcome,
                    "error_code": null
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "client_version": "1.2026.09.23",
            "protocol_version": "2025-06-18",
            "stages": stages,
            "tool_calls": tool_calls,
            "failure_recovery": "not_needed"
        }))
        .unwrap()
    }

    #[test]
    fn accepts_complete_bounded_evidence() {
        let bytes = valid_evidence();
        let evidence = AcceptanceEvidence::parse(&bytes, false).unwrap();
        assert_eq!(evidence.client_version, "1.2026.09.23");
        assert_eq!(evidence.stages.len(), AcceptanceStage::REQUIRED.len());
    }

    #[test]
    fn rejects_exit_output_without_structured_evidence() {
        assert!(AcceptanceEvidence::parse(b"", false).is_err());
        assert!(AcceptanceEvidence::parse(b"debug log\n{}", false).is_err());
    }

    #[test]
    fn rejects_incomplete_or_unbounded_evidence() {
        assert!(AcceptanceEvidence::parse(&valid_evidence(), true).is_err());
        assert!(AcceptanceEvidence::parse(&vec![b'x'; MAX_EVIDENCE_BYTES + 1], false).is_err());

        let mut value: serde_json::Value = serde_json::from_slice(&valid_evidence()).unwrap();
        value["stages"].as_array_mut().unwrap().pop();
        assert!(AcceptanceEvidence::parse(&serde_json::to_vec(&value).unwrap(), false).is_err());
    }

    #[test]
    fn rejects_free_text_fields_and_missing_approval_retry() {
        let mut value: serde_json::Value = serde_json::from_slice(&valid_evidence()).unwrap();
        value["private_workspace_dump"] = json!("not allowed");
        assert!(AcceptanceEvidence::parse(&serde_json::to_vec(&value).unwrap(), false).is_err());

        let mut value: serde_json::Value = serde_json::from_slice(&valid_evidence()).unwrap();
        value["tool_calls"][7]["outcome"] = json!("failed");
        value["tool_calls"][7]["error_code"] = json!("execution_failed");
        assert!(AcceptanceEvidence::parse(&serde_json::to_vec(&value).unwrap(), false).is_err());
    }
}

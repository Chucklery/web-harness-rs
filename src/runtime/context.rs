use crate::jobs::JobManager;
use crate::permission::PermissionEngine;
use crate::sandbox::SandboxBackend;
use crate::workspace::Workspace;

#[derive(Debug, Clone, Copy)]
pub struct RuntimeLimits {
    pub max_read_paths: usize,
    pub max_read_file_bytes: usize,
    pub max_read_batch_bytes: usize,
    pub max_search_queries: usize,
    pub max_search_results: usize,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_read_paths: 16,
            max_read_file_bytes: 256 * 1024,
            max_read_batch_bytes: 512 * 1024,
            max_search_queries: 8,
            max_search_results: 200,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeEnvironment {
    pub os: &'static str,
    pub arch: &'static str,
}

impl Default for RuntimeEnvironment {
    fn default() -> Self {
        Self {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        }
    }
}

pub struct ExecutionContext<'a> {
    workspace: &'a Workspace,
    jobs: &'a mut JobManager,
    permissions: &'a mut PermissionEngine,
    sandbox: SandboxBackend,
    limits: RuntimeLimits,
    environment: RuntimeEnvironment,
}

impl<'a> ExecutionContext<'a> {
    pub fn new(
        workspace: &'a Workspace,
        jobs: &'a mut JobManager,
        permissions: &'a mut PermissionEngine,
    ) -> Self {
        Self {
            workspace,
            jobs,
            permissions,
            sandbox: SandboxBackend::detect(),
            limits: RuntimeLimits::default(),
            environment: RuntimeEnvironment::default(),
        }
    }

    pub fn workspace(&self) -> &Workspace {
        self.workspace
    }

    pub fn jobs(&mut self) -> &mut JobManager {
        self.jobs
    }

    pub fn permissions(&mut self) -> &mut PermissionEngine {
        self.permissions
    }

    pub fn sandbox(&self) -> SandboxBackend {
        self.sandbox
    }

    pub fn limits(&self) -> RuntimeLimits {
        self.limits
    }

    pub fn environment(&self) -> RuntimeEnvironment {
        self.environment
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_exposes_bounded_defaults_without_environment_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);

        assert_eq!(context.limits().max_read_paths, 16);
        assert!(!context.environment().os.is_empty());
        assert!(!context.environment().arch.is_empty());
    }
}

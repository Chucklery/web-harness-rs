use super::context::ExecutionContext;
use super::exec::ExecRuntime;
use super::file::FileRuntime;
use super::git::GitRuntime;
use super::job::JobRuntime;
use super::patch::PatchRuntime;
use super::permission::PermissionRuntime;
use super::search::SearchRuntime;
use super::tool_trait::{RuntimeTool, RuntimeToolError};
use super::workspace::{WorkspaceInfoRuntime, WorkspaceInstructionsRuntime};
use serde_json::Value;
use std::collections::HashMap;

pub struct RuntimeRegistry {
    tools: HashMap<&'static str, Box<dyn RuntimeTool>>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register<T>(&mut self, tool: T)
    where
        T: RuntimeTool + 'static,
    {
        self.tools.insert(tool.name(), Box::new(tool));
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn names(&self) -> Vec<&'static str> {
        let mut names = self.tools.keys().copied().collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    pub fn call(
        &self,
        name: &str,
        context: &mut ExecutionContext<'_>,
        arguments: &Value,
    ) -> Option<Result<Value, RuntimeToolError>> {
        self.tools
            .get(name)
            .map(|tool| tool.call(context, arguments))
    }
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register(FileRuntime);
        registry.register(SearchRuntime);
        registry.register(ExecRuntime);
        registry.register(JobRuntime);
        registry.register(GitRuntime);
        registry.register(WorkspaceInfoRuntime);
        registry.register(WorkspaceInstructionsRuntime);
        registry.register(PatchRuntime);
        registry.register(PermissionRuntime);
        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_contains_file_runtime() {
        let registry = RuntimeRegistry::default();
        assert!(registry.tools.contains_key("read_files"));
        assert!(registry.tools.contains_key("search"));
        assert!(registry.tools.contains_key("exec"));
        assert!(registry.tools.contains_key("job"));
        assert!(registry.tools.contains_key("git"));
        assert!(registry.tools.contains_key("workspace_info"));
        assert!(registry.tools.contains_key("workspace_instructions"));
        assert!(registry.tools.contains_key("patch"));
        assert!(registry.tools.contains_key("permission"));
        assert_eq!(registry.len(), 9);
    }
}

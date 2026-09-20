use super::file::FileRuntime;
use super::search::SearchRuntime;
use super::tool_trait::{RuntimeTool, RuntimeToolError};
use crate::workspace::Workspace;
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

    pub fn call(
        &self,
        name: &str,
        workspace: &Workspace,
        arguments: &Value,
    ) -> Option<Result<Value, RuntimeToolError>> {
        self.tools
            .get(name)
            .map(|tool| tool.call(workspace, arguments))
    }
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        let mut registry = Self::new();
        registry.register(FileRuntime);
        registry.register(SearchRuntime);
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
    }
}

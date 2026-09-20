use super::context::ExecutionContext;
use super::registry::RuntimeRegistry;
use serde_json::{json, Value};

pub fn describe(registry: &RuntimeRegistry, context: &ExecutionContext<'_>) -> Value {
    let environment = context.environment();
    let sandbox = context.sandbox();
    json!({
        "service":"web-harness",
        "version":env!("CARGO_PKG_VERSION"),
        "runtime_exposure":"adaptive_shim",
        "client_id":"local",
        "tools":{"direct":registry.len(),"control":4},
        "projects":{"count":1,"mode":"configured_workspace"},
        "connection_layers":{"stdio_runtime":{"status":"ready"},"workspace":{"status":"ready"}},
        "environment":{"os":environment.os,"arch":environment.arch,"sandbox_enforced":sandbox.enforced()},
        "authority":{"workspace_boundary":true,"project_write":true,"shell":true,"git":true,"network":false}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::JobManager;
    use crate::permission::PermissionEngine;
    use crate::workspace::Workspace;

    #[test]
    fn status_uses_registry_and_context() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let mut jobs = JobManager::new();
        let mut permissions = PermissionEngine::new().unwrap();
        let context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let registry = RuntimeRegistry::default();
        let value = describe(&registry, &context);
        assert_eq!(value["tools"]["direct"], registry.len());
        assert!(!value["environment"]["os"].as_str().unwrap().is_empty());
    }
}

use super::registry::RuntimeRegistry;
use super::tool_trait::{RuntimeErrorKind, RuntimeToolError};
use serde_json::{json, Value};

pub fn describe(
    registry: &RuntimeRegistry,
    requested: Option<&str>,
) -> Result<Value, RuntimeToolError> {
    if let Some(name) = requested {
        if !registry.contains(name) {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "unknown runtime tool",
            ));
        }
    }
    let tools = registry
        .names()
        .into_iter()
        .filter(|name| requested.map(|wanted| wanted == *name).unwrap_or(true))
        .map(|name| {
            json!({
                "name": name,
                "route": "call_runtime_tool",
                "direct_tool_available": true,
                "input_schema_source": "tools/list"
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "runtime":"web-harness",
        "tools":tools,
        "recommended_flow":"Prefer direct tools; use call_runtime_tool only when the client expects an adaptive-runtime gateway."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_is_registry_backed() {
        let registry = RuntimeRegistry::default();
        assert_eq!(
            describe(&registry, Some("git")).unwrap()["tools"][0]["name"],
            "git"
        );
        assert!(describe(&registry, Some("unknown")).is_err());
    }
}

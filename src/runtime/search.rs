use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::search;
use crate::workspace::Workspace;
use serde_json::{json, Value};

const MAX_QUERY_BYTES: usize = 1024;
const MAX_QUERIES: usize = 8;
const DEFAULT_MAX_RESULTS: usize = 100;
const MAX_RESULTS: usize = 200;

pub struct SearchRuntime;

impl RuntimeTool for SearchRuntime {
    fn name(&self) -> &'static str {
        "search"
    }

    fn call(&self, workspace: &Workspace, arguments: &Value) -> Result<Value, RuntimeToolError> {
        let query = arguments.get("query").and_then(Value::as_str);
        let queries = arguments.get("queries").and_then(Value::as_array);
        if query.is_some() == queries.is_some() {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "exactly one of arguments.query or arguments.queries is required",
            ));
        }

        let max_results = arguments
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_MAX_RESULTS as u64) as usize;
        if !(1..=MAX_RESULTS).contains(&max_results) {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "max_results must be 1..=200",
            ));
        }

        if let Some(query) = query {
            validate_query(query)?;
            let matches =
                search::content_search(workspace, query, max_results).map_err(|error| {
                    RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())
                })?;
            return Ok(json!({"matches": matches}));
        }

        let queries = queries.expect("query shape validated above");
        if queries.is_empty() || queries.len() > MAX_QUERIES {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "queries must contain 1..=8 items",
            ));
        }

        let mut parsed = Vec::with_capacity(queries.len());
        for value in queries {
            let query = value.as_str().ok_or_else(|| {
                RuntimeToolError::new(
                    RuntimeErrorKind::InvalidArguments,
                    "queries items must be strings",
                )
            })?;
            validate_query(query)?;
            parsed.push(query);
        }

        let mut remaining_results = max_results;
        let mut results = Vec::with_capacity(parsed.len());
        for (index, query) in parsed.iter().enumerate() {
            let remaining_queries = parsed.len() - index;
            let per_query_limit = remaining_results.div_ceil(remaining_queries);
            let matches = if per_query_limit == 0 {
                Vec::new()
            } else {
                search::content_search(workspace, query, per_query_limit).map_err(|error| {
                    RuntimeToolError::new(RuntimeErrorKind::Execution, error.to_string())
                })?
            };
            remaining_results = remaining_results.saturating_sub(matches.len());
            results.push(json!({"query": query, "matches": matches}));
        }

        Ok(json!({"results": results}))
    }
}

fn validate_query(query: &str) -> Result<(), RuntimeToolError> {
    if query.is_empty() || query.len() > MAX_QUERY_BYTES {
        return Err(RuntimeToolError::new(
            RuntimeErrorKind::InvalidArguments,
            "query must be 1..=1024 bytes",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_ambiguous_query_shape() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let error = SearchRuntime
            .call(&workspace, &json!({"query": "alpha", "queries": ["beta"]}))
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }

    #[test]
    fn rejects_oversized_batch() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let queries = vec!["x"; MAX_QUERIES + 1];
        let error = SearchRuntime
            .call(&workspace, &json!({"queries": queries}))
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }
}

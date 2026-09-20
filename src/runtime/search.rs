use super::context::ExecutionContext;
use super::tool_trait::{RuntimeErrorKind, RuntimeTool, RuntimeToolError};
use crate::permission::Capability;
use crate::search::{self, SearchError};
use serde_json::{json, Value};

const MAX_QUERY_BYTES: usize = 1024;

pub struct SearchRuntime;

impl RuntimeTool for SearchRuntime {
    fn name(&self) -> &'static str {
        "search"
    }

    fn call(
        &self,
        context: &mut ExecutionContext<'_>,
        arguments: &Value,
    ) -> Result<Value, RuntimeToolError> {
        context
            .permissions()
            .authorize(Capability::WorkspaceRead)
            .map_err(|error| {
                RuntimeToolError::new(RuntimeErrorKind::Permission, error.to_string())
            })?;
        let limits = context.limits();
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
            .unwrap_or(100) as usize;
        if !(1..=limits.max_search_results).contains(&max_results) {
            return Err(RuntimeToolError::new(
                RuntimeErrorKind::InvalidArguments,
                "max_results must be 1..=200",
            ));
        }

        if let Some(query) = query {
            validate_query(query)?;
            let matches = search::content_search(context.workspace(), query, max_results)
                .map_err(search_error)?;
            return Ok(json!({"matches": matches}));
        }

        let queries = queries.expect("query shape validated above");
        if queries.is_empty() || queries.len() > limits.max_search_queries {
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
                search::content_search(context.workspace(), query, per_query_limit)
                    .map_err(search_error)?
            };
            remaining_results = remaining_results.saturating_sub(matches.len());
            results.push(json!({"query": query, "matches": matches}));
        }

        Ok(json!({"results": results}))
    }
}

/// A missing ripgrep is a dependency problem, not a failed search: the request
/// was well formed and reissuing it would fail identically.
fn search_error(error: SearchError) -> RuntimeToolError {
    let kind = match error {
        SearchError::RipgrepUnavailable => RuntimeErrorKind::Dependency,
        SearchError::Failed(_) => RuntimeErrorKind::Execution,
    };
    RuntimeToolError::new(kind, error.to_string())
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
        let workspace = crate::workspace::Workspace::new(dir.path()).unwrap();
        let mut jobs = crate::jobs::JobManager::new();
        let mut permissions = crate::permission::PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let error = SearchRuntime
            .call(
                &mut context,
                &json!({"query": "alpha", "queries": ["beta"]}),
            )
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }

    #[test]
    fn missing_ripgrep_is_reported_as_a_dependency_problem() {
        let error = search_error(SearchError::RipgrepUnavailable);
        assert_eq!(error.kind(), RuntimeErrorKind::Dependency);
    }

    #[test]
    fn failed_searches_stay_execution_errors() {
        let error = search_error(SearchError::Failed("boom".to_string()));
        assert_eq!(error.kind(), RuntimeErrorKind::Execution);
    }

    #[test]
    fn rejects_oversized_batch() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = crate::workspace::Workspace::new(dir.path()).unwrap();
        let mut jobs = crate::jobs::JobManager::new();
        let mut permissions = crate::permission::PermissionEngine::new().unwrap();
        let mut context = ExecutionContext::new(&workspace, &mut jobs, &mut permissions);
        let queries = vec!["x"; context.limits().max_search_queries + 1];
        let error = SearchRuntime
            .call(&mut context, &json!({"queries": queries}))
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidArguments);
    }
}

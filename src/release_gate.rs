use crate::benchmark::BenchmarkReport;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub struct ReleaseGateReport {
    pub overall: String,
    pub intel_8gb: String,
    pub apple_silicon_8gb: String,
    pub tunnel_plus_host_rss: String,
    pub evidence_files: Vec<String>,
    pub notes: Vec<String>,
}

pub fn evaluate(paths: &[PathBuf]) -> Result<ReleaseGateReport, Box<dyn std::error::Error>> {
    let mut intel = None;
    let mut arm = None;
    let mut tunnel_samples = Vec::new();
    let mut files = Vec::new();

    for path in paths {
        let report: BenchmarkReport = serde_json::from_str(&fs::read_to_string(path)?)?;
        files.push(path.display().to_string());
        if report.evaluation.tested_on_approximately_8gb_machine == Some(true)
            && report.machine.os == "macos"
        {
            let local_pass = report.evaluation.host_idle_rss_under_80_mib == Some(true)
                && report.evaluation.cold_start_under_500_ms == Some(true);
            match report.machine.arch.as_str() {
                "x86_64" => intel = Some(local_pass),
                "aarch64" | "arm64" => arm = Some(local_pass),
                _ => {}
            }
        }
        if let Some(value) = report.evaluation.tunnel_plus_host_idle_rss_under_150_mib {
            tunnel_samples.push(value);
        }
    }

    let state = |value: Option<bool>| match value {
        Some(true) => "pass".to_string(),
        Some(false) => "fail".to_string(),
        None => "not_evaluated".to_string(),
    };
    let tunnel = if tunnel_samples.is_empty() {
        "not_evaluated".to_string()
    } else if tunnel_samples.iter().all(|value| *value) {
        "pass".to_string()
    } else {
        "fail".to_string()
    };
    let intel_state = state(intel);
    let arm_state = state(arm);
    let overall = if intel_state == "fail" || arm_state == "fail" || tunnel == "fail" {
        "fail"
    } else if intel_state == "pass" && arm_state == "pass" && tunnel == "pass" {
        "pass"
    } else {
        "not_evaluated"
    }
    .to_string();

    Ok(ReleaseGateReport {
        overall,
        intel_8gb: intel_state,
        apple_silicon_8gb: arm_state,
        tunnel_plus_host_rss: tunnel,
        evidence_files: files,
        notes: vec![
            "pass requires physical 8 GB Intel and Apple Silicon evidence plus measured Tunnel + Host RSS under 150 MiB".into(),
            "missing evidence is reported as not_evaluated, never as pass".into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_architecture_evidence_stays_not_evaluated() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("intel.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": 1,
                "timestamp_unix_s": 1,
                "machine": {"os":"macos","arch":"x86_64","physical_memory_bytes":8589934592u64,"approximately_8gb":true},
                "gates": {"host_idle_rss_max_mib":80,"tunnel_plus_host_idle_rss_max_mib":150,"cold_start_max_ms":500,"local_dispatch_p95_max_ms":20,"job_memory_log_buffer_max_kib":512},
                "measurements": {
                    "workspace_info":{"iterations":1,"p50_us":1,"p95_us":1,"p99_us":1},
                    "read_file":{"iterations":1,"p50_us":1,"p95_us":1,"p99_us":1},
                    "search":{"status":"unavailable","latency":null,"detail":null},
                    "patch":{"status":"unavailable","latency":null,"detail":null},
                    "exec":{"status":"unavailable","latency":null,"detail":null},
                    "process":{"mcp_ready_ms":3,"idle_rss_kib":1024,"idle_cpu_percent":0.0}
                },
                "evaluation":{"host_idle_rss_under_80_mib":true,"cold_start_under_500_ms":true,"tunnel_plus_host_idle_rss_under_150_mib":null,"tested_on_approximately_8gb_machine":true},
                "notes":[]
            }))?,
        )?;
        let report = evaluate(&[path]).unwrap();
        assert_eq!(report.intel_8gb, "pass");
        assert_eq!(report.apple_silicon_8gb, "not_evaluated");
        assert_eq!(report.overall, "not_evaluated");
        Ok::<(), Box<dyn std::error::Error>>(())
    }
}

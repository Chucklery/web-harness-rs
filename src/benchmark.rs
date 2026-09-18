use crate::jobs::JobManager;
use crate::patch;
use crate::sandbox::SandboxBackend;
use crate::search;
use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub schema_version: u32,
    pub timestamp_unix_s: u64,
    pub machine: MachineInfo,
    pub gates: GateTargets,
    pub measurements: Measurements,
    pub evaluation: GateEvaluation,
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MachineInfo {
    pub os: String,
    pub arch: String,
    pub physical_memory_bytes: Option<u64>,
    pub approximately_8gb: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GateTargets {
    pub host_idle_rss_max_mib: u64,
    pub tunnel_plus_host_idle_rss_max_mib: u64,
    pub cold_start_max_ms: u64,
    pub local_dispatch_p95_max_ms: u64,
    pub job_memory_log_buffer_max_kib: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Measurements {
    pub workspace_info: LatencyMetric,
    pub read_file: LatencyMetric,
    pub search: OperationMetric,
    pub patch: OperationMetric,
    pub exec: OperationMetric,
    pub process: ProcessMetric,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LatencyMetric {
    pub iterations: usize,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OperationMetric {
    pub status: String,
    pub latency: Option<LatencyMetric>,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessMetric {
    pub mcp_ready_ms: Option<u64>,
    pub idle_rss_kib: Option<u64>,
    pub idle_cpu_percent: Option<f64>,
    #[serde(default)]
    pub tunnel_rss_kib: Option<u64>,
    #[serde(default)]
    pub tunnel_plus_host_rss_kib: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GateEvaluation {
    pub host_idle_rss_under_80_mib: Option<bool>,
    pub cold_start_under_500_ms: Option<bool>,
    pub tunnel_plus_host_idle_rss_under_150_mib: Option<bool>,
    pub tested_on_approximately_8gb_machine: Option<bool>,
}

pub fn run(
    workspace: &Workspace,
    iterations: usize,
    tunnel_pid: Option<u32>,
) -> Result<BenchmarkReport, Box<dyn std::error::Error>> {
    let iterations = iterations.clamp(100, 100_000);
    let workspace_info = latency(iterations, || {
        std::hint::black_box(workspace.info());
    });

    let bench_dir = tempfile::tempdir()?;
    fs::write(bench_dir.path().join("read.txt"), "needle\n".repeat(128))?;
    fs::write(bench_dir.path().join("patch.txt"), "alpha\n")?;
    let bench_workspace = Workspace::new(bench_dir.path())?;

    let read_iterations = iterations.min(10_000);
    let read_file = latency(read_iterations, || {
        let _ = std::hint::black_box(
            bench_workspace
                .read_text_bounded("read.txt", 256 * 1024)
                .expect("benchmark read"),
        );
    });

    let search = operation_metric(iterations.min(500), || {
        search::content_search(&bench_workspace, "needle", 10)
            .map(|_| ())
            .map_err(|error| error.to_string())
    });

    let patch_iterations = iterations.min(500);
    let mut patch_samples = Vec::with_capacity(patch_iterations);
    let mut alpha = true;
    let mut patch_error = None;
    for _ in 0..patch_iterations {
        let (old, new) = if alpha {
            ("alpha", "beta")
        } else {
            ("beta", "alpha")
        };
        let input = format!(
            "*** Begin Patch\n*** Update File: patch.txt\n@@\n-{old}\n+{new}\n*** End Patch"
        );
        let start = Instant::now();
        if let Err(error) = patch::apply(&bench_workspace, &input) {
            patch_error = Some(error.to_string());
            break;
        }
        patch_samples.push(start.elapsed().as_micros() as u64);
        alpha = !alpha;
    }
    let patch = samples_metric(patch_samples, patch_error);

    let exec_iterations = iterations.min(100);
    let true_path = if std::path::Path::new("/usr/bin/true").exists() {
        "/usr/bin/true"
    } else {
        "/bin/true"
    };
    let manager = JobManager::new();
    let sandboxed = SandboxBackend::detect().enforced();
    let mut exec_samples = Vec::with_capacity(exec_iterations);
    let mut exec_error = None;
    for _ in 0..exec_iterations {
        let start = Instant::now();
        match manager.run_foreground(
            &bench_workspace,
            &[true_path.to_string()],
            None,
            Some(2_000),
            sandboxed,
        ) {
            Ok(result) if result.exit_code == Some(0) => {
                exec_samples.push(start.elapsed().as_micros() as u64)
            }
            Ok(result) => {
                exec_error = Some(format!("unexpected exit code {:?}", result.exit_code));
                break;
            }
            Err(error) => {
                exec_error = Some(error.to_string());
                break;
            }
        }
    }
    let exec = samples_metric(exec_samples, exec_error);

    let process = process_metrics(workspace, tunnel_pid)?;
    let machine = machine_info();
    let evaluation = GateEvaluation {
        host_idle_rss_under_80_mib: process.idle_rss_kib.map(|rss| rss < 80 * 1024),
        cold_start_under_500_ms: process.mcp_ready_ms.map(|ms| ms < 500),
        tunnel_plus_host_idle_rss_under_150_mib: process
            .tunnel_plus_host_rss_kib
            .map(|rss| rss < 150 * 1024),
        tested_on_approximately_8gb_machine: machine.approximately_8gb,
    };

    Ok(BenchmarkReport {
        schema_version: 1,
        timestamp_unix_s: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        machine,
        gates: GateTargets {
            host_idle_rss_max_mib: 80,
            tunnel_plus_host_idle_rss_max_mib: 150,
            cold_start_max_ms: 500,
            local_dispatch_p95_max_ms: 20,
            job_memory_log_buffer_max_kib: 512,
        },
        measurements: Measurements {
            workspace_info,
            read_file,
            search,
            patch,
            exec,
            process,
        },
        evaluation,
        notes: vec![
            "workspace_info/read_file timings are in-process kernel measurements, not remote MCP tunnel latency".into(),
            "tunnel+host RSS is intentionally not evaluated by this local benchmark".into(),
            "8 GB acceptance requires evidence from physical 8 GB Intel and Apple Silicon Macs".into(),
        ],
    })
}

fn latency(mut iterations: usize, mut f: impl FnMut()) -> LatencyMetric {
    iterations = iterations.max(1);
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        f();
        samples.push(start.elapsed().as_micros() as u64);
    }
    metric_from_samples(samples)
}

fn operation_metric(
    iterations: usize,
    mut f: impl FnMut() -> Result<(), String>,
) -> OperationMetric {
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        if let Err(error) = f() {
            return OperationMetric {
                status: "unavailable".into(),
                latency: None,
                detail: Some(error),
            };
        }
        samples.push(start.elapsed().as_micros() as u64);
    }
    OperationMetric {
        status: "measured".into(),
        latency: Some(metric_from_samples(samples)),
        detail: None,
    }
}

fn samples_metric(samples: Vec<u64>, error: Option<String>) -> OperationMetric {
    if let Some(error) = error {
        return OperationMetric {
            status: "failed".into(),
            latency: None,
            detail: Some(error),
        };
    }
    if samples.is_empty() {
        return OperationMetric {
            status: "unavailable".into(),
            latency: None,
            detail: Some("no samples collected".into()),
        };
    }
    OperationMetric {
        status: "measured".into(),
        latency: Some(metric_from_samples(samples)),
        detail: None,
    }
}

fn metric_from_samples(mut samples: Vec<u64>) -> LatencyMetric {
    samples.sort_unstable();
    let iterations = samples.len();
    let percentile = |pct: usize| -> u64 {
        let index = ((iterations - 1) * pct / 100).min(iterations - 1);
        samples[index]
    };
    LatencyMetric {
        iterations,
        p50_us: percentile(50),
        p95_us: percentile(95),
        p99_us: percentile(99),
    }
}

fn process_metrics(
    workspace: &Workspace,
    tunnel_pid: Option<u32>,
) -> Result<ProcessMetric, Box<dyn std::error::Error>> {
    let binary = std::env::current_exe()?;
    let workspace_arg = workspace.root().display().to_string();
    let start = Instant::now();
    let mut child = Command::new(binary)
        .args(["serve", "--stdio", "--workspace", &workspace_arg])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let mut stdin = child.stdin.take().ok_or("missing benchmark child stdin")?;
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})
    )?;
    stdin.flush()?;

    let stdout = child
        .stdout
        .take()
        .ok_or("missing benchmark child stdout")?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let ready_ms = start.elapsed().as_millis() as u64;
    std::thread::sleep(Duration::from_millis(250));

    let rss = ps_value(child.id(), "rss").and_then(|value| value.parse::<u64>().ok());
    let cpu = ps_value(child.id(), "%cpu").and_then(|value| value.parse::<f64>().ok());
    let tunnel_rss = tunnel_pid
        .and_then(|pid| ps_value(pid, "rss"))
        .and_then(|value| value.parse::<u64>().ok());
    let tunnel_plus_host_rss_kib = match (rss, tunnel_rss) {
        (Some(host), Some(tunnel)) => Some(host + tunnel),
        _ => None,
    };
    drop(stdin);
    let _ = child.wait();

    Ok(ProcessMetric {
        mcp_ready_ms: Some(ready_ms),
        idle_rss_kib: rss,
        idle_cpu_percent: cpu,
        tunnel_rss_kib: tunnel_rss,
        tunnel_plus_host_rss_kib,
    })
}

fn ps_value(pid: u32, field: &str) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn machine_info() -> MachineInfo {
    let physical_memory_bytes = physical_memory_bytes();
    let approximately_8gb = physical_memory_bytes.map(|bytes| {
        let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        (7.0..=9.5).contains(&gib)
    });
    MachineInfo {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        physical_memory_bytes,
        approximately_8gb,
    }
}

fn physical_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        return String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok();
    }

    #[cfg(target_os = "linux")]
    {
        let text = fs::read_to_string("/proc/meminfo").ok()?;
        let kib = text
            .lines()
            .find(|line| line.starts_with("MemTotal:"))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()?;
        return Some(kib * 1024);
    }

    #[allow(unreachable_code)]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_percentiles_are_sorted() {
        let metric = metric_from_samples(vec![5, 1, 3, 2, 4]);
        assert_eq!(metric.p50_us, 3);
        assert_eq!(metric.p95_us, 4);
        assert_eq!(metric.p99_us, 4);
    }
}

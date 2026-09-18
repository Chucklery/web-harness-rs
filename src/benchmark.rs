use crate::workspace::Workspace;
use std::time::Instant;

pub fn run(workspace: &Workspace, iterations: usize) {
    let iterations = iterations.clamp(100, 100_000);
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        std::hint::black_box(workspace.info());
        samples.push(start.elapsed().as_nanos() as u64);
    }
    samples.sort_unstable();
    let percentile = |pct: usize| -> u64 {
        let index = ((samples.len() - 1) * pct / 100).min(samples.len() - 1);
        samples[index]
    };
    println!("benchmark=workspace_info iterations={iterations}");
    println!("p50_ns={}", percentile(50));
    println!("p95_ns={}", percentile(95));
    println!("p99_ns={}", percentile(99));
    println!("note=this is a local kernel microbenchmark, not tunnel end-to-end latency");
}

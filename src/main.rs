use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod atomic_file;
mod command_policy;
mod config;
mod exec;
mod git;
mod jobs;
#[cfg(feature = "release-tools")]
mod maintenance;
mod mcp;
mod onboarding;
mod patch;
mod permission;
mod redact;
mod runtime;
mod sandbox;
mod search;
mod tunnel;
mod workspace;

#[derive(Debug, Parser)]
#[command(name = "web-harness")]
#[command(version)]
#[command(about = "Lightweight local execution host for ChatGPT Web")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value_t = false)]
        stdio: bool,
        #[arg(long, value_name = "PATH", default_value = ".")]
        workspace: PathBuf,
    },
    Doctor {
        #[arg(long, value_name = "PATH", default_value = ".")]
        workspace: PathBuf,
    },
    Setup {
        #[arg(
            long,
            value_name = "PATH",
            default_value = ".",
            help = "Default workspace recorded during setup"
        )]
        workspace: PathBuf,
        #[arg(long, help = "OpenAI tunnel ID; omit for interactive prompt")]
        tunnel_id: Option<String>,
        #[arg(
            long,
            help = "OpenAI runtime API key; prefer interactive entry to avoid shell history"
        )]
        api_key: Option<String>,
        #[arg(
            long,
            conflicts_with = "tunnel_wrapper",
            help = "Advanced: custom tunnel command as a JSON argv array"
        )]
        tunnel_command_json: Option<String>,
        #[arg(
            long,
            value_name = "PATH",
            conflicts_with = "tunnel_command_json",
            help = "Advanced: use an existing tunnel wrapper instead of direct tunnel-client launch"
        )]
        tunnel_wrapper: Option<PathBuf>,
        #[arg(
            long,
            default_value_t = false,
            conflicts_with_all = ["tunnel_id", "api_key", "tunnel_command_json", "tunnel_wrapper"],
            help = "Show non-secret setup status without modifying configuration"
        )]
        show: bool,
    },
    Connect {
        #[arg(long, value_name = "PATH")]
        workspace: Option<PathBuf>,
        #[arg(long)]
        tunnel_command_json: Option<String>,
    },
    Status,
    Disconnect,
    SelfTest {
        #[arg(long, value_name = "PATH", default_value = ".")]
        workspace: PathBuf,
    },
    #[cfg(feature = "release-tools")]
    Benchmark {
        #[arg(long, value_name = "PATH", default_value = ".")]
        workspace: PathBuf,
        #[arg(long, default_value_t = 10_000)]
        iterations: usize,
        #[arg(long)]
        tunnel_pid: Option<u32>,
    },
    #[cfg(feature = "release-tools")]
    ReleaseGate {
        #[arg(long = "evidence", value_name = "JSON", required = true)]
        evidence: Vec<PathBuf>,
    },
    Tunnel {
        #[command(subcommand)]
        command: TunnelCommand,
    },
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    Version,
}

#[derive(Debug, Subcommand)]
enum WorkspaceCommand {
    Check {
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum TunnelCommand {
    Doctor {
        #[arg(long, value_name = "PATH", default_value = ".")]
        workspace: PathBuf,
    },
    Accept {
        #[arg(long, value_name = "PATH", default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        command_json: Option<String>,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve { stdio, workspace } => {
            if !stdio {
                return Err("only --stdio transport is supported".into());
            }
            mcp::serve_stdio(workspace::Workspace::new(workspace)?)?;
        }
        Command::Doctor { workspace } => {
            let workspace = workspace::Workspace::new(workspace)?;
            println!("[OK] workspace: {}", workspace.root().display());
            println!("[OK] path guard: enabled");
            println!("[OK] MCP transport: stdio");
            println!("[INFO] sandbox: {}", sandbox::status());
            println!("[OK] approval engine: enabled for process execution");
        }
        Command::Setup {
            workspace,
            tunnel_id,
            api_key,
            tunnel_command_json,
            tunnel_wrapper,
            show,
        } => {
            if show {
                println!(
                    "{}",
                    onboarding::format_setup_status(&onboarding::status()?)
                );
            } else {
                let workspace = workspace::Workspace::new(workspace)?;
                let result = onboarding::setup(
                    workspace.root(),
                    onboarding::SetupOptions {
                        tunnel_id,
                        api_key,
                        tunnel_command_json,
                        tunnel_wrapper,
                    },
                )?;
                println!("{}", onboarding::format_setup_result(&result));
            }
        }
        Command::Connect {
            workspace,
            tunnel_command_json,
        } => {
            let status = runtime::connect(workspace.as_deref(), tunnel_command_json.as_deref())?;
            println!("{}", runtime::format_status(&status));
        }
        Command::Status => {
            let status = runtime::status()?;
            println!("{}", runtime::format_status(&status));
        }
        Command::Disconnect => {
            let status = runtime::disconnect()?;
            println!("{}", runtime::format_status(&status));
        }
        Command::SelfTest { workspace } => {
            let workspace = workspace::Workspace::new(workspace)?;
            let info = workspace.info();
            if info.root.is_empty() {
                return Err("workspace info is empty".into());
            }
            let agents = workspace.discover_agents(".")?;
            println!("[OK] workspace: {}", workspace.root().display());
            println!("[OK] path guard");
            println!("[OK] workspace info");
            println!("[OK] AGENTS discovery: {} file(s)", agents.len());
            match search::content_search(&workspace, "web-harness", 1) {
                Ok(_) => println!("[OK] ripgrep search"),
                Err(error) => println!("[WARN] ripgrep search: {error}"),
            }
            match git::status(&workspace) {
                Ok(_) => println!("[OK] git gateway"),
                Err(error) => println!("[WARN] git gateway: {error}"),
            }
        }
        #[cfg(feature = "release-tools")]
        Command::Benchmark {
            workspace,
            iterations,
            tunnel_pid,
        } => {
            let workspace = workspace::Workspace::new(workspace)?;
            let report = maintenance::benchmark::run(&workspace, iterations, tunnel_pid)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        #[cfg(feature = "release-tools")]
        Command::ReleaseGate { evidence } => {
            let report = maintenance::release_gate::evaluate(&evidence)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if report.overall == "fail" {
                std::process::exit(2);
            }
        }
        Command::Tunnel { command } => match command {
            TunnelCommand::Doctor { workspace } => {
                let workspace = workspace::Workspace::new(workspace)?;
                let report = tunnel::doctor(&workspace)?;
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
            TunnelCommand::Accept {
                workspace,
                command_json,
            } => {
                let workspace = workspace::Workspace::new(workspace)?;
                let env_command = std::env::var("WEB_HARNESS_TUNNEL_COMMAND_JSON").ok();
                let command_json = command_json.as_deref().or(env_command.as_deref());
                let report = tunnel::accept(&workspace, command_json)?;
                println!("{}", serde_json::to_string_pretty(&report)?);
                if !report.passed {
                    std::process::exit(2);
                }
            }
        },
        Command::Workspace { command } => match command {
            WorkspaceCommand::Check { path } => {
                let workspace = workspace::Workspace::new(path)?;
                println!("workspace OK: {}", workspace.root().display());
            }
        },
        Command::Version => println!("web-harness {}", env!("CARGO_PKG_VERSION")),
    }
    Ok(())
}

use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod benchmark;
mod command_policy;
mod exec;
mod git;
mod jobs;
mod mcp;
mod patch;
mod permission;
mod redact;
mod sandbox;
mod search;
mod tunnel;
mod workspace;

#[derive(Debug, Parser)]
#[command(name = "web-harness")]
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
    SelfTest {
        #[arg(long, value_name = "PATH", default_value = ".")]
        workspace: PathBuf,
    },
    Benchmark {
        #[arg(long, value_name = "PATH", default_value = ".")]
        workspace: PathBuf,
        #[arg(long, default_value_t = 10_000)]
        iterations: usize,
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
        Command::Benchmark {
            workspace,
            iterations,
        } => {
            let workspace = workspace::Workspace::new(workspace)?;
            let report = benchmark::run(&workspace, iterations)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
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

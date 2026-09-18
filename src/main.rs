use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod exec;
mod git;
mod jobs;
mod mcp;
mod patch;
mod search;
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
            println!("[INFO] sandbox: not implemented yet");
            println!("[INFO] approval engine: not implemented yet");
        }
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

use clap::{Parser, Subcommand};

use vdw::commands;
use vdw::utils::logging;

/// Virtual Dev Workspace - Cross-repo development middleware
#[derive(Parser)]
#[command(name = "vdw", version, about = "Virtual Dev Workspace - Cross-repo development middleware")]
struct Cli {
    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new VDW workspace
    Init {
        /// Template to use for initialization
        #[arg(short, long)]
        template: Option<String>,
    },
    /// Create links for the workspace
    Link {
        /// Only link the specified repository URL
        #[arg(long)]
        repo: Option<String>,
    },
    /// Remove links from the workspace
    Unlink {
        /// Only unlink the specified repository URL
        #[arg(long)]
        repo: Option<String>,
    },
    /// Manage repositories in the workspace
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },
    /// Manage git worktrees
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
    /// Manage AI context filtering
    Context {
        #[command(subcommand)]
        command: ContextCommands,
    },
    /// Run diagnostics on the workspace
    Doctor,
    /// Validate workspace configuration
    Validate,
}

#[derive(Subcommand)]
enum RepoCommands {
    /// Add a repository to the workspace
    Add {
        /// Repository URL
        url: String,
        /// Local path for the repository
        #[arg(short, long)]
        path: Option<String>,
        /// Branch to checkout
        #[arg(short, long)]
        branch: Option<String>,
        /// Enable editable mode (git worktree)
        #[arg(short, long)]
        editable: bool,
    },
    /// Remove a repository from the workspace
    Remove {
        /// Name of the repository to remove
        name: String,
    },
    /// List all repositories in the workspace
    List,
}

#[derive(Subcommand)]
enum WorktreeCommands {
    /// List all worktrees
    List,
    /// Create a new worktree
    Create {
        /// Repository name
        repo: String,
        /// Branch for the worktree
        branch: String,
    },
    /// Delete a worktree
    Delete {
        /// Repository name
        repo: String,
        /// Branch of the worktree to delete
        branch: String,
        /// Force deletion even if worktree is locked
        #[arg(short, long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum ContextCommands {
    /// Preview the AI context for the workspace
    Preview,
    /// Show context statistics
    Stats,
}

fn run_command(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Init { template } => commands::init::execute(template),
        Commands::Link { repo } => commands::link::execute(repo.as_deref()),
        Commands::Unlink { repo } => commands::link::unlink(repo.as_deref()),
        Commands::Repo { command } => match command {
            RepoCommands::Add {
                url,
                path,
                branch,
                editable,
            } => commands::repo::add(url, path, branch, editable),
            RepoCommands::Remove { name } => commands::repo::remove(name),
            RepoCommands::List => commands::repo::list(),
        },
        Commands::Worktree { command } => match command {
            WorktreeCommands::List => commands::worktree::list(),
            WorktreeCommands::Create { repo, branch } => {
                commands::worktree::create(repo, branch)
            }
            WorktreeCommands::Delete { repo, branch, force } => {
                commands::worktree::delete(repo, branch, force)
            }
        },
        Commands::Context { command } => match command {
            ContextCommands::Preview => commands::context::preview(),
            ContextCommands::Stats => commands::context::stats(),
        },
        Commands::Doctor => commands::doctor::execute(),
        Commands::Validate => commands::doctor::validate(),
    }
}

fn main() {
    let cli = Cli::parse();

    if let Err(e) = logging::init(cli.verbose) {
        eprintln!("Warning: Failed to initialize logging: {}", e);
    }

    if let Err(e) = run_command(cli) {
        eprintln!("\x1b[31mError:\x1b[0m {}", e);

        // Provide suggestions for common errors
        let err_str = e.to_string();
        if err_str.contains("not found") || err_str.contains("no such file") {
            eprintln!("\n\x1b[33mSuggestion:\x1b[0m Run `vdw init` to initialize a workspace first.");
        } else if err_str.contains("already exists") {
            eprintln!("\n\x1b[33mSuggestion:\x1b[0m Use `vdw doctor` to check the workspace state.");
        }

        std::process::exit(1);
    }
}

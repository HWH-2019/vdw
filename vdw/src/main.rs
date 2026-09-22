use clap::{Parser, Subcommand};

use vdw::commands;
use vdw::utils::logging;

/// Version string shown by `--version` and in the debug banner.
/// Debug builds are tagged with a `-debug` suffix so they are easy to tell apart
/// from release builds at a glance.
#[cfg(debug_assertions)]
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-debug");
#[cfg(not(debug_assertions))]
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Virtual Dev Workspace - Declarative multi-repository worktree orchestration
#[derive(Parser)]
#[command(
    name = "vdw",
    version = VERSION,
    about = "Virtual Dev Workspace - Declarative multi-repository worktree orchestration"
)]
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
        /// Workspace name; created under the configured workspace root
        name: String,
        /// Template to use for initialization
        #[arg(short, long)]
        template: Option<String>,
    },
    /// Manage global VDW configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Materialize configured dependencies in the workspace
    Link {
        /// Only link the specified repository URL
        #[arg(long)]
        repo: Option<String>,
        /// Require a matching lock file and avoid network/ref updates
        #[arg(long)]
        frozen: bool,
        /// Watch for broken HardLinks after linking
        #[arg(long)]
        watch: bool,
    },
    /// Remove managed links and editable worktrees from the workspace
    Unlink {
        /// Only unlink the specified repository URL
        #[arg(long)]
        repo: Option<String>,
    },
    /// Refresh read-only projections and migrate legacy editable worktrees
    Relink {
        /// Deprecated compatibility flag; native editable worktrees are always preserved
        #[arg(long, hide = true)]
        allow_dirty: bool,
    },
    /// Inspect and resolve workspace/Store link conflicts
    Conflicts {
        #[command(subcommand)]
        command: ConflictCommands,
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
    Doctor {
        /// Check HardLink integrity by comparing inode metadata
        #[arg(long)]
        check_links: bool,
    },
    /// Validate workspace configuration
    Validate,
    /// Show aggregate workspace topology status
    Status {
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Resolve declared revisions and inspect or update .vdw.lock
    Sync {
        /// Fetch and report drift without changing .vdw.lock (default)
        #[arg(long, conflicts_with = "update_lock")]
        dry_run: bool,
        /// Accept resolved revisions and write .vdw.lock
        #[arg(long)]
        update_lock: bool,
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Inspect the shared Content Store and plan safe garbage collection
    Store {
        #[command(subcommand)]
        command: StoreCommands,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Set a global VDW configuration value
    Set {
        #[command(subcommand)]
        setting: ConfigSetCommands,
    },
    /// Show the effective global configuration
    Show {
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ConfigSetCommands {
    /// Set the parent directory used for new workspaces
    WorkspaceRoot {
        /// Absolute path or a path beginning with '~/'.
        path: std::path::PathBuf,
    },
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
        /// Discard uncommitted changes in an editable worktree
        #[arg(long)]
        force: bool,
    },
    /// List all repositories in the workspace
    List,
    /// Show repository dirty status (JSON output)
    Status {
        /// Name of the repository to check
        name: String,
    },
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
    Preview {
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Show context statistics
    Stats {
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ConflictCommands {
    /// List files whose workspace and Store versions differ
    List {
        /// Only inspect the specified repository URL
        #[arg(long)]
        repo: Option<String>,
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Resolve conflicts by accepting one side
    Resolve {
        /// Accept either the Store or workspace version
        #[arg(long, value_enum)]
        accept: commands::conflicts::ConflictResolutionSide,
        /// Only resolve the specified repository URL
        #[arg(long)]
        repo: Option<String>,
        /// Resolve only this workspace-relative path; repeat for multiple files
        #[arg(long = "path")]
        paths: Vec<std::path::PathBuf>,
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum StoreCommands {
    /// Show Store objects, references, size, and retention state
    Status {
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Plan garbage collection without deleting Store content
    Gc {
        /// Required safety flag; v0.9.1 never deletes Store content
        #[arg(long)]
        dry_run: bool,
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
    },
}

fn run_command(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Init { name, template } => commands::init::execute(name, template),
        Commands::Config { command } => match command {
            ConfigCommands::Set { setting } => match setting {
                ConfigSetCommands::WorkspaceRoot { path } => {
                    commands::config::set_workspace_root(path)
                }
            },
            ConfigCommands::Show { json } => commands::config::show(json),
        },
        Commands::Link {
            repo,
            frozen,
            watch,
        } => commands::link::execute(repo.as_deref(), watch, frozen),
        Commands::Unlink { repo } => commands::link::unlink(repo.as_deref()),
        Commands::Relink { allow_dirty } => commands::link::relink(allow_dirty),
        Commands::Conflicts { command } => match command {
            ConflictCommands::List { repo, json } => {
                commands::conflicts::list(repo.as_deref(), json)
            }
            ConflictCommands::Resolve {
                accept,
                repo,
                paths,
                json,
            } => commands::conflicts::resolve(repo.as_deref(), accept, &paths, json),
        },
        Commands::Repo { command } => match command {
            RepoCommands::Add {
                url,
                path,
                branch,
                editable,
            } => commands::repo::add(url, path, branch, editable),
            RepoCommands::Remove { name, force } => commands::repo::remove(name, force),
            RepoCommands::List => commands::repo::list(),
            RepoCommands::Status { name } => commands::repo::status(name),
        },
        Commands::Worktree { command } => match command {
            WorktreeCommands::List => commands::worktree::list(),
            WorktreeCommands::Create { repo, branch } => commands::worktree::create(repo, branch),
            WorktreeCommands::Delete {
                repo,
                branch,
                force,
            } => commands::worktree::delete(repo, branch, force),
        },
        Commands::Context { command } => match command {
            ContextCommands::Preview { json } => commands::context::preview(json),
            ContextCommands::Stats { json } => commands::context::stats(json),
        },
        Commands::Doctor { check_links } => commands::doctor::execute(check_links),
        Commands::Validate => commands::doctor::validate(),
        Commands::Status { json } => commands::status::show(json),
        Commands::Sync {
            dry_run,
            update_lock,
            json,
        } => commands::sync::execute(dry_run, update_lock, json),
        Commands::Store { command } => match command {
            StoreCommands::Status { json } => commands::store::status(json),
            StoreCommands::Gc { dry_run, json } => commands::store::gc_dry_run(dry_run, json),
        },
    }
}

fn main() {
    // Tag debug builds at runtime: print a banner to stderr so it is obvious a
    // debug binary is running. Kept on stderr to avoid polluting command stdout
    // (e.g. `status --json` parsed by the VS Code extension).
    #[cfg(debug_assertions)]
    eprintln!("\x1b[33m[vdw {} — debug build]\x1b[0m", VERSION);

    let cli = Cli::parse();

    if let Err(e) = logging::init(cli.verbose) {
        eprintln!("Warning: Failed to initialize logging: {}", e);
    }

    if let Err(e) = run_command(cli) {
        eprintln!("\x1b[31mError:\x1b[0m {}", e);

        // Provide suggestions for common errors
        let err_str = e.to_string();
        if err_str.contains("not found") || err_str.contains("no such file") {
            eprintln!(
                "\n\x1b[33mSuggestion:\x1b[0m Run `vdw init <name>` to initialize a workspace first."
            );
        } else if err_str.contains("already exists") {
            eprintln!(
                "\n\x1b[33mSuggestion:\x1b[0m Use `vdw doctor` to check the workspace state."
            );
        }

        std::process::exit(1);
    }
}

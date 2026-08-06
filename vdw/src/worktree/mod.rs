pub mod git_ops;
pub mod manager;

// Re-export commonly used types
pub use manager::{create_worktree, list_worktrees, remove_worktree, WorktreeInfo};
pub use git_ops::{
    check_dirty, open_bare_repo, open_repo, list_branches, list_remote_branches,
    find_or_create_local_branch, get_branch_commit, get_head_commit, DirtyStatus,
};

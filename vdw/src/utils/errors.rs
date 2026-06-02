use thiserror::Error;

/// Top-level VDW error type
#[derive(Error, Debug)]
pub enum VDWError {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Store error: {0}")]
    Store(#[from] StoreError),

    #[error("Link error: {0}")]
    Link(#[from] LinkError),

    #[error("Git error: {0}")]
    Git(#[from] GitError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Workspace not found: {0}")]
    WorkspaceNotFound(String),

    #[error("Already initialized: {0}")]
    AlreadyInitialized(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
}

/// Configuration-related errors
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Config file not found: {0}")]
    NotFound(String),

    #[error("Config parse error: {0}")]
    ParseError(String),

    #[error("Config validation error: {0}")]
    ValidationError(String),

    #[error("Environment variable not found: {0}")]
    EnvVarNotFound(String),

    #[error("Invalid config value: {key} = {value}")]
    InvalidValue { key: String, value: String },
}

/// Content store related errors
#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Store not initialized")]
    NotInitialized,

    #[error("Object not found: {0}")]
    ObjectNotFound(String),

    #[error("Hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },

    #[error("Store corruption: {0}")]
    Corruption(String),

    #[error("Disk space insufficient: needed {needed}, available {available}")]
    InsufficientSpace { needed: u64, available: u64 },
}

/// Link-related errors
#[derive(Error, Debug)]
pub enum LinkError {
    #[error("Link target not found: {0}")]
    TargetNotFound(String),

    #[error("Link already exists: {0}")]
    AlreadyExists(String),

    #[error("Cross-device link not supported: from {from} to {to}")]
    CrossDevice { from: String, to: String },

    #[error("Permission denied creating link: {0}")]
    PermissionDenied(String),

    #[error("Platform unsupported: {0}")]
    UnsupportedPlatform(String),
}

/// Git-related errors
#[derive(Error, Debug)]
pub enum GitError {
    #[error("Repository not found: {0}")]
    RepoNotFound(String),

    #[error("Branch not found: {0}")]
    BranchNotFound(String),

    #[error("Worktree error: {0}")]
    WorktreeError(String),

    #[error("Git operation failed: {0}")]
    OperationFailed(String),

    #[error("Merge conflict: {0}")]
    MergeConflict(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),
}

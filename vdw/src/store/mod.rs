pub mod alternates;
pub mod editable_locks;
pub mod manager;
pub mod metadata;
pub mod object_pool;
pub mod registry;

// Re-export commonly used types
pub use alternates::{remove_alternates, setup_alternates, validate_alternates};
pub use editable_locks::{
    acquire_editable_lock, check_editable_lock, cleanup_stale_locks, load_editable_locks,
    release_editable_lock, save_editable_locks,
};
pub use manager::{
    get_store_path, get_vdw_home, init_store, init_store_at, is_store_initialized,
    resolve_store_path, GlobalConfig,
};
pub use object_pool::{clone_to_store, compute_repo_hash, list_store_repos, remove_from_store};
pub use registry::{
    load_workspace_registry, register_workspace, WorkspaceRegistration, WorkspaceRegistry,
};

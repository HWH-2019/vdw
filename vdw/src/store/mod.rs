pub mod alternates;
pub mod editable_locks;
pub mod manager;
pub mod metadata;
pub mod object_pool;

// Re-export commonly used types
pub use manager::{get_store_path, get_vdw_home, init_store, init_store_at, is_store_initialized, GlobalConfig};
pub use object_pool::{clone_to_store, compute_repo_hash, list_store_repos, remove_from_store};
pub use alternates::{remove_alternates, setup_alternates, validate_alternates};
pub use editable_locks::{load_editable_locks, save_editable_locks, check_editable_lock, acquire_editable_lock, release_editable_lock, cleanup_stale_locks};
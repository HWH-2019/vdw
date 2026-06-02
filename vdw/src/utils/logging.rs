use tracing_subscriber::EnvFilter;

/// Initialize the logging subsystem with tracing-subscriber.
///
/// When `verbose` is true, sets the default level to DEBUG.
/// Otherwise, respects the RUST_LOG environment variable (defaulting to WARN).
pub fn init(verbose: bool) -> anyhow::Result<()> {
    let filter = if verbose {
        EnvFilter::try_new("debug")?
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .init();

    Ok(())
}

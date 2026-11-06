//! Local-only dashboard. UI assets and the HTTP service ship in the CLI binary.
mod handoff;
mod http;
pub mod preferences;
mod remotes;
pub mod service;

pub fn run(port: u16, no_open: bool) -> anyhow::Result<()> {
    tokio::runtime::Runtime::new()?.block_on(http::serve(port, no_open))
}

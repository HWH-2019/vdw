use crate::lifecycle::{Engine, Operation};
use anyhow::Result;
use std::process::{Command, Stdio};

/// Execute the same durable plan used by CLI clients, then start only its own
/// explicitly configured derived summaries. Never recover unrelated jobs.
pub fn execute_plan(id: &str) -> Result<Operation> {
    let mut engine = Engine::open()?;
    let operation = engine.execute_plan(id)?;
    drop(engine);
    start_summaries(&operation);
    Ok(operation)
}

pub fn start_summaries(operation: &Operation) {
    for result in &operation.results {
        if result.outcome == "archived" && result.memory_state == "pending" {
            if let (Some(archive), Ok(exe)) = (&result.archive_id, std::env::current_exe()) {
                if let Ok(mut child) = Command::new(exe)
                    .args(["lifecycle", "memory-worker", archive])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
            }
        }
    }
}

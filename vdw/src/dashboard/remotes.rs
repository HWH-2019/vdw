use serde_json::{json, Value};
use std::time::Duration;

pub async fn inspect(source: &str, branch: Option<&str>) -> Value {
    let mut command = tokio::process::Command::new("git");
    command.args([
        "-c",
        "protocol.ext.allow=never",
        "ls-remote",
        "--",
        source,
        "HEAD",
    ]);
    if let Some(branch) = branch {
        command.arg(format!("refs/heads/{branch}"));
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=8")
        .kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(12), command.output()).await;
    match result {
        Ok(Ok(output)) if output.status.success() => success(&output.stdout, branch),
        Ok(Ok(output)) => failure(&String::from_utf8_lossy(&output.stderr)),
        _ => failure("network timeout"),
    }
}

fn success(stdout: &[u8], branch: Option<&str>) -> Value {
    let refs = String::from_utf8_lossy(stdout);
    let reference = branch.map(|name| format!("refs/heads/{name}"));
    let present = reference.as_ref().map(|reference| {
        refs.lines().any(|line| {
            line.split_once('\t')
                .map(|(_, name)| name == reference)
                .unwrap_or(false)
        })
    });
    json!({"state":if present == Some(false) {"branch_missing"} else {"available"},
        "repository_state":"available", "branch_state":match present {Some(true)=>"present",Some(false)=>"missing",None=>"not_applicable"},
        "branch":branch,"checked_at":chrono::Utc::now()})
}

fn failure(stderr: &str) -> Value {
    let error = stderr.to_lowercase();
    let state = if error.contains("authentication")
        || error.contains("permission denied")
        || error.contains("403")
        || error.contains("could not read username")
    {
        "auth_error"
    } else if error.contains("not found")
        || error.contains("does not appear")
        || error.contains("404")
    {
        "not_found_or_forbidden"
    } else {
        "network_error"
    };
    // Private repositories often return 404. Failure never proves deletion or a missing branch.
    json!({"state":state,"repository_state":state,"branch_state":"unknown","checked_at":chrono::Utc::now()})
}

pub fn local() -> Value {
    json!({"state":"local","repository_state":"local","branch_state":"not_applicable"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_access_and_exact_branch_existence_are_independent() {
        let refs = b"123\tHEAD\n123\trefs/heads/feature/one-more\n";
        let missing = success(refs, Some("feature/one"));
        assert_eq!(missing["repository_state"], "available");
        assert_eq!(missing["branch_state"], "missing");
        assert_eq!(
            success(refs, Some("feature/one-more"))["branch_state"],
            "present"
        );
        assert_eq!(success(b"", None)["repository_state"], "available");
        for message in [
            "repository not found",
            "HTTP 404",
            "Permission denied",
            "network timeout",
        ] {
            let failed = failure(message);
            assert_eq!(failed["branch_state"], "unknown");
            assert_ne!(failed["repository_state"], "deleted");
        }
    }
}

//! Future cloud/message adapters implement these contracts. None are implicitly enabled.
use super::{
    io::hash,
    types::{ArchiveTrigger, ProviderCapabilities},
};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutArtifact {
    pub workspace_id: String,
    pub archive_id: String,
    pub resource_kind: String,
    pub content_hash: String,
    pub local_path: PathBuf,
    pub credential_ref: Option<String>,
}
impl PutArtifact {
    pub fn idempotency_key(&self) -> String {
        hash(
            &serde_json::to_vec(&(
                &self.workspace_id,
                &self.archive_id,
                &self.resource_kind,
                &self.content_hash,
            ))
            .expect("String tuple serialization is infallible"),
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderReceipt {
    pub provider: String,
    pub remote_id: String,
    pub remote_version: String,
    pub checksum: String,
    pub persistence_state: String,
}
pub trait ArchiveProvider {
    fn capabilities(&self) -> ProviderCapabilities;
    fn put_manifest(&self, _request: &PutArtifact) -> Result<ProviderReceipt> {
        bail!("Unsupported provider manifest upload")
    }
    fn get_manifest(&self, _remote_id: &str, _version: &str) -> Result<Vec<u8>> {
        bail!("Unsupported provider manifest download")
    }
    fn put_memory(&self, _request: &PutArtifact) -> Result<ProviderReceipt> {
        bail!("Unsupported provider memory upload")
    }
    fn list_memory(&self, _workspace_id: &str) -> Result<Vec<ProviderReceipt>> {
        bail!("Unsupported provider memory search")
    }
    fn put_blob(&self, _request: &PutArtifact) -> Result<ProviderReceipt> {
        bail!("Unsupported provider blob upload")
    }
    fn get_blob(&self, _receipt: &ProviderReceipt, _target: &std::path::Path) -> Result<()> {
        bail!("Unsupported provider blob download")
    }
    fn verify_receipt(&self, _receipt: &ProviderReceipt) -> Result<()> {
        bail!("Unsupported provider read-back verification")
    }
}
pub struct ReservedProvider {
    pub name: String,
}
impl ArchiveProvider for ReservedProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            provider: self.name.clone(),
            implemented: false,
            manifest: false,
            memory: false,
            blobs: false,
            download: false,
        }
    }
}
pub trait TriggerSource {
    fn name(&self) -> &str;
    /// Adapters authenticate their source; an event is data, never execution authorization.
    fn poll(&self, _cursor: Option<&str>) -> Result<(Vec<ArchiveTrigger>, Option<String>)>;
}
pub struct ReservedTriggerSource {
    pub name: String,
}
impl TriggerSource for ReservedTriggerSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn poll(&self, _cursor: Option<&str>) -> Result<(Vec<ArchiveTrigger>, Option<String>)> {
        bail!("Unsupported external trigger source; no listener or archive authorization is configured")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reservations_never_report_an_upload_or_external_event_as_success() {
        let provider = ReservedProvider {
            name: "kbase".into(),
        };
        assert!(!provider.capabilities().implemented);
        let request = PutArtifact {
            workspace_id: "ws-a".into(),
            archive_id: "archive-a".into(),
            resource_kind: "memory".into(),
            content_hash: "sha".into(),
            local_path: "/unused".into(),
            credential_ref: None,
        };
        assert!(provider.put_memory(&request).is_err());
        assert!(provider.get_manifest("id", "version").is_err());
        assert!(ReservedTriggerSource {
            name: "future".into()
        }
        .poll(None)
        .is_err());
    }
}

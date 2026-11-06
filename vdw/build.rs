//! Verify checked-in frontend artifacts without requiring Node at Cargo build time.
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn digest(path: &Path) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            fs::read(path).unwrap_or_else(|e| panic!("Cannot read {}: {e}", path.display()))
        )
    )
}
fn verify(root: &Path, files: &serde_json::Value) {
    for (name, expected) in files.as_object().expect("Invalid dashboard manifest") {
        let relative = Path::new(name);
        assert!(
            relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "Unsafe dashboard manifest path"
        );
        let path = root.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        assert_eq!(digest(&path), expected.as_str().unwrap(), "Stale/corrupt dashboard asset: {name}. Run npm ci && npm run build in vdw-dashboard before compiling VDW.");
    }
}
fn main() {
    let crate_root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let assets = crate_root.join("dashboard-assets");
    let manifest_path = assets.join("manifest.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let manifest:serde_json::Value=serde_json::from_slice(&fs::read(&manifest_path)
        .expect("Dashboard build missing. Run npm ci && npm run build in vdw-dashboard. Released source packages include these resources; end users never need Node/npm.")).unwrap();
    verify(&assets, &manifest["files"]);
    let frontend = crate_root.parent().unwrap().join("vdw-dashboard");
    if frontend.join("package.json").exists() {
        println!("cargo:rerun-if-changed={}", frontend.join("src").display());
        println!(
            "cargo:rerun-if-changed={}",
            frontend.join("public").display()
        );
        verify(&frontend, &manifest["sources"]);
    }
}

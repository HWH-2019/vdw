use crate::lifecycle::io::{hash, read_or, save, Guard};
use crate::store::manager::get_vdw_home;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub default_editor: String,
    pub refresh_seconds: u64,
    pub remote_cache_seconds: u64,
    pub theme: String,
    pub editor_program: String,
    pub editor_args: Vec<String>,
    pub workspace_editors: BTreeMap<String, String>,
    pub remote_hosts: BTreeMap<String, String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            default_editor: "auto".into(),
            refresh_seconds: 15,
            remote_cache_seconds: 300,
            theme: "light".into(),
            editor_program: String::new(),
            editor_args: vec!["{workspace}".into()],
            workspace_editors: BTreeMap::new(),
            remote_hosts: BTreeMap::new(),
        }
    }
}
fn config_path() -> Result<PathBuf> {
    Ok(get_vdw_home()?.join("dashboard/config.json"))
}
pub fn load() -> Result<Preferences> {
    read_or(&config_path()?)
}
pub fn version(p: &Preferences) -> Result<String> {
    Ok(hash(&serde_json::to_vec(p)?))
}
pub fn store(p: Preferences, expected: &str) -> Result<Preferences> {
    let _guard = Guard::acquire(&get_vdw_home()?)?;
    if version(&load()?)? != expected {
        bail!("PLAN_STALE: dashboard preferences changed; reload first");
    }
    if !(5..=3600).contains(&p.refresh_seconds) || !(30..=86400).contains(&p.remote_cache_seconds) {
        bail!("Refresh must be 5–3600 seconds; remote cache must be 30–86400 seconds");
    }
    if !["light", "dark", "system"].contains(&p.theme.as_str()) {
        bail!("Invalid theme");
    }
    let ids = [
        "auto", "vscode", "cursor", "qoder", "codex", "system", "custom",
    ];
    if !ids.contains(&p.default_editor.as_str())
        || p.workspace_editors
            .values()
            .any(|e| !ids.contains(&e.as_str()))
    {
        bail!("Unknown editor");
    }
    if p.default_editor == "custom" && !Path::new(&p.editor_program).is_absolute() {
        bail!("Custom editor requires an absolute executable path");
    }
    for base in p.remote_hosts.values() {
        if !(base.starts_with("https://") || base.starts_with("http://"))
            || base.contains(['\n', '\r', '@', '?', '#'])
        {
            bail!("Remote web bases must be HTTP(S) URLs without credentials, query or fragment");
        }
    }
    save(&config_path()?, &p)?;
    Ok(p)
}
fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p).any(|d| {
                d.join(name).is_file() || cfg!(windows) && d.join(format!("{name}.exe")).is_file()
            })
        })
        .unwrap_or(false)
}
pub fn editors() -> serde_json::Value {
    let available = |program: &str, app: &str| {
        on_path(program) || cfg!(target_os = "macos") && Path::new(app).is_dir()
    };
    serde_json::json!([
        {"id":"auto","name":"自动选择编辑器","available":true},
        {"id":"vscode","name":"Visual Studio Code","available":available("code","/Applications/Visual Studio Code.app")},
        {"id":"cursor","name":"Cursor","available":available("cursor","/Applications/Cursor.app")},
        {"id":"qoder","name":"Qoder","available":available("qoder","/Applications/Qoder IDE.app")},
        {"id":"codex","name":"Codex","available":on_path("codex") || codex_app_available()},
        {"id":"system","name":"系统关联程序 / 文件管理器","available":true},
        {"id":"custom","name":"自定义编辑器","available":true}
    ])
}
fn codex_app_available() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    if Path::new("/Applications/Codex.app").is_dir() {
        return true;
    }
    let info = "/Applications/ChatGPT.app/Contents/Info.plist";
    Path::new(info).is_file()
        && Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", "Print:CFBundleIdentifier", info])
            .output()
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).trim() == "com.openai.codex"
            })
            .unwrap_or(false)
}
pub fn selected_editor(requested: Option<&str>, workspace_id: &str) -> Result<String> {
    let p = load()?;
    let mut selected = requested
        .or_else(|| p.workspace_editors.get(workspace_id).map(String::as_str))
        .unwrap_or(&p.default_editor)
        .to_string();
    if selected == "auto" {
        selected = editors()
            .as_array()
            .unwrap()
            .iter()
            .find(|e| {
                ["vscode", "cursor", "qoder", "codex"].contains(&e["id"].as_str().unwrap_or(""))
                    && e["available"] == true
            })
            .and_then(|e| e["id"].as_str())
            .context("未找到编辑器，请在配置中选择系统关联程序或设置自定义编辑器")?
            .to_string();
    }
    Ok(selected)
}
pub fn launch(path: &Path, requested: Option<&str>, workspace_id: &str) -> Result<String> {
    let p = load()?;
    let selected = selected_editor(requested, workspace_id)?;
    let mut command = match selected.as_str() {
        "custom" => {
            if !Path::new(&p.editor_program).is_absolute() {
                bail!("Custom editor requires an absolute executable path");
            }
            let mut c = Command::new(&p.editor_program);
            let mut inserted = false;
            for arg in &p.editor_args {
                if arg.contains("{workspace}") {
                    inserted = true;
                }
                c.arg(arg.replace("{workspace}", &path.to_string_lossy()));
            }
            if !inserted {
                c.arg(path);
            }
            c
        }
        "system" => system_open(path),
        "vscode" | "cursor" | "qoder" | "codex" => {
            let (program, app) = match selected.as_str() {
                "vscode" => ("code", "Visual Studio Code"),
                "cursor" => ("cursor", "Cursor"),
                "qoder" => ("qoder", "Qoder IDE"),
                _ => ("codex", "Codex"),
            };
            if selected == "codex" && codex_app_available() {
                let mut c = Command::new("/usr/bin/open");
                c.args(["-b", "com.openai.codex"]).arg(format!(
                    "codex://new?path={}",
                    super::service::url_component(&path.to_string_lossy())
                ));
                c
            } else if cfg!(target_os = "macos")
                && Path::new(&format!("/Applications/{app}.app")).is_dir()
            {
                let mut c = Command::new("open");
                c.args(["-a", app]).arg(path);
                c
            } else {
                let mut c = Command::new(program);
                if selected == "codex" {
                    c.arg("app");
                } else if selected == "qoder" {
                    // The qoder CLI dispatcher may select a terminal agent. An IDE-only flag
                    // makes that dispatcher reject the request instead of treating the path as a prompt.
                    c.arg("--reuse-window");
                }
                c.arg(path);
                c
            }
        }
        _ => bail!("Unknown editor"),
    };
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("无法启动编辑器，请检查安装位置和配置")?;
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(selected)
}
pub fn system_open(path: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = Command::new("explorer.exe");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = Command::new("xdg-open");
    command.arg(path);
    command
}

use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use vdw::types::config::{Dependency, Scope, VDWConfig, WorkspaceMeta};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
}
impl Fixture {
    fn rebuild(&self, workspaces: &[&Path]) -> Value {
        let mut args = vec![
            "workspace",
            "rebuild",
            "--preview",
            "--search-history",
            "--json",
        ];
        let paths = workspaces
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        args.extend(paths.iter().map(String::as_str));
        self.json(&args)
    }
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let home = root.join("home");
        fs::create_dir(&home).unwrap();
        Self {
            _temp: temp,
            root,
            home,
        }
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vdw"))
            .args(args)
            .env("HOME", &self.home)
            .current_dir(&self.root)
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> Output {
        let o = self.run(args);
        assert!(
            o.status.success(),
            "{:?}\nstdout:{}\nstderr:{}",
            args,
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        o
    }
    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_slice(&self.ok(args).stdout).unwrap()
    }
    fn execute(&self, p: &Value) -> Value {
        self.json(&[
            "lifecycle",
            "plan",
            "execute",
            p["plan_id"].as_str().unwrap(),
            "--confirm",
            "--json",
        ])
    }
    fn workspace(&self, name: &str, count: usize, editable: bool) -> PathBuf {
        self.workspace_with_dependency_prefix(name, count, editable, "packages")
    }
    fn workspace_with_dependency_prefix(
        &self,
        name: &str,
        count: usize,
        editable: bool,
        prefix: &str,
    ) -> PathBuf {
        let workspace = self.root.join(name);
        fs::create_dir(&workspace).unwrap();
        let mut deps = vec![];
        for n in 0..count {
            let source = self.root.join(format!("{name}-source-{n}"));
            fs::create_dir(&source).unwrap();
            git(&source, &["init", "--quiet", "--initial-branch=main"]);
            fs::write(source.join("version.txt"), format!("{name}-{n}\n")).unwrap();
            fs::write(source.join(".gitignore"), "node_modules/\n.env\n").unwrap();
            git(&source, &["add", "."]);
            git(
                &source,
                &[
                    "-c",
                    "user.name=VDW Test",
                    "-c",
                    "user.email=vdw@example.com",
                    "commit",
                    "--quiet",
                    "-m",
                    "initial",
                ],
            );
            deps.push(Dependency {
                remote: String::new(),
                local: source.to_string_lossy().into_owned(),
                origin: String::new(),
                repo: String::new(),
                path: format!("{prefix}/repo{n}"),
                branch: Some("main".into()),
                tag: None,
                commit: None,
                scope: Scope {
                    include: vec!["**/*".into()],
                    exclude: vec![],
                },
                editable,
            });
        }
        let c = VDWConfig {
            workspace: WorkspaceMeta {
                name: name.into(),
                version: "1.0".into(),
            },
            dependencies: deps,
        };
        fs::write(workspace.join(".vdwrc"), serde_yaml::to_string(&c).unwrap()).unwrap();
        fs::create_dir_all(workspace.join("notes/empty")).unwrap();
        fs::write(
            workspace.join("notes/decisions.md"),
            "# 决策\n采用批量查询\n\n# 待办\n- [ ] 在线验证\n",
        )
        .unwrap();
        fs::create_dir(workspace.join(".vscode")).unwrap();
        fs::write(workspace.join(".vscode/settings.json"), "{}\n").unwrap();
        for args in [
            &["sync", "--update-lock", "--json"][..],
            &["link", "--frozen"][..],
        ] {
            let o = Command::new(env!("CARGO_BIN_EXE_vdw"))
                .args(args)
                .env("HOME", &self.home)
                .current_dir(&workspace)
                .output()
                .unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        }
        workspace
    }
    fn archive_plan(&self, workspace: &Path) -> Value {
        self.json(&[
            "workspace",
            "archive",
            workspace.to_str().unwrap(),
            "--preview",
            "--json",
        ])
    }
    fn archive(&self, workspace: &Path) -> Value {
        self.execute(&self.archive_plan(workspace))
    }
    fn registry(&self) -> Value {
        serde_json::from_slice(&fs::read(self.home.join(".vdw/workspaces.json")).unwrap()).unwrap()
    }
    fn configure(&self, cfg: Value) {
        let path = self.root.join("lifecycle.json");
        fs::write(&path, serde_json::to_vec(&cfg).unwrap()).unwrap();
        let p = self.json(&[
            "config",
            "set",
            "lifecycle-file",
            path.to_str().unwrap(),
            "--preview",
            "--json",
        ]);
        self.execute(&p);
    }
}

#[test]
fn rebuild_preview_is_read_only_and_uses_actual_head_over_lock_baseline() {
    let f = Fixture::new();
    let ws = f.workspace_with_dependency_prefix("rebuild-head", 1, true, "./packages");
    let repo = ws.join("packages/repo0");
    fs::write(repo.join("version.txt"), "new committed version\n").unwrap();
    git(&repo, &["add", "version.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=VDW Test",
            "-c",
            "user.email=vdw@example.com",
            "commit",
            "--quiet",
            "-m",
            "advance",
        ],
    );
    let actual = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    fn files(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        let mut result = std::collections::BTreeMap::new();
        for entry in walkdir::WalkDir::new(root) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                result.insert(entry.path().to_path_buf(), fs::read(entry.path()).unwrap());
            }
        }
        result
    }
    let before = files(&f.root);
    let p = f.rebuild(&[&ws]);
    assert_eq!(p["kind"], "rebuild_preview");
    assert_eq!(p["executable"], false);
    assert!(p.get("plan_id").is_none());
    let row = &p["workspaces"][0];
    assert_eq!(row["classification"], "skipped");
    assert_eq!(row["repositories"][0]["original_commit"], actual);
    let evidence = row["repositories"][0]["evidence"].as_array().unwrap();
    assert!(evidence
        .iter()
        .any(|e| e["authority"] == "baseline" && e["commit"] != actual));
    assert_eq!(
        before,
        files(&f.root),
        "preview changed a workspace, Store, registry, logs or lifecycle state"
    );
    assert!(!f
        .run(&["workspace", "rebuild", ws.to_str().unwrap()])
        .status
        .success());
    assert!(!f
        .run(&["workspace", "rebuild", "--preview", "--execute"])
        .status
        .success());
}

#[test]
fn rebuild_missing_readonly_projection_is_rebuildable_at_exact_lock() {
    let f = Fixture::new();
    let ws = f.workspace_with_dependency_prefix("rebuild-projection", 1, false, "./references");
    fs::remove_dir_all(ws.join("references/repo0")).unwrap();
    fs::create_dir_all(ws.join("packages/owned-empty")).unwrap();
    fs::write(ws.join("packages/owned.md"), "preserve this material").unwrap();
    let p = f.rebuild(&[&ws]);
    let row = &p["workspaces"][0];
    assert_eq!(row["classification"], "rebuildable", "{p}");
    assert_eq!(
        row["repositories"][0]["evidence"][0]["authority"],
        "exact_projection"
    );
    assert!(row["materials"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["path"] == "packages/owned.md"));
    assert!(!ws.join("references/repo0").exists());
}

#[test]
fn rebuild_broken_editable_never_promotes_lock_or_branch_candidate_to_original_head() {
    let f = Fixture::new();
    let ws = f.workspace("rebuild-broken", 1, true);
    let repo = ws.join("packages/repo0");
    fs::write(repo.join(".git"), "gitdir: /missing-worktree-metadata\n").unwrap();
    let p = f.rebuild(&[&ws]);
    let row = &p["workspaces"][0];
    assert_eq!(row["classification"], "blocked");
    let r = &row["repositories"][0];
    assert!(r["original_commit"].is_null());
    assert!(r["dirty"].is_null());
    assert!(r["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["authority"] == "candidate"));
    assert!(r["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["code"] == "dirty_state_unknown"));
    assert!(repo.join("version.txt").exists());
}

#[test]
fn rebuild_dirty_workspace_skips_all_repositories_and_retains_other_blockers() {
    let f = Fixture::new();
    let ws = f.workspace("rebuild-dirty", 2, true);
    fs::write(
        ws.join("packages/repo0/.git"),
        "gitdir: /missing-worktree-metadata\n",
    )
    .unwrap();
    fs::write(ws.join("packages/repo1/unfinished.txt"), "keep uncommitted").unwrap();
    let p = f.rebuild(&[&ws]);
    let row = &p["workspaces"][0];
    assert_eq!(row["classification"], "skipped");
    assert_eq!(row["repositories"].as_array().unwrap().len(), 2);
    assert_eq!(row["repositories"][1]["dirty"]["untracked"], 1);
    assert!(!row["repositories"][0]["blockers"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(p["summary"]["skipped"], 1);
    assert_eq!(p["summary"]["rebuildable"], 0);
}

#[test]
fn rebuild_batch_reports_invalid_workspace_without_losing_other_results() {
    let f = Fixture::new();
    let ws = f.workspace("rebuild-good", 1, false);
    let invalid = f.root.join("invalid-workspace");
    let p = f.rebuild(&[&invalid, &ws]);
    assert_eq!(p["summary"]["blocked"], 1);
    assert_eq!(p["summary"]["skipped"], 1);
    assert_eq!(p["workspaces"].as_array().unwrap().len(), 2);
    assert!(!invalid.exists());
}

#[test]
fn rebuild_gitlink_is_reported_and_never_silently_removed_or_ignored() {
    let f = Fixture::new();
    let ws = f.workspace("rebuild-gitlink", 1, true);
    let repo = ws.join("packages/repo0");
    let head = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    git(
        &repo,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},orphan"),
        ],
    );
    git(
        &repo,
        &[
            "-c",
            "user.name=VDW Test",
            "-c",
            "user.email=vdw@example.com",
            "commit",
            "--quiet",
            "-m",
            "add gitlink",
        ],
    );
    fs::create_dir(repo.join("orphan")).unwrap();
    let p = f.rebuild(&[&ws]);
    let row = &p["workspaces"][0];
    assert_eq!(row["classification"], "skipped", "{p}");
    assert_eq!(
        row["repositories"][0]["gitlinks"].as_array().unwrap().len(),
        1
    );
    assert!(row["repositories"][0]["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["code"] == "gitlink_requires_review"));
    assert!(repo.join("orphan").exists());
}
fn git(path: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}
fn summary_is_complete(op: &Value) {
    let s = &op["summary"];
    assert_eq!(
        s["scope_total"].as_u64().unwrap(),
        s["archived_count"].as_u64().unwrap() + s["not_archived_count"].as_u64().unwrap()
    );
    assert_eq!(
        s["not_archived_count"].as_u64().unwrap(),
        s["skipped_count"].as_u64().unwrap()
            + s["failed_count"].as_u64().unwrap()
            + s["unfinished_count"].as_u64().unwrap()
    );
    assert_eq!(
        s["scope_total"].as_u64().unwrap() as usize,
        op["results"].as_array().unwrap().len()
    );
}

#[test]
fn leading_dot_dependencies_round_trip_without_archiving_managed_files_as_materials() {
    for prefix in ["./packages", "./references"] {
        for editable in [true, false] {
            let f = Fixture::new();
            let w = f.workspace_with_dependency_prefix("dotted", 1, editable, prefix);
            let original_config = fs::read(w.join(".vdwrc")).unwrap();
            let dependency = format!("{}/repo0", prefix.trim_start_matches("./"));
            let head = if editable {
                Some(git(&w.join(&dependency), &["rev-parse", "HEAD"]))
            } else {
                None
            };
            let plan = f.archive_plan(&w);
            assert_eq!(plan["entries"][0]["reason_codes"], json!([]), "{plan}");
            assert!(w.join(&dependency).join("version.txt").is_file());
            let materials = plan["entries"][0]["materials"].as_array().unwrap();
            assert!(materials.iter().any(|m| m["path"] == "notes/decisions.md"));
            assert!(materials.iter().all(|m| {
                let path = m["path"].as_str().unwrap();
                path != dependency && !path.starts_with(&format!("{dependency}/"))
            }));

            let operation = f.execute(&plan);
            assert_eq!(operation["summary"]["archived_count"], 1, "{operation}");
            assert!(!w.join(&dependency).join("version.txt").exists());
            if editable {
                assert!(!w.join(&dependency).exists());
            }
            let archive = &operation["results"][0];
            let snapshot = Path::new(archive["archive_path"].as_str().unwrap());
            assert!(!snapshot.join("workspace-files").join(&dependency).exists());
            assert_eq!(fs::read(w.join(".vdwrc")).unwrap(), original_config);

            let restore = f.json(&[
                "workspace",
                "restore",
                archive["archive_id"].as_str().unwrap(),
                "--preview",
                "--json",
            ]);
            let restored = f.execute(&restore);
            assert_eq!(restored["summary"]["restored_count"], 1, "{restored}");
            assert_eq!(fs::read(w.join(".vdwrc")).unwrap(), original_config);
            assert_eq!(
                fs::read_to_string(w.join(&dependency).join("version.txt")).unwrap(),
                "dotted-0\n"
            );
            if let Some(head) = head {
                assert_eq!(git(&w.join(&dependency), &["rev-parse", "HEAD"]), head);
            }
        }
    }
}

#[test]
fn leading_dot_dependencies_still_skip_the_whole_dirty_workspace() {
    let f = Fixture::new();
    let w = f.workspace_with_dependency_prefix("dotted-dirty", 2, true, "./packages");
    fs::write(w.join("packages/repo1/version.txt"), "unfinished\n").unwrap();
    let plan = f.archive_plan(&w);
    assert_eq!(
        plan["entries"][0]["reason_codes"],
        json!(["dirty"]),
        "{plan}"
    );
    let operation = f.execute(&plan);
    assert_eq!(operation["summary"]["archived_count"], 0);
    assert_eq!(operation["summary"]["skipped_count"], 1);
    assert!(w.join("packages/repo0/.git").is_file());
    assert!(w.join("packages/repo1/.git").is_file());
    assert_eq!(
        fs::read_to_string(w.join("packages/repo1/version.txt")).unwrap(),
        "unfinished\n"
    );
    summary_is_complete(&operation);
}

#[test]
#[cfg(unix)]
fn leading_dot_dependencies_do_not_allow_symlink_escapes() {
    let f = Fixture::new();
    let w = f.workspace_with_dependency_prefix("dotted-symlink", 1, true, "./packages");
    let outside = f.root.join("outside-workspace");
    fs::rename(w.join("packages"), &outside).unwrap();
    std::os::unix::fs::symlink(&outside, w.join("packages")).unwrap();
    let plan = f.archive_plan(&w);
    assert_eq!(
        plan["entries"][0]["reason_codes"],
        json!(["inspection_failed"])
    );
    assert!(plan["entries"][0]["message"]
        .as_str()
        .unwrap()
        .contains("Symlink boundary"));
    assert!(outside.join("repo0/version.txt").is_file());
}

#[test]
fn batch_reports_success_dirty_and_protection_without_losing_any_workspace() {
    let f = Fixture::new();
    let clean = f.workspace("clean", 1, true);
    let dirty = f.workspace("dirty", 2, true);
    let protected = f.workspace("protected", 1, true);
    fs::write(dirty.join("packages/repo1/version.txt"), "unfinished\n").unwrap();
    f.ok(&[
        "workspace",
        "protect",
        "add",
        protected.to_str().unwrap(),
        "--reason",
        "long-lived",
    ]);
    let p = f.json(&[
        "workspace",
        "archive",
        clean.to_str().unwrap(),
        dirty.to_str().unwrap(),
        protected.to_str().unwrap(),
        "--preview",
        "--json",
    ]);
    assert_eq!(p["entries"].as_array().unwrap().len(), 3);
    let op = f.execute(&p);
    summary_is_complete(&op);
    assert_eq!(op["summary"]["archived_count"], 1);
    assert_eq!(op["summary"]["skipped_count"], 2);
    assert!(!clean.join("packages/repo0").exists());
    assert!(dirty.join("packages/repo0/.git").exists());
    assert!(dirty.join("packages/repo1/.git").exists());
    assert!(protected.join("packages/repo0/.git").exists());
    assert!(op["results"][1]["reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("dirty")));
    assert_eq!(op["results"][1]["repositories"][1]["dirty"]["unstaged"], 1);
    let again = f.execute(&p);
    assert_eq!(again["operation_id"], op["operation_id"]);
    let report = f.json(&[
        "lifecycle",
        "operation",
        "status",
        op["operation_id"].as_str().unwrap(),
        "--json",
    ]);
    assert_eq!(report, op);
    let human = f.ok(&[
        "lifecycle",
        "operation",
        "status",
        op["operation_id"].as_str().unwrap(),
    ]);
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains("已归档 1"));
    assert!(text.contains("未归档 2"));
    assert!(text.contains("跳过 2"));
}

#[test]
fn dirty_created_after_preview_skips_all_repositories() {
    let f = Fixture::new();
    let w = f.workspace("late-dirty", 2, true);
    let p = f.archive_plan(&w);
    fs::write(w.join("packages/repo1/untracked.txt"), "preserve me").unwrap();
    let op = f.execute(&p);
    assert_eq!(op["summary"]["skipped_count"], 1);
    assert_eq!(op["results"][0]["reason_codes"][0], "dirty");
    assert!(w.join("packages/repo0/.git").exists());
    assert!(w.join("packages/repo1/untracked.txt").exists());
    summary_is_complete(&op);
}

#[test]
fn staged_changes_are_protected_and_unknown_ignored_files_are_not_discarded() {
    let f = Fixture::new();
    let w = f.workspace("staged", 1, true);
    let repo = w.join("packages/repo0");
    fs::write(repo.join("version.txt"), "staged").unwrap();
    git(&repo, &["add", "version.txt"]);
    let op = f.archive(&w);
    assert_eq!(op["summary"]["skipped_count"], 1);
    assert_eq!(op["results"][0]["repositories"][0]["dirty"]["staged"], 1);
    git(&repo, &["reset", "--hard", "HEAD"]);
    fs::write(repo.join(".env"), "LOCAL_SECRET=not-for-deletion").unwrap();
    let op = f.archive(&w);
    assert_eq!(
        op["results"][0]["reason_codes"][0],
        "ignored_files_need_review"
    );
    assert!(repo.join(".env").exists());
}

#[test]
fn archive_preserves_materials_branch_and_actual_head_and_restores_exactly() {
    let f = Fixture::new();
    let w = f.workspace("restore", 1, true);
    let repo = w.join("packages/repo0");
    let lock: Value = serde_yaml::from_slice(&fs::read(w.join(".vdw.lock")).unwrap()).unwrap();
    let locked = lock["dependencies"][0]["resolved_commit"].as_str().unwrap();
    fs::write(repo.join("version.txt"), "committed ahead of lock").unwrap();
    git(&repo, &["add", "version.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=VDW Test",
            "-c",
            "user.email=vdw@example.com",
            "commit",
            "--quiet",
            "-m",
            "local only",
        ],
    );
    let head = git(&repo, &["rev-parse", "HEAD"]);
    assert_ne!(head, locked);
    let op = f.archive(&w);
    assert_eq!(op["summary"]["archived_count"], 1);
    let row = &op["results"][0];
    let aid = row["archive_id"].as_str().unwrap();
    let loc = Path::new(row["archive_path"].as_str().unwrap());
    assert!(w.join(".vdwrc").exists());
    assert!(w.join("notes/decisions.md").exists());
    assert!(loc.join("workspace-files/notes/empty").is_dir());
    assert!(loc.join("workspace-files/.vscode/settings.json").exists());
    assert!(!loc.join("workspace-files/packages").exists());
    assert!(loc.join("memory/catalog.md").exists());
    let m: Value = serde_json::from_slice(&fs::read(loc.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(m["repositories"][0]["head"], head);
    assert_eq!(m["repositories"][0]["locked_commit"], locked);
    let bare = Path::new(m["repositories"][0]["bare"].as_str().unwrap());
    assert_eq!(git(bare, &["rev-parse", "refs/heads/main"]), head);
    let status = Command::new(env!("CARGO_BIN_EXE_vdw"))
        .args(["status", "--json"])
        .env("HOME", &f.home)
        .current_dir(&w)
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["lifecycle_state"],
        "archived"
    );
    let p = f.json(&["workspace", "restore", aid, "--preview", "--json"]);
    let restored = f.execute(&p);
    assert_eq!(restored["results"][0]["outcome"], "restored");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
}

#[test]
fn archive_root_change_migrates_then_restores_from_new_location() {
    let f = Fixture::new();
    let w = f.workspace("move", 1, true);
    let op = f.archive(&w);
    let row = &op["results"][0];
    let old = PathBuf::from(row["archive_path"].as_str().unwrap());
    let aid = row["archive_id"].as_str().unwrap();
    let dest = f.root.join("new-archive-volume");
    let p = f.json(&[
        "config",
        "set",
        "archive-root",
        dest.to_str().unwrap(),
        "--preview",
        "--json",
    ]);
    assert_eq!(p["value"]["moves"].as_array().unwrap().len(), 1);
    let migration = f.execute(&p);
    assert_eq!(migration["kind"], "migration");
    assert_eq!(migration["state"], "completed");
    assert_eq!(migration["progress"]["archives_done"], 1);
    assert!(!old.exists());
    let rec = f.json(&["workspace", "memory", "list", aid, "--json"]);
    let location = Path::new(rec["location"].as_str().unwrap());
    assert!(location.starts_with(&dest));
    assert!(location.join("manifest.json").exists());
    let p = f.json(&["workspace", "restore", aid, "--preview", "--json"]);
    f.execute(&p);
    assert!(w.join("packages/repo0/.git").is_file());
}

#[test]
fn added_protection_and_activity_invalidate_old_permission() {
    let f = Fixture::new();
    let w = f.workspace("race", 1, true);
    let p = f.archive_plan(&w);
    f.ok(&["workspace", "protect", "add", w.to_str().unwrap()]);
    let op = f.execute(&p);
    assert_eq!(op["results"][0]["reason_codes"][0], "protected");
    let remove = f.json(&[
        "workspace",
        "protect",
        "remove",
        w.to_str().unwrap(),
        "--preview",
        "--json",
    ]);
    f.execute(&remove);
    let p = f.archive_plan(&w);
    f.ok(&["workspace", "touch", w.to_str().unwrap()]);
    let output = f.run(&[
        "lifecycle",
        "plan",
        "execute",
        p["plan_id"].as_str().unwrap(),
        "--confirm",
        "--json",
    ]);
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["failed_count"], 1);
    assert!(w.join("packages/repo0/.git").exists());
}

#[test]
fn policy_automation_and_legacy_observation_are_distinct_from_last_seen() {
    let f = Fixture::new();
    let old = f.workspace("old", 1, true);
    let fresh = f.workspace("fresh", 1, true);
    let p = f.json(&[
        "lifecycle",
        "auto",
        "enable",
        "--preview",
        "--no-scheduler",
        "--json",
    ]);
    f.execute(&p);
    let path = f.home.join(".vdw/workspaces.json");
    let mut registry = f.registry();
    for ws in registry["workspaces"].as_array_mut().unwrap() {
        if ws["name"] == "old" {
            let then = (chrono::Utc::now() - chrono::Duration::days(40)).to_rfc3339();
            ws["last_active_at"] = json!(then);
            ws["observation_started_at"] = json!(then);
            ws["candidate_since"] =
                json!((chrono::Utc::now() - chrono::Duration::hours(73)).to_rfc3339());
        }
    }
    fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
    let op = f.json(&["lifecycle", "run", "--json"]);
    summary_is_complete(&op);
    assert_eq!(op["summary"]["archived_count"], 1);
    assert_eq!(op["summary"]["skipped_count"], 1);
    assert!(!old.join("packages/repo0").exists());
    assert!(fresh.join("packages/repo0/.git").exists());
    let legacy = f.workspace("legacy", 0, false);
    fs::write(
        &path,
        serde_json::to_vec(
            &json!({"version":1,"workspaces":[{"path":legacy,"last_seen":"2000-01-01T00:00:00Z"}]}),
        )
        .unwrap(),
    )
    .unwrap();
    let scan = f.json(&["lifecycle", "scan", "--preview", "--json"]);
    assert!(scan["entries"][0]["reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("not_due")));
    assert_eq!(f.registry()["version"], 2);
}

#[test]
fn archived_references_survive_missing_skeleton_and_corruption_blocks_restore() {
    let f = Fixture::new();
    let w = f.workspace("refs", 1, true);
    let op = f.archive(&w);
    let aid = op["results"][0]["archive_id"].as_str().unwrap();
    let loc = PathBuf::from(op["results"][0]["archive_path"].as_str().unwrap());
    fs::remove_dir_all(&w).unwrap();
    let status = f.json(&["store", "status", "--json"]);
    assert_eq!(status["reference_inventory_complete"], true);
    assert!(!status["objects"][0]["archive_references"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(status["objects"][0]["gc_candidate"], false);
    fs::write(loc.join("workspace-files/notes/decisions.md"), "corrupted").unwrap();
    let status = f.json(&["store", "status", "--json"]);
    assert_eq!(status["reference_inventory_complete"], false);
    assert_eq!(status["objects"][0]["gc_candidate"], false);
    let o = f.run(&["workspace", "restore", aid, "--preview", "--json"]);
    assert!(!o.status.success());
    assert!(!w.exists());
}

#[test]
fn restore_does_not_reset_an_advanced_branch_or_overwrite_new_notes() {
    let f = Fixture::new();
    let w = f.workspace("advanced", 1, true);
    let op = f.archive(&w);
    let row = &op["results"][0];
    let aid = row["archive_id"].as_str().unwrap();
    let loc = Path::new(row["archive_path"].as_str().unwrap());
    let m: Value = serde_json::from_slice(&fs::read(loc.join("manifest.json")).unwrap()).unwrap();
    let bare = Path::new(m["repositories"][0]["bare"].as_str().unwrap());
    fs::write(w.join("notes/decisions.md"), "new note after archive").unwrap();
    let o = f.run(&["workspace", "restore", aid, "--preview", "--json"]);
    assert!(!o.status.success());
    assert_eq!(
        fs::read_to_string(w.join("notes/decisions.md")).unwrap(),
        "new note after archive"
    );
    let target = f.root.join("restored-at-new-path");
    let p = f.json(&[
        "workspace",
        "restore",
        aid,
        "--target",
        target.to_str().unwrap(),
        "--preview",
        "--json",
    ]);
    f.execute(&p);
    assert!(target.join("packages/repo0/.git").exists());
    assert!(git(bare, &["show-ref", "--heads"]).contains("refs/heads/main"));
}

#[test]
fn portable_bundle_measurement_is_read_only_and_can_restore_without_store() {
    let f = Fixture::new();
    let w = f.workspace("portable", 1, true);
    let before = git(&w.join("packages/repo0"), &["show-ref"]);
    let p = f.json(&[
        "workspace",
        "archive",
        w.to_str().unwrap(),
        "--preview",
        "--measure-bundle",
        "--git-backup",
        "portable_bundle",
        "--json",
    ]);
    assert!(
        p["entries"][0]["repositories"][0]["bundle_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(git(&w.join("packages/repo0"), &["show-ref"]), before);
    let op = f.execute(&p);
    let aid = op["results"][0]["archive_id"].as_str().unwrap();
    let loc = PathBuf::from(op["results"][0]["archive_path"].as_str().unwrap());
    let m: Value = serde_json::from_slice(&fs::read(loc.join("manifest.json")).unwrap()).unwrap();
    let bare = PathBuf::from(m["repositories"][0]["bare"].as_str().unwrap());
    fs::remove_dir_all(&bare).unwrap();
    let p = f.json(&["workspace", "restore", aid, "--preview", "--json"]);
    f.execute(&p);
    assert!(w.join("packages/repo0/.git").exists());
    assert_eq!(
        git(&w.join("packages/repo0"), &["rev-parse", "HEAD"]),
        m["repositories"][0]["head"].as_str().unwrap()
    );
}

#[test]
fn ai_is_optional_and_summary_uses_only_preserved_materials_with_source_checks() {
    let f = Fixture::new();
    let w = f.workspace("memory", 1, true);
    let op = f.archive(&w);
    let aid = op["results"][0]["archive_id"].as_str().unwrap();
    let location = Path::new(op["results"][0]["archive_path"].as_str().unwrap());
    assert_eq!(op["results"][0]["memory_state"], "disabled");
    assert!(location.join("memory/sources.jsonl").exists());
    let script = f.root.join("summary.py");
    fs::write(&script,"import argparse,json\np=argparse.ArgumentParser();p.add_argument('--input');p.add_argument('--output');a=p.parse_args();r=json.load(open(a.input));s=r['sources'];assert all(not x['path'].startswith('packages/') for x in s);refs=[{'path':x['path'],'hash':x['hash']} for x in s];json.dump({'summary':'Project summary','evidence_refs':refs,'improvements':[{'observation':'recorded task','recommendation':'verify online','evidence_refs':refs}]},open(a.output,'w'))\n").unwrap();
    let mut cfg = serde_json::to_value(vdw::lifecycle::LifecycleConfig::default()).unwrap();
    cfg["memory"]["ai"]["enabled"] = json!(true);
    cfg["memory"]["ai"]["provider"] = json!("command");
    cfg["memory"]["ai"]["command"] = json!(["python3", script]);
    f.configure(cfg);
    let p = f.json(&[
        "workspace",
        "memory",
        "summarize",
        aid,
        "--preview",
        "--json",
    ]);
    let result = f.execute(&p);
    assert_eq!(result["state"], "completed");
    let record = f.json(&["workspace", "memory", "list", aid, "--json"]);
    assert_eq!(record["memory_state"], "completed");
    let generation = record["generations"][0].as_str().unwrap();
    assert!(location
        .join("derived/ai")
        .join(generation)
        .join("COMPLETE")
        .exists());
}

#[test]
fn read_only_projection_changes_skip_the_workspace_and_clean_projection_restores() {
    let f = Fixture::new();
    let w = f.workspace("read-only", 1, false);
    let file = w.join("packages/repo0/version.txt");
    // An editor's atomic save detaches the projection from its Store inode.
    fs::remove_file(&file).unwrap();
    fs::write(&file, "local edit").unwrap();
    let op = f.archive(&w);
    assert_eq!(op["results"][0]["outcome"], "skipped");
    assert!(op["results"][0]["reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("dirty")));
    assert!(file.exists());
    let state = vdw::types::workspace::WorkspaceState::load(&w).unwrap();
    let source = &state
        .links
        .iter()
        .find(|l| l.target == file)
        .unwrap()
        .source;
    fs::remove_file(&file).unwrap();
    fs::hard_link(source, &file).unwrap();
    let op = f.archive(&w);
    assert_eq!(op["summary"]["archived_count"], 1);
    let p = f.json(&[
        "workspace",
        "restore",
        op["results"][0]["archive_id"].as_str().unwrap(),
        "--preview",
        "--json",
    ]);
    f.execute(&p);
    assert!(file.exists());
}

#[test]
fn unknown_selection_does_not_hide_valid_workspaces_or_the_failure_report() {
    let f = Fixture::new();
    let w = f.workspace("known", 1, true);
    let p = f.json(&[
        "workspace",
        "archive",
        w.to_str().unwrap(),
        "missing-workspace",
        "--preview",
        "--json",
    ]);
    let result = f.run(&[
        "lifecycle",
        "plan",
        "execute",
        p["plan_id"].as_str().unwrap(),
        "--confirm",
        "--json",
    ]);
    assert_eq!(result.status.code(), Some(2));
    let op: Value = serde_json::from_slice(&result.stdout).unwrap();
    summary_is_complete(&op);
    assert_eq!(op["summary"]["archived_count"], 1);
    assert_eq!(op["summary"]["failed_count"], 1);
    assert_eq!(op["results"][1]["name"], "missing-workspace");
}

#[test]
fn leases_and_known_disposable_ignored_directories_have_different_effects() {
    let f = Fixture::new();
    let w = f.workspace("lease", 1, true);
    let repo = w.join("packages/repo0");
    fs::create_dir(repo.join("node_modules")).unwrap();
    fs::write(repo.join("node_modules/generated.js"), "cache").unwrap();
    f.ok(&[
        "workspace",
        "lease",
        w.to_str().unwrap(),
        "--client",
        "test-ide",
    ]);
    let op = f.archive(&w);
    assert!(op["results"][0]["reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("active_lease")));
    assert!(repo.exists());
    f.ok(&[
        "workspace",
        "lease",
        w.to_str().unwrap(),
        "--client",
        "test-ide",
        "--release",
    ]);
    let op = f.archive(&w);
    assert_eq!(op["summary"]["archived_count"], 1);
    assert!(!repo.exists());
}

#[test]
fn restore_resume_accepts_only_unchanged_worktrees_and_keeps_attempt_history() {
    let f = Fixture::new();
    let w = f.workspace("resume", 2, true);
    let archived = f.archive(&w);
    let aid = archived["results"][0]["archive_id"].as_str().unwrap();
    let plan = f.json(&["workspace", "restore", aid, "--preview", "--json"]);
    let restored = f.execute(&plan);
    let oid = restored["operation_id"].as_str().unwrap();
    // Model a crash after restoring repo0: disk and journal both contain partial progress.
    let bare = git(
        &w.join("packages/repo1"),
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    git(
        Path::new(&bare),
        &[
            "worktree",
            "remove",
            w.join("packages/repo1").to_str().unwrap(),
        ],
    );
    let registry_path = f.home.join(".vdw/workspaces.json");
    let mut registry = f.registry();
    registry["workspaces"][0]["lifecycle_state"] = json!("restoring");
    registry["workspaces"][0]["current_operation_id"] = json!(oid);
    fs::write(&registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
    let mut partial = restored.clone();
    partial["state"] = json!("failed");
    partial["results"][0]["outcome"] = json!("failed");
    partial["results"][0]["completed_steps"] = json!(["restored:packages/repo0"]);
    fs::write(
        f.home.join(format!(".vdw/lifecycle/operations/{oid}.json")),
        serde_json::to_vec(&partial).unwrap(),
    )
    .unwrap();
    let resume = f.json(&[
        "lifecycle",
        "operation",
        "resume",
        oid,
        "--preview",
        "--json",
    ]);
    let op = f.execute(&resume);
    assert_eq!(op["results"][0]["outcome"], "restored");
    assert_eq!(op["attempts"], 2);
    assert_eq!(op["attempt_history"].as_array().unwrap().len(), 1);
    assert!(w.join("packages/repo0/.git").exists());
    assert!(w.join("packages/repo1/.git").exists());
}

#[test]
fn migration_continues_from_verified_partial_copy_and_preserves_symlink_metadata() {
    let f = Fixture::new();
    let w = f.workspace("resume-move", 1, true);
    #[cfg(unix)]
    std::os::unix::fs::symlink("decisions.md", w.join("notes/latest")).unwrap();
    let archived = f.archive(&w);
    let old = PathBuf::from(archived["results"][0]["archive_path"].as_str().unwrap());
    let aid = archived["results"][0]["archive_id"].as_str().unwrap();
    let root = f.root.join("second-location");
    let p = f.json(&[
        "config",
        "set",
        "archive-root",
        root.to_str().unwrap(),
        "--preview",
        "--json",
    ]);
    let completed = f.execute(&p);
    let oid = completed["operation_id"].as_str().unwrap();
    let index_path = f.home.join(".vdw/lifecycle/archives.json");
    let mut records: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    let new = PathBuf::from(records[0]["location"].as_str().unwrap());
    // Model an interrupted migration with an intact source and a verified partial staging file.
    fs::create_dir_all(old.parent().unwrap()).unwrap();
    fs::rename(&new, &old).unwrap();
    records[0]["location"] = json!(old);
    fs::write(&index_path, serde_json::to_vec(&records).unwrap()).unwrap();
    let stage = new.with_file_name(format!(".{aid}.migration-{oid}"));
    fs::create_dir_all(stage.join("workspace-files/notes")).unwrap();
    fs::copy(
        old.join("workspace-files/notes/decisions.md"),
        stage.join("workspace-files/notes/decisions.md"),
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("decisions.md", stage.join("workspace-files/notes/latest")).unwrap();
    let mut partial = completed.clone();
    partial["state"] = json!("paused");
    partial["completed_archive_ids"] = json!([]);
    partial["progress"]["phase"] = json!("copying");
    fs::write(
        f.home.join(format!(".vdw/lifecycle/operations/{oid}.json")),
        serde_json::to_vec(&partial).unwrap(),
    )
    .unwrap();
    let op = f.json(&["lifecycle", "operation", "continue", oid, "--json"]);
    assert_eq!(op["state"], "completed");
    assert_eq!(
        op["progress"]["bytes_copied"],
        op["progress"]["bytes_total"]
    );
    assert_eq!(op["progress"]["files_done"], op["progress"]["files_total"]);
    assert!(!old.exists());
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(new.join("workspace-files/notes/latest")).unwrap(),
        PathBuf::from("decisions.md")
    );
}

#[test]
fn background_migration_reports_running_and_can_be_observed_without_worker_lock() {
    let f = Fixture::new();
    let w = f.workspace("background", 1, true);
    f.archive(&w);
    let root = f.root.join("background-location");
    let p = f.json(&[
        "config",
        "set",
        "archive-root",
        root.to_str().unwrap(),
        "--preview",
        "--json",
    ]);
    let op = f.json(&[
        "lifecycle",
        "plan",
        "execute",
        p["plan_id"].as_str().unwrap(),
        "--confirm",
        "--background",
        "--json",
    ]);
    assert_eq!(op["state"], "running");
    let start = std::time::Instant::now();
    loop {
        let current = f.json(&[
            "lifecycle",
            "operation",
            "status",
            op["operation_id"].as_str().unwrap(),
            "--json",
        ]);
        if current["state"] != "running" {
            assert_eq!(current["state"], "completed", "{current}");
            assert_eq!(current["progress"]["archives_done"], 1);
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(15));
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn portable_readonly_restore_rebuilds_store_files_and_runtime_identity() {
    let f = Fixture::new();
    let w = f.workspace("portable-ro", 1, false);
    let p = f.json(&[
        "workspace",
        "archive",
        w.to_str().unwrap(),
        "--preview",
        "--git-backup",
        "portable_bundle",
        "--json",
    ]);
    let op = f.execute(&p);
    let aid = op["results"][0]["archive_id"].as_str().unwrap();
    let loc = Path::new(op["results"][0]["archive_path"].as_str().unwrap());
    let m: Value = serde_json::from_slice(&fs::read(loc.join("manifest.json")).unwrap()).unwrap();
    let object = Path::new(m["repositories"][0]["bare"].as_str().unwrap())
        .parent()
        .unwrap();
    fs::remove_dir_all(object).unwrap();
    let p = f.json(&["workspace", "restore", aid, "--preview", "--json"]);
    f.execute(&p);
    let status = f.json(&["store", "status", "--json"]);
    assert_eq!(status["reference_inventory_complete"], true);
    assert_eq!(status["objects"][0]["state"], "referenced");
    let state = vdw::types::workspace::WorkspaceState::load(&w).unwrap();
    assert!(state
        .links
        .iter()
        .all(|l| l.source.exists() && l.target.exists()));
    assert_eq!(
        fs::read_to_string(w.join("packages/repo0/version.txt")).unwrap(),
        "portable-ro-0\n"
    );
}

#[test]
fn native_advanced_branch_is_preserved_when_explicit_restore_branch_is_chosen() {
    let f = Fixture::new();
    let w = f.workspace("new-branch", 1, true);
    let op = f.archive(&w);
    let row = &op["results"][0];
    let aid = row["archive_id"].as_str().unwrap();
    let location = Path::new(row["archive_path"].as_str().unwrap());
    let m: Value =
        serde_json::from_slice(&fs::read(location.join("manifest.json")).unwrap()).unwrap();
    let bare = Path::new(m["repositories"][0]["bare"].as_str().unwrap());
    let other = f.root.join("other-worktree");
    git(bare, &["worktree", "add", other.to_str().unwrap(), "main"]);
    fs::write(other.join("version.txt"), "branch moved elsewhere").unwrap();
    git(&other, &["add", "version.txt"]);
    git(
        &other,
        &[
            "-c",
            "user.name=VDW Test",
            "-c",
            "user.email=vdw@example.com",
            "commit",
            "--quiet",
            "-m",
            "later",
        ],
    );
    let advanced = git(&other, &["rev-parse", "HEAD"]);
    assert!(!f
        .run(&["workspace", "restore", aid, "--preview", "--json"])
        .status
        .success());
    let p = f.json(&[
        "workspace",
        "restore",
        aid,
        "--branch",
        "restored-snapshot",
        "--preview",
        "--json",
    ]);
    f.execute(&p);
    assert_eq!(git(bare, &["rev-parse", "refs/heads/main"]), advanced);
    assert_eq!(
        git(&w.join("packages/repo0"), &["branch", "--show-current"]),
        "restored-snapshot"
    );
    let lock = vdw::lockfile::WorkspaceLock::load_required(&w).unwrap();
    assert_eq!(lock.dependencies[0].revision.value, "restored-snapshot");
}

#[test]
fn archive_publish_checkpoint_can_be_recovered_without_an_index_record() {
    let f = Fixture::new();
    let w = f.workspace("publish-crash", 1, true);
    let op = f.archive(&w);
    let oid = op["operation_id"].as_str().unwrap();
    let aid = op["results"][0]["archive_id"].as_str().unwrap();
    let loc = Path::new(op["results"][0]["archive_path"].as_str().unwrap());
    let m: Value = serde_json::from_slice(&fs::read(loc.join("manifest.json")).unwrap()).unwrap();
    let bare = Path::new(m["repositories"][0]["bare"].as_str().unwrap());
    git(
        bare,
        &[
            "worktree",
            "add",
            w.join("packages/repo0").to_str().unwrap(),
            "main",
        ],
    );
    let mut reg = f.registry();
    reg["workspaces"][0]["lifecycle_state"] = json!("active");
    reg["workspaces"][0]["latest_archive_id"] = Value::Null;
    fs::write(
        f.home.join(".vdw/workspaces.json"),
        serde_json::to_vec(&reg).unwrap(),
    )
    .unwrap();
    fs::write(f.home.join(".vdw/lifecycle/archives.json"), b"[]").unwrap();
    let mut partial = op.clone();
    partial["state"] = json!("failed");
    partial["results"][0]["outcome"] = json!("failed");
    partial["results"][0]["completed_steps"] = json!([]);
    fs::write(
        f.home.join(format!(".vdw/lifecycle/operations/{oid}.json")),
        serde_json::to_vec(&partial).unwrap(),
    )
    .unwrap();
    let p = f.json(&[
        "lifecycle",
        "operation",
        "resume",
        oid,
        "--preview",
        "--json",
    ]);
    let recovered = f.execute(&p);
    assert_eq!(recovered["summary"]["archived_count"], 1);
    assert_eq!(recovered["results"][0]["archive_id"], aid);
    assert!(!w.join("packages/repo0").exists());
}

#[test]
fn ide_preview_keeps_its_lease_and_confirmation_cannot_override_another_client() {
    let f = Fixture::new();
    let w = f.workspace("ide-lease", 1, true);
    f.ok(&[
        "workspace",
        "lease",
        w.to_str().unwrap(),
        "--client",
        "ide-a",
    ]);
    let p = f.json(&[
        "workspace",
        "archive",
        w.to_str().unwrap(),
        "--client",
        "ide-a",
        "--preview",
        "--json",
    ]);
    assert!(!p["entries"][0]["reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("active_lease")));
    let ordinary = f.archive_plan(&w);
    assert!(ordinary["entries"][0]["reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("active_lease")));
    f.ok(&[
        "workspace",
        "lease",
        w.to_str().unwrap(),
        "--client",
        "ide-b",
    ]);
    let result = f.execute(&p);
    assert_eq!(result["results"][0]["reason_codes"][0], "active_lease");
    assert!(w.join("packages/repo0/.git").exists());
    f.ok(&[
        "workspace",
        "lease",
        w.to_str().unwrap(),
        "--client",
        "ide-b",
        "--release",
    ]);
    let p = f.json(&[
        "workspace",
        "archive",
        w.to_str().unwrap(),
        "--client",
        "ide-a",
        "--preview",
        "--json",
    ]);
    assert_eq!(f.execute(&p)["summary"]["archived_count"], 1);
}

#[test]
fn workspace_policy_overrides_explain_effective_values_and_temporary_retention() {
    let f = Fixture::new();
    let w = f.workspace("policy", 1, true);
    let p = f.json(&[
        "config",
        "set",
        "idle-days",
        "60",
        "--workspace",
        w.to_str().unwrap(),
        "--preview",
        "--json",
    ]);
    f.execute(&p);
    let effective = f.json(&[
        "config",
        "show",
        "--effective",
        "--workspace",
        w.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(effective["effective"]["idle_days"]["value"], 60);
    assert_eq!(
        effective["effective"]["idle_days"]["source"],
        "workspace_override"
    );
    let p = f.json(&[
        "config",
        "set",
        "workspace-auto",
        w.to_str().unwrap(),
        "false",
        "--preview",
        "--json",
    ]);
    f.execute(&p);
    let p = f.json(&[
        "config",
        "set",
        "retain-until",
        "2099-01-01T00:00:00+08:00",
        "--workspace",
        w.to_str().unwrap(),
        "--preview",
        "--json",
    ]);
    f.execute(&p);
    let result = f.archive(&w);
    assert_eq!(result["summary"]["skipped_count"], 1);
    assert!(result["results"][0]["reason_codes"]
        .as_array()
        .unwrap()
        .contains(&json!("retain_until")));
    let p = f.json(&[
        "config",
        "set",
        "retain-until",
        "none",
        "--workspace",
        w.to_str().unwrap(),
        "--preview",
        "--json",
    ]);
    f.execute(&p);
    assert_eq!(f.archive(&w)["summary"]["archived_count"], 1);
}

#[test]
fn merge_conflicts_are_reported_and_the_repository_is_left_untouched() {
    let f = Fixture::new();
    let w = f.workspace("conflict", 1, true);
    let repo = w.join("packages/repo0");
    git(&repo, &["checkout", "-b", "conflict-side"]);
    fs::write(repo.join("version.txt"), "side\n").unwrap();
    git(&repo, &["add", "version.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=VDW Test",
            "-c",
            "user.email=vdw@example.com",
            "commit",
            "--quiet",
            "-m",
            "side",
        ],
    );
    git(&repo, &["checkout", "main"]);
    fs::write(repo.join("version.txt"), "main\n").unwrap();
    git(&repo, &["add", "version.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=VDW Test",
            "-c",
            "user.email=vdw@example.com",
            "commit",
            "--quiet",
            "-m",
            "main",
        ],
    );
    let merge = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args([
            "-c",
            "user.name=VDW Test",
            "-c",
            "user.email=vdw@example.com",
            "merge",
            "--no-commit",
            "conflict-side",
        ])
        .output()
        .unwrap();
    assert!(!merge.status.success());
    let before = git(&repo, &["ls-files", "--unmerged"]);
    let result = f.archive(&w);
    assert_eq!(result["summary"]["skipped_count"], 1);
    assert_eq!(
        result["results"][0]["repositories"][0]["dirty"]["conflicts"],
        1
    );
    assert_eq!(git(&repo, &["ls-files", "--unmerged"]), before);
}

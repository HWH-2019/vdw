#!/usr/bin/env python3
"""Run the shipped binary outside the checkout with an isolated HOME and no Node/npm.

Only Python's standard library is required. All writes and lifecycle operations use
temporary fixture workspaces, never the user's VDW registry or repositories.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request


def run(binary, hold=False, use_source_binary=False, test_system_trash=False):
    root = Path(tempfile.mkdtemp(prefix="vdw-dashboard-smoke-"))
    tools = root / "bin"
    tools.mkdir()
    windows = os.name == "nt"
    git = shutil.which("git")
    if not git:
        raise RuntimeError("Git is required for the isolated lifecycle test fixtures")
    copied = tools / ("vdw.exe" if windows else "vdw")
    if use_source_binary:
        copied = binary
    else:
        # Copy and flush exact bytes before launching the shipped executable.
        with binary.open("rb") as source, copied.open("wb") as target:
            shutil.copyfileobj(source, target)
            target.flush()
            os.fsync(target.fileno())
        copied.chmod(binary.stat().st_mode)
        assert hashlib.sha256(binary.read_bytes()).digest() == hashlib.sha256(copied.read_bytes()).digest()
    if windows:
        # Git for Windows needs its own runtime; keep only Git and Windows system dirs.
        git_paths = [str(Path(git).parent), str(Path(git).parent.parent / "mingw64" / "bin"),
                     str(Path(git).parent.parent / "usr" / "bin")]
        system = os.environ.get("SystemRoot", r"C:\Windows")
        path = os.pathsep.join([str(tools), *git_paths, system, str(Path(system) / "System32")])
    else:
        (tools / "git").symlink_to(git)
        path = str(tools)
    home = root / "home"
    home.mkdir()
    env = {**os.environ, "HOME": str(home), "USERPROFILE": str(home), "PATH": path,
           "GIT_CONFIG_NOSYSTEM": "1", "GIT_TERMINAL_PROMPT": "0"}
    assert not shutil.which("node", path=path) and not shutil.which("npm", path=path), "Fixture PATH must exclude Node/npm"

    def cmd(*args):
        result = subprocess.run(args, cwd=root, env=env, text=True, encoding="utf-8", errors="replace", stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=60)
        if result.returncode:
            raise AssertionError(f"Command failed ({result.returncode}): {args!r}\n{result.stdout}\n{result.stderr}")
        return result.stdout

    config_dir = home / ".vdw"
    config_dir.mkdir()
    workspaces = root / "workspaces"
    workspaces.mkdir()
    # Obtain complete defaults from the binary so fixtures follow the actual schema.
    config = json.loads(cmd(str(copied), "config", "show", "--json"))
    config.update(workspace_root=str(workspaces), store_path=str(root / "store"))
    config["lifecycle"]["archive"]["root"] = str(root / "archives")
    (config_dir / "config.json").write_text(json.dumps(config))

    def create(name):
        source = root / f"source-{name}"
        source.mkdir()
        cmd(git, "init", "-b", "main", str(source))
        (source / "README.md").write_text(f"# fixture {name}\n")
        cmd(git, "-C", str(source), "add", "README.md")
        cmd(git, "-C", str(source), "-c", "user.name=VDW Test", "-c", "user.email=vdw-test@example.invalid", "commit", "-m", "fixture")
        cmd(str(copied), "init", name)
        workspace = workspaces / name
        for args in [["repo", "add", str(source), "--path", "packages/sample", "--branch", "main", "--editable"],
                     ["sync", "--update-lock", "--json"], ["link", "--frozen"]]:
            result = subprocess.run([str(copied), *args], cwd=workspace, env=env, text=True, encoding="utf-8", errors="replace",
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=60)
            assert result.returncode == 0, f"{args!r}\n{result.stdout}\n{result.stderr}"
        return workspace

    clean = create("clean-fixture")
    dirty = create("dirty-fixture")
    (dirty / "packages/sample/new-file.txt").write_text("Preserve this untracked work.\n")
    # A pending AI/migration descriptor must not be resumed by dashboard reads.
    stdout = root / "server.log"
    output = stdout.open("w")
    server = subprocess.Popen([str(copied), "dashboard", "--no-open"], cwd=root, env=env,
                              stdout=output, stderr=subprocess.STDOUT)
    git_http = None
    try:
        instance_file = config_dir / "dashboard/instance.json"
        for _ in range(150):
            if instance_file.exists():
                break
            if server.poll() is not None:
                raise AssertionError(stdout.read_text())
            time.sleep(0.1)
        instance = json.loads(instance_file.read_text())
        origin, token = instance["origin"], instance["token"]
        handler = urllib.request.build_opener(urllib.request.ProxyHandler({}))

        def request(path, body=None, auth=True, origin_header=None, method=None, extra_headers=None):
            headers = {"Content-Type": "application/json"}
            if auth:
                headers["Authorization"] = "Bearer " + token
            if origin_header:
                headers["Origin"] = origin_header
            headers.update(extra_headers or {})
            req = urllib.request.Request(origin + path, data=json.dumps(body).encode() if body is not None else None,
                                         headers=headers, method=method)
            try:
                with handler.open(req, timeout=30) as response:
                    raw = response.read()
                    return response.status, json.loads(raw) if "json" in response.headers.get("Content-Type", "") else raw
            except urllib.error.HTTPError as error:
                return error.code, error.read()

        for _ in range(100):
            try:
                if request("/api/health", auth=False)[0] == 200:
                    break
            except OSError:
                time.sleep(0.1)
        assert request("/api/workspaces", auth=False)[0] == 401
        assert request("/api/workspaces", origin_header="https://example.invalid")[0] == 403
        assert request("/api/workspaces", extra_headers={"Sec-Fetch-Site": "cross-site"})[0] == 403
        assert request("/api/health", auth=False, extra_headers={"Host": "example.invalid"})[0] == 403
        assert request("/api/unknown")[0] == 404
        status, html = request("/", auth=False, extra_headers={"Sec-Fetch-Site": "cross-site"})
        assert status == 200 and b"root" in html
        for asset in re.findall(rb'(?:src|href)="(/assets/[^"]+)"', html):
            status, data = request(asset.decode(), auth=False)
            assert status == 200 and len(data) > 1000
        registry_path = config_dir / "workspaces.json"
        before = hashlib.sha256(registry_path.read_bytes()).hexdigest()
        status, snapshot = request("/api/workspaces")
        assert status == 200, snapshot
        assert len(snapshot["workspaces"]) == 2
        assert hashlib.sha256(registry_path.read_bytes()).hexdigest() == before, "Reads must not touch registry/activity"
        rows = {r["workspace"]["name"]: r for r in snapshot["workspaces"]}
        clean_id = rows["clean-fixture"]["workspace"]["workspace_id"]
        dirty_id = rows["dirty-fixture"]["workspace"]["workspace_id"]
        assert rows["clean-fixture"]["repositories"][0]["git"]["branch"] == "main"
        assert rows["dirty-fixture"]["dirty_count"] == 1
        assert all(r["last_modified_at"] and r["status"]["state"] == "active" for r in rows.values())
        original_modified = rows["clean-fixture"]["last_modified_at"]
        # Serve a disposable bare Git repository over dumb HTTP; no external network/credentials.
        bare = root / "remote.git"
        cmd(git, "clone", "--bare", str(root / "source-clean-fixture"), str(bare))
        cmd(git, "--git-dir", str(bare), "update-ref", "refs/heads/feature/check", "refs/heads/main")
        cmd(git, "--git-dir", str(bare), "update-server-info")

        remote_started = threading.Event()
        remote_release = threading.Event()
        delay_remote = threading.Event()

        class GitHandler(http.server.SimpleHTTPRequestHandler):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, directory=str(root), **kwargs)

            def do_GET(self):
                if delay_remote.is_set():
                    remote_started.set()
                    remote_release.wait(timeout=10)
                if (root / "delay-remote").exists():
                    time.sleep(3)
                super().do_GET()

            def log_message(self, *_args):
                pass

        git_http = http.server.ThreadingHTTPServer(("127.0.0.1", 0), GitHandler)
        git_thread = threading.Thread(target=git_http.serve_forever, daemon=True)
        git_thread.start()
        declaration = clean / ".vdwrc"
        original_config = declaration.read_bytes()
        original_times = declaration.stat()

        def remote_declaration(repository):
            declaration.write_text(json.dumps({"workspace": {"name": "clean-fixture", "version": "1.0"},
                "dependencies": [{"remote": f"http://127.0.0.1:{git_http.server_port}/{repository}",
                    "path": "packages/sample", "branch": "feature/check", "editable": True,
                    "scope": {"include": ["**/*"], "exclude": []}}]}))

        try:
            remote_declaration("remote.git")
            checked = request(f"/api/workspaces/{clean_id}/remote", {})[1]["packages/sample"]
            assert checked["repository_state"] == "available" and checked["branch_state"] == "present", checked
            delay_remote.set()
            concurrent = []
            remote_thread = threading.Thread(target=lambda: concurrent.append(request(f"/api/workspaces/{clean_id}/remote", {})), daemon=True)
            remote_thread.start()
            assert remote_started.wait(timeout=5), "Remote check did not start"
            started = time.monotonic()
            assert request(f"/api/workspaces/{dirty_id}/trash/preview", {})[0] == 200
            assert request(f"/api/workspaces/{dirty_id}/remote", {})[0] == 200
            assert remote_thread.is_alive() and time.monotonic() - started < 3, "Other card actions must not wait for the remote check"
            delay_remote.clear()
            remote_release.set()
            remote_thread.join(timeout=15)
            assert concurrent[0][0] == 200
            cmd(git, "--git-dir", str(bare), "update-ref", "-d", "refs/heads/feature/check")
            cmd(git, "--git-dir", str(bare), "update-server-info")
            checked = request(f"/api/workspaces/{clean_id}/remote", {})[1]["packages/sample"]
            assert checked["repository_state"] == "available" and checked["branch_state"] == "missing", checked
            remote_declaration("missing.git")
            checked = request(f"/api/workspaces/{clean_id}/remote", {})[1]["packages/sample"]
            assert checked["repository_state"] == "not_found_or_forbidden" and checked["branch_state"] == "unknown", checked
        finally:
            declaration.write_bytes(original_config)
            os.utime(declaration, ns=(original_times.st_atime_ns, original_times.st_mtime_ns))
            if not hold:
                git_http.shutdown()
                git_http.server_close()
                git_thread.join(timeout=2)
        assert hashlib.sha256(registry_path.read_bytes()).hexdigest() == before, "Remote checks must not update activity"
        assert request("/api/workspaces/not-an-id/open", {})[0] != 200
        assert request("/api/plans/not-previewed/execute", {"confirm": True})[0] == 400

        def execute(plan):
            key = plan["plan_id"]
            assert request(f"/api/plans/{key}/execute", {"confirm": False})[0] == 400
            assert request(f"/api/plans/{key}/execute", {"confirm": True})[0] == 202
            for _ in range(200):
                status, job = request(f"/api/jobs/{key}")
                assert status == 200
                if job["state"] not in ("running", "queued", "pending"):
                    return job
                time.sleep(0.1)
            raise AssertionError("Operation did not finish")

        status, plan = request("/api/plans/archive", {"workspace_ids": [dirty_id]})
        assert status == 200 and plan["entries"][0]["reason_codes"], plan
        job = execute(plan)
        assert job["operation"]["summary"]["skipped_count"] == 1, job
        assert (dirty / "packages/sample/new-file.txt").read_text() == "Preserve this untracked work.\n"
        # A pointer-only orphan is a non-blocking warning; populated children remain protected.
        repository = clean / "packages/sample"
        original_head = cmd(git, "-C", str(repository), "rev-parse", "HEAD").strip()
        (repository / ".gitmodules").write_text("")
        cmd(git, "-C", str(repository), "add", ".gitmodules")
        cmd(git, "-C", str(repository), "update-index", "--add", "--cacheinfo", f"160000,{original_head},nested-child")
        cmd(git, "-C", str(repository), "-c", "user.name=VDW Test", "-c", "user.email=vdw-test@example.invalid", "commit", "-m", "legacy orphan gitlink")
        (repository / "nested-child").mkdir()
        orphan_head = cmd(git, "-C", str(repository), "rev-parse", "HEAD").strip()
        orphan_tree = cmd(git, "-C", str(repository), "ls-tree", "HEAD", "--", ".gitmodules", "nested-child")
        assert not cmd(git, "-C", str(repository), "status", "--porcelain").strip()
        admin = Path(cmd(git, "-C", str(repository), "rev-parse", "--absolute-git-dir").strip())
        before_index = (admin / "index").read_bytes()
        before_registry = registry_path.read_bytes()
        row = next(r for r in request("/api/refresh", {})[1]["workspaces"] if r["workspace"]["workspace_id"] == clean_id)
        assert row["repositories"][0]["git"]["change_count"] == 0
        warnings = row["repositories"][0]["archive_warnings"]
        assert not row["repositories"][0]["archive_blockers"] and row["issue_count"] == 0, row
        assert warnings[0]["code"] == "empty_orphan_submodule" and warnings[0]["path"] == "nested-child"
        assert "允许归档" in warnings[0]["message"] and "原样保留指针" in warnings[0]["message"]
        assert before_registry == registry_path.read_bytes() and before_index == (admin / "index").read_bytes(), "Archive diagnostics must remain read-only"
        status, eligible = request("/api/plans/archive", {"workspace_ids": [clean_id]})
        assert status == 200 and not eligible["entries"][0]["reason_codes"], eligible
        assert eligible["entries"][0]["repositories"][0]["archive_warnings"] == warnings
        # Only disposable fixture content is changed. Parent Git may hide orphan contents.
        (repository / "nested-child/local-work.txt").write_text("Keep submodule work.\n")
        row = next(r for r in request("/api/refresh", {})[1]["workspaces"] if r["workspace"]["workspace_id"] == clean_id)
        assert row["repositories"][0]["archive_blockers"][0]["path"] == "nested-child" and row["issue_count"] > 0
        rejected = execute(eligible)
        assert not rejected["operation"]["summary"]["archived_count"]
        assert not rejected["operation"]["results"][0]["completed_steps"] and repository.is_dir()
        assert (repository / "nested-child/local-work.txt").read_text() == "Keep submodule work.\n"
        (repository / "nested-child/local-work.txt").unlink()
        row = next(r for r in request("/api/refresh", {})[1]["workspaces"] if r["workspace"]["workspace_id"] == clean_id)
        assert not row["repositories"][0]["archive_blockers"]
        original_modified = row["last_modified_at"]
        print("PASS: empty orphan gitlink warns without blocking; new child content stops reviewed archive; diagnostics remain read-only.", flush=True)
        status, plan = request("/api/plans/archive", {"workspace_ids": [clean_id]})
        assert status == 200, plan
        job = execute(plan)
        assert job["operation"]["summary"]["archived_count"] == 1, job
        archive_operation = job["operation"]["operation_id"]
        archived = request("/api/workspaces")[1]
        row = next(r for r in archived["workspaces"] if r["workspace"]["workspace_id"] == clean_id)
        assert row["workspace"]["lifecycle_state"] == "archived" and len(row["repositories"]) == 1
        assert row["status"]["state"] == "archived"
        assert row["last_modified_at"] == original_modified, "Archive must retain the source modification date"
        assert row["repositories"][0]["archive_warnings"] == warnings
        head = row["repositories"][0]["head"]
        status, undo = request(f"/api/operations/{archive_operation}/undo/preview", {})
        assert status == 200 and undo["kind"] == "undo", undo
        undo_job = execute(undo)
        assert undo_job["state"] == "completed", undo_job
        assert next(r for r in request("/api/workspaces")[1]["workspaces"] if r["workspace"]["workspace_id"] == clean_id)["workspace"]["lifecycle_state"] == "active"
        assert cmd(git, "-C", str(repository), "rev-parse", "HEAD").strip() == orphan_head
        assert cmd(git, "-C", str(repository), "ls-tree", "HEAD", "--", ".gitmodules", "nested-child") == orphan_tree
        assert not list((repository / "nested-child").iterdir())
        assert request(f"/api/operations/{archive_operation}/undo/preview", {})[0] == 400
        assert execute(undo)["operation"]["operation_id"] == undo_job["operation"]["operation_id"], "Undo must be idempotent"
        status, plan = request("/api/plans/archive", {"workspace_ids": [clean_id]})
        assert status == 200, plan
        job = execute(plan)
        assert job["state"] == "completed", job
        archive_operation = job["operation"]["operation_id"]
        row = next(r for r in request("/api/workspaces")[1]["workspaces"] if r["workspace"]["workspace_id"] == clean_id)
        status, plan = request("/api/plans/restore", {"archive_id": row["workspace"]["latest_archive_id"]})
        assert status == 200, plan
        job = execute(plan)
        assert job["operation"]["summary"]["restored_count"] == 1, job
        restore_operation = job["operation"]["operation_id"]
        assert request(f"/api/operations/{archive_operation}/undo/preview", {})[0] == 400, "Later lifecycle operations must block undo of older records"
        status, undo = request(f"/api/operations/{restore_operation}/undo/preview", {})
        assert status == 200, undo
        undo_job = execute(undo)
        assert undo_job["state"] == "completed", undo_job
        row = next(r for r in request("/api/workspaces")[1]["workspaces"] if r["workspace"]["workspace_id"] == clean_id)
        assert row["workspace"]["lifecycle_state"] == "archived"
        status, plan = request("/api/plans/restore", {"archive_id": row["workspace"]["latest_archive_id"]})
        assert status == 200, plan
        job = execute(plan)
        assert job["state"] == "completed", job
        restore_operation = job["operation"]["operation_id"]
        (clean / "new-local-notes.txt").write_text("Preserve new work after restore.")
        assert request(f"/api/operations/{restore_operation}/undo/preview", {})[0] == 400
        assert (clean / "new-local-notes.txt").read_text() == "Preserve new work after restore."
        (clean / "new-local-notes.txt").unlink()
        restored = request("/api/workspaces")[1]
        row = next(r for r in restored["workspaces"] if r["workspace"]["workspace_id"] == clean_id)
        assert row["repositories"][0]["git"]["head"] == head
        restored_modified = row["last_modified_at"]
        status, cfg = request("/api/config")
        assert status == 200
        prefs = cfg["preferences"]
        prefs["refresh_seconds"] = 20
        assert request("/api/preferences", {"version": cfg["preferences_version"], "preferences": prefs}, method="PUT")[0] == 200
        assert request("/api/preferences", {"version": cfg["preferences_version"], "preferences": prefs}, method="PUT")[0] == 409
        change = {"workspace_root": str(workspaces), "max_store_size_gb": 51, "gc_threshold_days": 31}
        status, plan = request("/api/config/preview", {"kind": "base", "version": cfg["version"], "value": change})
        assert status == 200, plan
        job = execute(plan)
        assert job["state"] == "completed", job
        assert request("/api/config")[1]["global"]["max_store_size_gb"] == 51
        assert request("/api/config/preview", {"kind": "base", "version": cfg["version"], "value": change})[0] == 409
        settings_operation = job["operation"]["operation_id"]
        status, undo = request(f"/api/operations/{settings_operation}/undo/preview", {})
        assert status == 200, undo
        assert execute(undo)["state"] == "completed"
        assert request("/api/config")[1]["global"]["max_store_size_gb"] == config["max_store_size_gb"]
        # A selected history row must not overwrite later configuration changes.
        cfg = request("/api/config")[1]
        status, plan = request("/api/config/preview", {"kind": "base", "version": cfg["version"], "value": change})
        prior = execute(plan)["operation"]["operation_id"]
        cfg = request("/api/config")[1]
        change["max_store_size_gb"] = 52
        status, plan = request("/api/config/preview", {"kind": "base", "version": cfg["version"], "value": change})
        assert execute(plan)["state"] == "completed"
        assert request(f"/api/operations/{prior}/undo/preview", {})[0] == 400
        # Policy undo restores inheritance instead of pinning the former effective default.
        cfg = request("/api/config")[1]
        status, plan = request("/api/config/preview", {"kind": "policy", "version": cfg["version"], "workspace_id": dirty_id, "key": "idle_days", "value": 99})
        assert status == 200, plan
        policy_operation = execute(plan)["operation"]["operation_id"]
        status, undo = request(f"/api/operations/{policy_operation}/undo/preview", {})
        assert status == 200, undo
        assert execute(undo)["state"] == "completed"
        row = next(r for r in request("/api/workspaces")[1]["workspaces"] if r["workspace"]["workspace_id"] == dirty_id)
        assert row["policy"]["idle_days"] is None
        history = request("/api/operations")[1]
        assert any(op["kind"] == "undo" for op in history)
        assert next(op for op in history if op["operation_id"] == settings_operation)["undo"]["operation_id"]
        for operation in history:
            saved_plan = json.loads((config_dir / "lifecycle/plans" / (operation["plan_id"] + ".json")).read_text())
            assert not (saved_plan.get("value") or {}).get("undo_parent"), "Internal undo steps must not appear as user operations"
        # Verify editor argv handling without launching a user's real editor.
        cfg = request("/api/config")[1]
        editor_script = root / "fake editor.py"
        editor_output = root / "editor-argv.json"
        editor_script.write_text("import json,sys\nfrom pathlib import Path\nPath(" + repr(str(editor_output)) + ").write_text(json.dumps(sys.argv[1:]))\n")
        prefs = cfg["preferences"]
        prefs.update(default_editor="custom", editor_program=sys.executable,
                     editor_args=[str(editor_script), "{workspace}"])
        assert request("/api/preferences", {"version": cfg["preferences_version"], "preferences": prefs}, method="PUT")[0] == 200
        status, opened = request(f"/api/workspaces/{clean_id}/open", {"editor": "custom"})
        assert status == 200 and opened["state"] == "launch_requested", opened
        assert opened["diagnostics"] is None, "A clean workspace must not prefill chats or change the clipboard"
        for _ in range(50):
            if editor_output.exists():
                break
            time.sleep(0.1)
        assert json.loads(editor_output.read_text()) == [str(clean.resolve())]
        assert request(f"/api/workspaces/{clean_id}/remote", {})[1]["packages/sample"]["state"] == "local"
        refreshed = request("/api/refresh", {})[1]
        opened_row = next(r for r in refreshed["workspaces"] if r["workspace"]["workspace_id"] == clean_id)
        assert opened_row["last_modified_at"] == restored_modified, "Opening an editor must not change the source date"
        cfg = request("/api/config")[1]
        prefs["default_editor"] = "auto"
        assert request("/api/preferences", {"version": cfg["preferences_version"], "preferences": prefs}, method="PUT")[0] == 200
        cfg = request("/api/config")[1]
        life = cfg["global"]["lifecycle"]
        life["dirty_policy"] = "force"
        assert request("/api/config/preview", {"kind": "lifecycle", "version": cfg["version"], "value": life})[0] == 400
        # Simultaneous CLI/dashboard processes use the same existing instance.
        reuse = cmd(str(copied), "dashboard", "--no-open")
        assert origin in reuse and "already running" in reuse
        assert request("/api/operations")[1], "Durable operation results should remain available"
        print("PASS: embedded HTML/JS/CSS; no Node/npm PATH; session/origin protection; read-only activity;")
        print("      actual Git state; dirty archive skip; clean archive and exact-HEAD restore;")
        print("      preferences/config concurrency; safe editor argv; plan confirmation; persistent operations; instance reuse.")
        print("      repository access, exact branch deletion, unknown permissions, and manual cache bypass.")
        print("PASS: selected archive/restore/config undo, repeat and later-change guards, hidden internal steps;")
        print("      fresh data after terminal job without manual refresh; concurrent per-card remote checks.")
        if test_system_trash:
            # Explicit opt-in: only this uniquely named disposable fixture enters the real OS trash.
            disposable = create("trash-fixture-" + root.name.removeprefix("vdw-dashboard-smoke-"))
            (disposable / "local-notes.txt").write_text("Keep local notes in the recycled directory.\n")
            (disposable / "packages/sample/.env").write_text("Ignored fixture material, not a real secret.\n")
            (disposable / "packages/sample/new-file.txt").write_text("Keep untracked work.\n")
            disposable_path = str(disposable.resolve())
            row = next(r for r in request("/api/refresh", {})[1]["workspaces"] if r["workspace"]["path"] == disposable_path)
            wid = row["workspace"]["workspace_id"]
            before = registry_path.read_bytes()
            status, trash_plan = request(f"/api/workspaces/{wid}/trash/preview", {})
            assert status == 200 and trash_plan["kind"] == "trash", trash_plan
            assert before == registry_path.read_bytes() and disposable.is_dir(), "Preview must not move files or alter registration"
            endpoint = f"/api/plans/{trash_plan['plan_id']}/execute"
            assert request(endpoint, {"confirm": False})[0] == 400
            assert disposable.is_dir(), "An explicit confirmation is required"
            assert request(endpoint, {"confirm": True})[0] == 202
            for _ in range(150):
                trash_job = request(f"/api/jobs/{trash_plan['plan_id']}")[1]
                if trash_job["state"] not in ["queued", "running"]:
                    break
                time.sleep(0.1)
            assert trash_job["state"] == "completed", trash_job
            assert trash_job["operation"]["summary"]["trashed_count"] == 1
            assert not disposable.exists(), "A successful recycle must move the entire directory"
            assert request(endpoint, {"confirm": True})[1]["state"] == "completed", "Repeated confirmation is idempotent"
            assert request(f"/api/workspaces/{wid}/trash/preview", {})[0] == 400
            assert not any(r["workspace"]["workspace_id"] == wid for r in request("/api/workspaces")[1]["workspaces"])
            registration = next(w for w in json.loads(registry_path.read_text())["workspaces"] if w["workspace_id"] == wid)
            assert registration["lifecycle_state"] == "trashed"
            inventory = json.loads(cmd(str(copied), "store", "status", "--json"))
            retained = next(w for w in inventory["registered_workspaces"] if w["path"] == disposable_path)
            assert retained["state"] == "ready" and "trash" in retained["message"], retained
            print("PASS: whole-directory OS trash; explicit confirmation; idempotency; list removal; durable result.", flush=True)
            trash_operation = trash_job["operation"]["operation_id"]
            plans_before = len(list((config_dir / "lifecycle/plans").glob("*.json")))
            registry_before = registry_path.read_bytes()
            assert next(op for op in request("/api/operations")[1] if op["operation_id"] == trash_operation)["undo"]["available"]
            assert plans_before == len(list((config_dir / "lifecycle/plans").glob("*.json"))), "History availability must not generate plans"
            assert registry_before == registry_path.read_bytes(), "History availability must not alter registration"
            disposable.mkdir()
            (disposable / "occupied.txt").write_text("Never overwrite this directory")
            blocked_status = next(op for op in request("/api/operations")[1] if op["operation_id"] == trash_operation)["undo"]
            assert not blocked_status["available"] and "原路径" in blocked_status["reason"], blocked_status
            status, blocked = request(f"/api/operations/{trash_operation}/undo/preview", {})
            assert status == 400 and "原路径" in blocked.decode(), blocked
            (disposable / "occupied.txt").unlink()
            disposable.rmdir()
            status, undo = request(f"/api/operations/{trash_operation}/undo/preview", {})
            assert status == 200, undo
            job = execute(undo)
            assert job["state"] == "completed", job
            assert (disposable / "local-notes.txt").read_text() == "Keep local notes in the recycled directory.\n"
            assert (disposable / "packages/sample/.env").read_text() == "Ignored fixture material, not a real secret.\n"
            assert (disposable / "packages/sample/new-file.txt").read_text() == "Keep untracked work.\n"
            row = next(r for r in request("/api/workspaces")[1]["workspaces"] if r["workspace"]["workspace_id"] == wid)
            assert row["workspace"]["lifecycle_state"] == "active"
            assert request(f"/api/operations/{trash_operation}/undo/preview", {})[0] == 400
            assert not any(path.read_text() == f"VDW trash retention: {wid}" for path in (root / "store/objects").glob("*/bare.git/worktrees/*/locked"))
            print("PASS: undo trash restores exact original notes, ignored/untracked files and registration; path collision blocked.", flush=True)
        if use_source_binary:
            print("NOTE: executable runs from its build location; HOME, cwd and no-Node PATH remain isolated.")
        if hold:
            # Leave the disposable missing-branch case available for visual browser verification.
            remote_declaration("remote.git")
            request("/api/refresh", {})
            state = root / "browser-state.json"
            state.write_text(json.dumps({"origin": origin, "url": origin + "/#session=" + token, "home": str(home),
                                         "server_pid": server.pid, "runner_pid": os.getpid()}))
            print(f"Browser fixture: {state}", flush=True)
            while server.poll() is None:
                time.sleep(1)
    finally:
        if git_http is not None and hold:
            git_http.shutdown()
            git_http.server_close()
        if server.poll() is None:
            if windows:
                server.terminate()
            else:
                server.send_signal(signal.SIGINT)
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
        output.close()
        if not hold:
            shutil.rmtree(root)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--hold", action="store_true", help="Keep isolated fixture server for browser verification")
    parser.add_argument("--use-source-binary", action="store_true", help="Use the built executable in place when the host blocks executing temporary copies; data/cwd/PATH remain isolated")
    parser.add_argument("--test-system-trash", action="store_true", help="Explicitly recycle one uniquely named temporary fixture using the real system trash")
    args = parser.parse_args()
    run(args.binary.resolve(), args.hold, args.use_source_binary, args.test_system_trash)

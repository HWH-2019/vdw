# VDW仓库管理完善_需求文档_v1.0

---

## 1. 改动背景

### 1.1 现状分析

VDW 当前在仓库移除和工作空间初始化两个环节存在用户体验不足：

1. **仓库移除不检测未提交变更**：`vdw repo remove` (CLI) 和 `vdw.removeRepo` (VSCode) 在移除仓库时，直接执行 unlink → 删除 .vdwrc 配置 → 清理 store 的流程，**不会检测 worktree 中是否存在未提交的变更**（包括 staged/unstaged 文件修改、未追踪文件等）。如果用户在 editable 仓库中进行了修改但忘记提交，移除操作会直接丢弃这些变更，导致工作成果丢失。

2. **VSCode 插件无初始化引导**：当用户用 VS Code/Qoder 打开一个尚未包含 `.vdwrc` 文件的项目时，插件因 `activationEvents: ["workspaceContains:.vdwrc"]` 条件不满足而**根本不会被激活**。用户必须先在终端手动执行 `vdw init`，然后重新加载窗口才能使用插件功能，入门体验不友好。

### 1.2 业务诉求

- 仓库移除前检测变更状态，给予用户"清理变更"或"保留变更"的选择权，避免工作成果意外丢失
- VSCode 插件在无 `.vdwrc` 时也能激活并提供初始化入口，降低新用户上手门槛

---

## 2. 改动目标

### 2.1 核心目标

- CLI `vdw repo remove` 在移除 editable 仓库前检测 worktree 脏状态，通过 `dialoguer` 交互式提示用户选择处理方式
- VSCode 插件 `vdw.removeRepo` 在移除前检测变更，通过 `showWarningMessage` 弹窗让用户选择
- VSCode 插件将 `activationEvents` 扩展为始终激活（或添加 `*`），在无 `.vdwrc` 时显示"初始化 VDW 工作空间"引导按钮
- 版本号从 `0.5.0` 升级到 `0.6.0`（或下一个合适版本）

### 2.2 非目标

- 不修改 `vdw link`/`vdw unlink` 的现有逻辑
- 不修改 Content Store 的存储结构
- 不引入新的后端服务或网络依赖
- 不改变 editable lock 机制

---

## 3. 功能需求

### 3.1 CLI 侧：仓库移除前检测变更

#### 功能描述

在 `vdw repo remove <name>` 执行移除前，如果被移除的依赖为 editable（worktree），检测其 worktree 路径下是否存在未提交的变更。如果存在变更，通过 `dialoguer` 给出交互式选择。

#### 涉及文件

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `vdw/src/worktree/git_ops.rs` | 新增函数 | 添加 `has_uncommitted_changes(path: &Path) -> Result<bool>` |
| `vdw/src/commands/repo.rs` | 修改 `remove()` | 在 unlink 之前插入变更检测和交互逻辑 |

#### 变更检测规则

通过 `git2::Repository` 打开 worktree 路径（非 bare repo），使用 `statuses()` API 检测：

- **Modified**：已追踪文件有修改（staged 或 unstaged）
- **Added/Deleted**：staged 的新增或删除
- **Untracked**：未追踪的新文件

以下情况视为**"存在变更"**（dirty）：
```
statuses 返回非空结果（排除 .gitignore 已忽略的文件）
```

#### 交互流程

```
vdw repo remove api-service

⚠️  Repository "api-service" (branch: feature/dev) has uncommitted changes:
    3 modified, 1 untracked

  What would you like to do?
  ❯ Discard changes and remove
    Keep worktree files (remove from config only)
    Cancel
```

- **Discard changes and remove**：正常执行现有移除流程（unlink → remove config → clean store）
- **Keep worktree files**：仅从 `.vdwrc` 中移除依赖配置，不 unlink 也不清理 store/worktree，保留 worktree 目录供用户后续手动处理
- **Cancel**：中止操作，不做任何修改

#### 业务规则

- 仅对 `editable: true` 的依赖执行变更检测；只读依赖直接走现有移除流程
- 如果 worktree 路径不存在（已 unlink），跳过检测，直接移除
- 非交互终端（如 CI 环境 `!atty`）默认行为：打印警告后直接移除（保持向后兼容）
- 变更检测失败时（如 git2 打开失败），打印警告并继续移除（graceful degradation）

### 3.2 VSCode 侧：仓库移除前检测变更

#### 功能描述

在 `vdw.removeRepo` 命令执行移除前，检测 editable 依赖的 worktree 是否存在未提交变更。如果存在，在确认弹窗中展示变更信息，并给出处理选项。

#### 涉及文件

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `vdw-vscode/src/commands/index.ts` | 修改 `vdw.removeRepo` | 在 unlink 之前插入变更检测逻辑 |
| `vdw-vscode/src/services/vdwCliService.ts` | 新增方法 | 添加 `checkDirty(repoPath: string): Promise<DirtyCheckResult>` |

#### 变更检测方案

**方案 A（推荐）：复用 CLI 子命令**

新增 CLI 子命令 `vdw repo status <name>`，输出 JSON 格式的变更状态：

```json
{
  "repo": "api-service",
  "branch": "feature/dev",
  "dirty": true,
  "changes": {
    "modified": 3,
    "added": 0,
    "deleted": 1,
    "untracked": 2
  }
}
```

VSCode 插件通过 `cliService.executeCommand('repo', ['status', repoIdentifier])` 调用并解析结果。

**方案 B（备选）：VSCode 内置 Git API**

使用 VS Code 的 `vscode.extensions.getExtension('vscode.git')` API 检查 worktree 路径的 Git 状态。但此方案依赖 VS Code Git 扩展已注册该仓库，不够可靠。

**选择方案 A**，因为它复用了 CLI 的 git2 实现，保持一致性。

#### 交互流程

1. 用户点击 TreeView 中的 Remove 按钮
2. 插件调用 `vdw repo status <repo>` 检测变更
3. 如果 dirty，弹出增强确认框：

```
⚠️ Remove repository "api-service"?

Path: services/api

This repository has uncommitted changes (3 modified, 2 untracked).

[Discard & Remove]  [Keep Files]  [Cancel]
```

- **Discard & Remove**：执行现有完整移除流程
- **Keep Files**：仅从 .vdwrc 移除配置项，保留 worktree 文件
- **Cancel**：取消操作

4. 如果 clean，弹出标准确认框（现有逻辑不变）

#### 业务规则

- 仅对 `dep.editable === true` 执行变更检测
- 如果 `vdw repo status` 命令不存在或失败（旧版 CLI），graceful degradation 到现有行为（直接确认移除）
- 检测超时阈值 5 秒，超时视为 clean

### 3.3 VSCode 侧：无 .vdwrc 时显示初始化引导

#### 功能描述

当 VS Code 打开的工作空间中不存在 `.vdwrc` 文件时，VDW 插件仍然激活，并在 TreeView 中显示"初始化 VDW 工作空间"的引导按钮。

#### 涉及文件

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `vdw-vscode/package.json` | 修改 `activationEvents` | 移除 `workspaceContains:.vdwrc` 限制，改为 `onStartupFinished` 或 `*` |
| `vdw-vscode/package.json` | 修改 `views` 的 `when` 条件 | 为 `vdwWorkspace` view 添加 `when` 子句或保持始终可见 |
| `vdw-vscode/src/providers/workspaceTreeView.ts` | 修改 `getChildren()` | 在无配置时显示初始化引导项 |
| `vdw-vscode/src/extension.ts` | 修改 `activate()` | 适配无 .vdwrc 场景，部分服务延迟初始化 |

#### activationEvents 修改

**当前：**
```json
"activationEvents": [
    "workspaceContains:.vdwrc"
]
```

**目标：**
```json
"activationEvents": [
    "workspaceContains:.vdwrc",
    "onStartupFinished"
]
```

添加 `onStartupFinished` 使插件在 VS Code 启动后始终激活，但保留 `workspaceContains:.vdwrc` 以确保有 .vdwrc 时立即激活。

#### TreeView 引导 UI

当 `configService.getConfig()` 返回 `null`（无 .vdwrc）时，`getChildren()` 返回引导项：

```
┌──────────────────────────────────────┐
│  🔧 Initialize VDW Workspace         │
│     Click to set up your workspace   │
└──────────────────────────────────────┘
```

- 点击触发 `vdw.init` 命令
- TreeItem 配置：`iconPath: $(play)`, `command: 'vdw.init'`, `contextType: 'initGuide'`

#### 服务初始化策略调整

| 服务 | 有 .vdwrc | 无 .vdwrc |
|------|-----------|-----------|
| `VDWCliService` | ✅ 立即初始化 | ✅ 立即初始化 |
| `ConfigService` | ✅ 加载配置 | ✅ 加载（返回 null） |
| `WorkspaceTreeProvider` | ✅ 显示依赖树 | ✅ 显示初始化引导 |
| `StatusBarManager` | ✅ 显示状态 | ⏸️ 隐藏或显示"未初始化" |
| `ContextService` | ✅ 正常 | ⏸️ 跳过（无依赖可处理）|
| `AutoOpenService` | ✅ 打开 Git 仓库 | ⏸️ 跳过（无 editable 依赖）|
| `FileProvider` | ✅ 正常 | ⏸️ 跳过（无文件可提供） |
| `.vdwrc` watcher | ✅ 监听变更 | ✅ 监听创建事件 |

当 `.vdwrc` 被创建后（通过 `onDidCreate` watcher），所有延迟初始化的服务自动激活，与现有 reload 逻辑一致。

#### 业务规则

- 仅在工作空间存在至少一个文件夹时显示引导（`workspaceFolders.length > 0`）
- 如果工作空间为空（没有打开文件夹），保持现有行为（打印日志并 return）
- `.vdwrc` 创建后，TreeView 自动刷新切换为依赖列表视图
- 引导项不显示 context menu（右键菜单），仅有点击触发 init 的功能

---

## 4. CLI 新增子命令：`vdw repo status`

### 功能描述

为支持 VSCode 插件的变更检测需求，新增 `vdw repo status <name>` 子命令，以 JSON 格式输出指定仓库的变更状态。

### 涉及文件

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `vdw/src/commands/repo.rs` | 新增 `status()` 函数 | 实现变更检测逻辑 |
| `vdw/src/commands/mod.rs` | 注册子命令 | 在 `RepoSubcommand` enum 中添加 `Status` |
| `vdw/src/main.rs` | 路由分发 | 添加 `Status` 的 match 分支 |

### 命令格式

```bash
vdw repo status <name> [--json]
```

### 输出示例

**有变更（dirty）：**
```json
{
  "repo": "api-service",
  "branch": "feature/dev",
  "editable": true,
  "dirty": true,
  "changes": {
    "modified": 3,
    "added": 0,
    "deleted": 1,
    "untracked": 2
  },
  "worktree_path": "/Users/xxx/.vdw/store/objects/api-service-abc123/worktrees/feature-dev"
}
```

**无变更（clean）：**
```json
{
  "repo": "api-service",
  "branch": "feature/dev",
  "editable": true,
  "dirty": false,
  "changes": {
    "modified": 0,
    "added": 0,
    "deleted": 0,
    "untracked": 0
  },
  "worktree_path": "/Users/xxx/.vdw/store/objects/api-service-abc123/worktrees/feature-dev"
}
```

**非 editable 依赖：**
```json
{
  "repo": "shared-lib",
  "branch": "main",
  "editable": false,
  "dirty": false,
  "changes": null,
  "worktree_path": null
}
```

### 实现要点

1. 通过 `resolve_repo_id()` 解析仓库标识
2. 从 `.vdwrc` 中查找匹配的依赖项
3. 如果 `dep.editable == true` 且 `dep.local` 非空且路径存在：
   - 使用 `git2::Repository::open(dep.local)` 打开 worktree
   - 调用 `repo.statuses(None)` 获取状态列表
   - 按 `MODIFIED | INDEX_NEW | INDEX_MODIFIED | INDEX_DELETED | INDEX_RENAMED | INDEX_TYPECHANGE | WT_NEW` 分类统计
4. 输出 JSON（默认 `--json`，可省略 flag 因为此命令面向程序调用）

### 错误处理

| 场景 | 行为 |
|------|------|
| 仓库不存在 | `exit 1`，stderr 输出错误信息 |
| worktree 路径不存在 | `dirty: false`, `changes: null` |
| git2 打开失败 | `exit 1`，stderr 输出错误信息 |

---

## 5. 技术设计

### 5.1 变更检测核心函数

**位置**：`vdw/src/worktree/git_ops.rs`

```rust
/// 检测结果
pub struct DirtyStatus {
    pub modified: usize,
    pub added: usize,
    pub deleted: usize,
    pub untracked: usize,
}

impl DirtyStatus {
    pub fn is_dirty(&self) -> bool {
        self.modified + self.added + self.deleted + self.untracked > 0
    }

    pub fn total(&self) -> usize {
        self.modified + self.added + self.deleted + self.untracked
    }
}

/// 检测 worktree 路径下是否存在未提交的变更
pub fn check_dirty(worktree_path: &Path) -> Result<DirtyStatus> {
    let repo = Repository::open(worktree_path)?;
    let statuses = repo.statuses(None)?;

    let mut modified = 0;
    let mut added = 0;
    let mut deleted = 0;
    let mut untracked = 0;

    for entry in statuses.iter() {
        let status = entry.status();
        if status.intersects(git2::Status::INDEX_NEW | git2::Status::INDEX_MODIFIED
            | git2::Status::INDEX_DELETED | git2::Status::INDEX_RENAMED
            | git2::Status::INDEX_TYPECHANGE) {
            // Staged changes
            if status.contains(git2::Status::INDEX_NEW) { added += 1; }
            if status.contains(git2::Status::INDEX_MODIFIED) { modified += 1; }
            if status.contains(git2::Status::INDEX_DELETED) { deleted += 1; }
        } else if status.intersects(git2::Status::WT_MODIFIED | git2::Status::WT_TYPECHANGE) {
            modified += 1;
        } else if status.intersects(git2::Status::WT_DELETED) {
            deleted += 1;
        } else if status.contains(git2::Status::WT_NEW) {
            untracked += 1;
        }
    }

    Ok(DirtyStatus { modified, added, deleted, untracked })
}
```

### 5.2 CLI 交互流程实现

**位置**：`vdw/src/commands/repo.rs` 的 `remove()` 函数

```rust
// 在 Step 2 (unlink) 之前插入：
if let Some(ref id) = repo_id {
    // 查找匹配的依赖
    if let Ok(config) = parser::load_config(&workspace_dir) {
        for dep in &config.dependencies {
            if dep.dep_id() == *id && dep.editable && !dep.local.is_empty() {
                let local_path = Path::new(&dep.local);
                if local_path.exists() {
                    match git_ops::check_dirty(local_path) {
                        Ok(status) if status.is_dirty() => {
                            // 交互式提示
                            if atty::is(atty::Stream::Stdin) {
                                let choice = handle_dirty_prompt(&dep, &status)?;
                                match choice {
                                    DirtyChoice::Discard => { /* 继续正常移除 */ }
                                    DirtyChoice::KeepFiles => {
                                        // 仅从配置中移除，不 unlink
                                        remove_from_config(&name)?;
                                        return Ok(());
                                    }
                                    DirtyChoice::Cancel => {
                                        println!("Operation cancelled.");
                                        return Ok(());
                                    }
                                }
                            } else {
                                // 非交互环境，打印警告后继续
                                eprintln!("Warning: {} has uncommitted changes ({} files)",
                                    id, status.total());
                            }
                        }
                        _ => { /* clean 或检测失败，继续正常移除 */ }
                    }
                }
                break;
            }
        }
    }
}
```

### 5.3 VSCode 交互流程实现

**位置**：`vdw-vscode/src/commands/index.ts` 的 `vdw.removeRepo` 命令

```typescript
// 在 Step 1 (unlink) 之前插入：
if (dep.editable) {
    try {
        const statusResult = await cliService.executeCommand('repo', ['status', repoIdentifier]);
        if (statusResult.exitCode === 0) {
            const status = JSON.parse(statusResult.stdout);
            if (status.dirty) {
                const { modified, untracked, added, deleted } = status.changes;
                const parts: string[] = [];
                if (modified > 0) parts.push(`${modified} modified`);
                if (added > 0) parts.push(`${added} added`);
                if (deleted > 0) parts.push(`${deleted} deleted`);
                if (untracked > 0) parts.push(`${untracked} untracked`);

                const choice = await vscode.window.showWarningMessage(
                    `Repository "${repoName}" has uncommitted changes (${parts.join(', ')}).\n\n` +
                    `Path: ${dep.path}\n\nHow would you like to proceed?`,
                    { modal: true },
                    'Discard & Remove',
                    'Keep Files',
                    'Cancel'
                );

                if (choice === 'Keep Files') {
                    // 仅从 .vdwrc 移除配置
                    removeFromVdwrc(dep);
                    refreshUI();
                    return;
                }
                if (choice !== 'Discard & Remove') {
                    return; // Cancel or dismissed
                }
            }
        }
    } catch {
        // CLI 不支持 status 命令（旧版本），降级到现有行为
    }
}
```

### 5.4 TreeView 初始化引导实现

**位置**：`vdw-vscode/src/providers/workspaceTreeView.ts`

```typescript
getChildren(element?: WorkspaceTreeItem): WorkspaceTreeItem[] {
    const config = this.configService.getConfig();

    // 无 .vdwrc 文件时显示初始化引导
    if (!config) {
        const initItem = new WorkspaceTreeItem(
            'Initialize VDW Workspace',
            vscode.TreeItemCollapsibleState.None,
            'initGuide'
        );
        initItem.iconPath = new vscode.ThemeIcon('play-circle');
        initItem.description = 'Click to set up your virtual dev workspace';
        initItem.command = {
            command: 'vdw.init',
            title: 'Initialize VDW Workspace',
        };
        return [initItem];
    }

    // ... 现有逻辑不变
}
```

---

## 6. 影响范围

### 6.1 CLI 改动范围

| 模块 | 文件 | 风险等级 |
|------|------|----------|
| worktree | `git_ops.rs` | 低（新增函数，不影响现有逻辑） |
| commands | `repo.rs` | 中（修改 remove 流程，需确保向后兼容） |
| commands | `mod.rs` | 低（新增子命令注册） |
| main | `main.rs` | 低（新增 match 分支） |
| types | `config.rs` | 低（新增 DirtyStatus 结构体，如果需要） |

### 6.2 VSCode 改动范围

| 模块 | 文件 | 风险等级 |
|------|------|----------|
| commands | `index.ts` | 中（修改 removeRepo 流程） |
| providers | `workspaceTreeView.ts` | 低（新增引导 UI 分支） |
| services | `vdwCliService.ts` | 低（新增 status 调用方法） |
| config | `package.json` | 中（修改 activationEvents，需验证不影响现有激活逻辑） |
| extension | `extension.ts` | 中（适配无 .vdwrc 场景，部分服务延迟初始化） |

### 6.3 兼容性

- `vdw repo status` 为新增命令，不影响现有命令
- `vdw repo remove` 在非交互终端保持现有行为（向后兼容）
- VSCode 插件的 activationEvents 变更为追加模式，有 .vdwrc 时行为完全不变
- VSCode 插件在 `vdw repo status` 不可用时（旧版 CLI）graceful degradation

---

## 7. 测试要点

### 7.1 CLI 测试

| 测试场景 | 预期结果 |
|----------|----------|
| `vdw repo remove` 移除 clean 的 editable 仓库 | 直接移除，无额外提示 |
| `vdw repo remove` 移除 dirty 的 editable 仓库（交互终端） | 显示选择菜单，三种选项均可正常工作 |
| `vdw repo remove` 移除 dirty 的 editable 仓库（非交互终端） | 打印警告后直接移除 |
| `vdw repo remove` 移除只读仓库（有/无变更） | 直接移除，不检测变更 |
| `vdw repo remove` worktree 路径不存在 | 跳过检测，正常移除 |
| `vdw repo status` 查询 clean 仓库 | 输出 `dirty: false` |
| `vdw repo status` 查询 dirty 仓库 | 输出变更统计 JSON |
| `vdw repo status` 查询不存在的仓库 | exit 1 + 错误信息 |
| `vdw repo status` 查询只读仓库 | 输出 `dirty: false, changes: null` |

### 7.2 VSCode 测试

| 测试场景 | 预期结果 |
|----------|----------|
| Remove clean editable 仓库 | 弹出标准确认框，正常移除 |
| Remove dirty editable 仓库 | 弹出增强确认框，三个选项均可正常工作 |
| Remove dirty 仓库选择 "Keep Files" | 仅从 .vdwrc 移除，worktree 文件保留 |
| Remove dirty 仓库选择 "Cancel" | 不做任何操作 |
| Remove 只读仓库 | 不检测变更，弹出标准确认框 |
| 旧版 CLI（无 status 命令）时 remove | graceful degradation 到现有行为 |
| 打开无 .vdwrc 的项目 | TreeView 显示"Initialize VDW Workspace"按钮 |
| 点击初始化按钮 | 触发 `vdw.init`，创建 .vdwrc 后自动切换为依赖列表 |
| 有 .vdwrc 的项目 | 行为与改动前完全一致 |

---

## 8. 里程碑

| 阶段 | 内容 | 优先级 |
|------|------|--------|
| P0 | CLI `git_ops::check_dirty()` 实现 | 高 |
| P0 | CLI `vdw repo status` 子命令 | 高 |
| P0 | CLI `vdw repo remove` 变更检测与交互 | 高 |
| P1 | VSCode `vdw.removeRepo` 变更检测弹窗 | 高 |
| P1 | VSCode `activationEvents` 扩展 + 初始化引导 UI | 中 |
| P2 | 集成测试覆盖 | 中 |

# VDW 依赖配置重构 & AutoOpenService 简化

## 版本历史

| 版本 | 日期 | 描述 | 作者 |
|------|------|------|------|
| v1.0 | 2026-06-02 | 初始版本 | - |

## 1. 概述

### 1.1 背景

当前 VDW 的依赖配置（`.vdwrc`）的设计存在以下问题：

1. **`repo` 字段语义不清晰**：该字段同时承担"远程 Git URL"和"本地文件系统路径"两种角色，导致代码中需要多次判断 repo 值的类型。
2. **AutoOpenService 获取 worktree 路径需要查 state.json**：VS Code 插件需要打开 editable 依赖的 Git 仓库时，必须从 `.vdw/state.json` 中匹配 `repo_url` + `branch` 来查找 worktree 路径，流程繁琐且 `state.json` 可能不存在或已过期。
3. **冗余状态同步**：`.vdwrc` 中已有依赖列表，`state.json` 中又维护了一份 worktree 路径，两份数据需要保持同步，增加了不一致的风险。

### 1.2 目标

- 重构 `.vdwrc` 的依赖配置结构，明确区分远程地址和本地路径
- 将 worktree/local checkout 路径直接记录在 `.vdwrc` 中，消除对 `state.json` 的依赖
- 简化 `AutoOpenService` 的逻辑，直接从配置中读取 worktree 路径

### 1.3 范围

| 模块 | 改动范围 |
|------|---------|
| Rust CLI: `types/config.rs` | `Dependency` 结构体重构 |
| Rust CLI: `commands/link.rs` | worktree 创建后写入 `.vdwrc` |
| Rust CLI: `config/parser.rs` | 兼容旧版本 `.vdwrc` 解析 |
| VS Code: `types/vdwTypes.ts` | `Dependency` 接口重构 |
| VS Code: `services/autoOpenService.ts` | 简化 worktree 路径查找 |
| VS Code: `providers/workspaceTreeView.ts` | 适配新配置结构 |
| VS Code: `commands/addRepo.ts` | 适配新配置结构 |
| VS Code: `services/configService.ts` | 适配新配置结构 |

## 2. 功能需求

### 2.1 依赖配置结构重构

#### 2.1.1 当前结构（待替换）

```yaml
dependencies:
  - repo: git@gitlab.alibaba-inc.com:eleme-domPoi/alsc.sales_aoi_service.git
    path: ./packages/alsc.sales_aoi_service
    branch: master
    editable: true
```

#### 2.1.2 新结构

```yaml
dependencies:
  # 场景一：远程仓库（通过 Git URL 克隆）
  - remote: git@gitlab.alibaba-inc.com:eleme-domPoi/alsc.sales_aoi_service.git
    local: /Users/tbsg/.vdw/store/objects/alsc.sales_aoi_service-3d5c6756/worktrees/master
    path: ./packages/alsc.sales_aoi_service
    branch: master
    editable: true

  # 场景二：本地仓库（已存在的本地 Git 仓库）
  - remote: ''
    local: /Users/tbsg/Desktop/codes/agent-wiki
    path: ./packages/agent-wiki
    branch: main
    editable: true
```

#### 2.1.3 字段说明

| 字段 | 必填 | 说明 |
|------|------|------|
| `remote` | 否 | 远程 Git 仓库 URL（如 `git@github.com:org/repo.git`），本地仓库可为空 |
| `local` | 是 | 本地 Git 仓库路径（worktree 或本地仓库的绝对路径） |
| `path` | 是 | 工作区中的链接目标路径（相对于 `.vdwrc`） |
| `branch` | 否 | 分支名 |
| `tag` | 否 | 标签名 |
| `commit` | 否 | 提交 SHA |
| `scope.include` | 是 | 文件包含模式 |
| `scope.exclude` | 否 | 文件排除模式 |
| `editable` | 否 | 是否可编辑（默认 false） |

#### 2.1.4 验证规则

- `remote` 和 `local` 不能同时为空
- `remote` 格式校验：必须为 `git@`、`https://`、`http://` 开头，或为空字符串
- `local` 必须为有效的文件系统路径
- `branch` / `tag` / `commit` 三者互斥，必须指定至少一个（旧规则沿用）
- `scope.include` 不能为空

### 2.2 AutoOpenService 简化

#### 2.2.1 当前方案（问题分析）

当前 `openAllEditableRepositories` 流程：

```
遍历 config.dependencies (filter editable)
  -> getWorktreePath(dep) 读取 state.json
    -> 解析 state.json
    -> 在 worktrees[] 中匹配 repo_url + branch
    -> 返回 worktree path
  -> isValidGitRepository(worktreePath)
  -> git.openRepository(uri)
```

**问题 1**：`state.json` 需要预先通过 CLI 的 `vdw link` 创建，如果用户修改了 `.vdwrc` 但未执行 `vdw link`，worktree 路径无法获取。

**问题 2**：`state.json` 中的 `worktrees` 路径通过 `repo_url + branch` 匹配，当同一 repo 有多个分支时匹配逻辑脆弱。

**问题 3**：`state.json` 路径位于 `.vdw` 目录，不在版本控制中，团队协作时需要额外步骤。

#### 2.2.2 新方案

```
遍历 config.dependencies (filter editable)
  -> dep.local 即为 worktree 路径（直接从 .vdwrc 读取）
  -> isValidGitRepository(dep.local)
  -> git.openRepository(uri)
```

**简化点**：
- 删除 `getWorktreePath()` 方法（不再需要读 `state.json`）
- 删除 `WorkspaceState` 和 `WorktreeInfo` 接口（不再需要解析 `state.json` worktrees）
- 从 `O(n²)` 匹配（遍历 deps × 遍历 worktrees）降为 `O(1)` 直接读取

### 2.3 CLI worktree 路径写入

#### 2.3.1 `vdw link` 命令修改

执行 `vdw link` 时，对于 editable 依赖，创建 worktree 后将 worktree 路径写入 `.vdwrc` 的 `local` 字段：

```rust
// 当前代码（link.rs）
let wt_path = worktree_manager::create_worktree(&bare_git_path, branch, &worktree_base)?;
state.worktrees.push(WorktreeInfo { ... });

// 新增：将 worktree 路径写回 .vdwrc
dep.local = Some(wt_path.to_string_lossy().to_string());
save_config_to_vdwrc(&config)?;
```

#### 2.3.2 `vdw repo add` 命令修改

执行 `vdw repo add` 后，对于 editable 仓库，在克隆完成后创建 worktree 并将路径写入 `.vdwrc`：

```rust
// 新增：create 成功后写回 local
if editable {
    let wt_path = worktree_manager::create_worktree(&bare_git_path, &branch, &worktree_base)?;
    dependency.local = Some(wt_path.to_string_lossy().to_string());
}
```

### 2.4 向后兼容

#### 2.4.1 旧配置兼容

解析 `.vdwrc` 时，如果 `dependency` 使用旧的 `repo` 字段（没有 `remote`/`local`），自动做兼容处理：

```rust
// config/parser.rs 增强
if dep.remote.is_none() && dep.local.is_none() {
    // 旧的 repo 字段：判断是 remote URL 还是 local path
    if dep.repo.starts_with("git@") || dep.repo.starts_with("https://") || dep.repo.starts_with("http://") {
        dep.remote = Some(dep.repo.clone());
        // 不设置 local，等待 vdw link 时填充
    } else {
        // 是本地路径
        dep.local = Some(dep.repo.clone());
    }
}
```

#### 2.4.2 迁移警告

当检测到旧格式时，打印提示信息告知用户运行 `vdw migrate-config` 来更新配置格式。

### 2.5 运行时行为变更

#### 2.5.1 CLI 运行时

- `vdw link`：对于 editable 依赖，若 `local` 为空，创建 worktree 并填充；若 `local` 已存在且有效，直接使用。
- `vdw unlink`：清除 `local` 字段（设为空字符串）。
- `vdw doctor`：增加对 `local` 路径有效性检查（路径是否存在、是否为 Git 仓库）。

#### 2.5.2 Extension 运行时

- `AutoOpenService` 直接读取 `dep.local`，不再访问 `state.json`
- `WorkspaceTreeProvider` 中 `linkedRepos` 检测逻辑不变（依旧检查 `dep.path`）
- `addRepo.ts` 向导：选择本地仓库时，直接填充 `local` 为选定路径；选择远程仓库时，`local` 初始为空，由 `vdw link` 填充

## 3. 接口设计

### 3.1 Rust 侧 `Dependency` 结构体

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    /// Remote Git repository URL (e.g., git@github.com:org/repo.git)
    /// Empty string for local-only repos
    #[serde(default)]
    pub remote: String,

    /// Local repository path on disk (worktree or local git repo)
    /// This replaces the need for state.json worktree lookup
    #[serde(default)]
    pub local: String,

    /// Workspace link path (relative to workspace root)
    pub path: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    
    pub scope: Scope,
    
    #[serde(default = "default_editable")]
    pub editable: bool,
}
```

### 3.2 TypeScript 侧 `Dependency` 接口

```typescript
export interface Dependency {
    remote: string;   // Remote Git URL, empty string for local repos
    local: string;    // Local checkout/worktree path on disk
    path: string;     // Workspace link path (relative)
    branch?: string;
    tag?: string;
    commit?: string;
    scope: Scope;
    editable: boolean;
}
```

### 3.3 AutoOpenService 新流程

```typescript
async openAllEditableRepositories(): Promise<void> {
    const config = this.configService.getConfig();
    if (!config) return;
    if (!this.isAutoOpenEnabled()) return;

    const editableDeps = config.dependencies.filter(d => d.editable);
    const delay = this.getOpenDelay();

    for (const dep of editableDeps) {
        const repoPath = dep.local; // 直接从配置读取
        if (!repoPath) {
            log(`AutoOpen: ${dep.remote || dep.path} has no local path set`);
            continue;
        }
        if (this.isValidGitRepository(repoPath)) {
            try {
                await vscode.commands.executeCommand(
                    'git.openRepository',
                    vscode.Uri.file(repoPath)
                );
            } catch (err) {
                logError(`AutoOpen: Failed to open Git repository: ${repoPath}`, err);
            }
            await new Promise(resolve => setTimeout(resolve, delay));
        }
    }
}
```

## 4. 数据流与状态变更

### 4.1 配置生命周期

```
vdw init
  -> 创建 .vdwrc（无 dependencies）

vdw repo add <url> --editable
  -> clone_to_store()
  -> create_worktree()
  -> .vdwrc 写入 {remote, local, path, branch, editable}
  -> .vdwrc 更新到版本控制

vdw link
  -> 读取 .vdwrc
  -> 若 editable && local 为空：创建 worktree，填充 local
  -> 若 local 已存在：直接使用
  -> 创建硬链接/符号链接

vdw unlink
  -> 移除链接
  -> .vdwrc 清除 local 字段（设为空字符串）

extension 激活
  -> 解析 .vdwrc
  -> 读取 dep.local（无需 state.json）
  -> git.openRepository(local)
```

### 4.2 `state.json` 的定位变化

**保留内容**：
- `links[]`：链接条目信息（用于 `vdw unlink` 精确删除）

**移除内容**：
- `worktrees[]`：不再需要，worktree 路径已记录在 `.vdwrc` 中

## 5. 影响分析

### 5.1 需修改的文件清单

| 文件 | 修改类型 | 说明 |
|------|---------|------|
| `vdw/src/types/config.rs` | 重构 | `Dependency` 结构体：`repo` → `remote` + `local` |
| `vdw/src/config/parser.rs` | 增强 | 兼容旧格式 `repo` 字段 |
| `vdw/src/commands/link.rs` | 增强 | worktree 创建后写回 `.vdwrc` |
| `vdw/src/commands/repo.rs` | 增强 | `add` 后写 `local` |
| `vdw/src/types/workspace.rs` | 重构 | 移除 `WorktreeInfo`、`worktrees` 字段 |
| `vdw/src/worktree/manager.rs` | 无损 | 无需修改 |
| `vdw/README.md` | 更新 | 配置示例更新 |
| `vdw-vscode/src/types/vdwTypes.ts` | 重构 | `Dependency` 接口 |
| `vdw-vscode/src/services/autoOpenService.ts` | 简化 | 删除 `getWorktreePath()` |
| `vdw-vscode/src/services/configService.ts` | 适配 | `Dependency` 新字段 |
| `vdw-vscode/src/providers/workspaceTreeView.ts` | 适配 | `dep.remote` 替代 `dep.repo` |
| `vdw-vscode/src/commands/addRepo.ts` | 适配 | 新字段写入 |

### 5.2 需要调整的测试

| 测试文件 | 调整内容 |
|---------|---------|
| `vdw/src/types/config.rs` 测试 | `sample_dependency()` 更新为新结构；`test_parse_valid_yaml_config()` 更新测试 YAML |
| `vdw/src/config/parser.rs` 测试 | 增加旧格式兼容解析测试 |
| `vdw/src/types/workspace.rs` 测试 | `WorktreeInfo` 相关测试移除或调整 |
| `vdw-vscode/src/test/extension.test.ts` | `Dependency` 结构更新 |

## 6. 风险与注意事项

### 6.1 向后兼容风险

- **配置文件格式突变**：`repo` → `remote` + `local` 的变更需要兼容层，否则用户已有的 `.vdwrc` 会解析失败。
- **缓解方案**：在 `parser.rs` 中实现智能兼容：检测旧格式自动映射到新字段，并给出迁移提示。

### 6.2 state.json 存量数据

- 用户已有的 `state.json` 中包含 `worktrees` 数据，升级后这部分数据不再维护但不应影响功能。
- **缓解方案**：`AutoOpenService` 按"优先读 `.vdwrc` local，降级读 `state.json`"的优先级获取 worktree 路径，1-2 个版本后完全移除 `state.json` worktree 逻辑。

### 6.3 worktree 路径漂移

- 如果用户手动移动了 VDW store 目录，`.vdwrc` 中的 `local` 路径会失效。
- **缓解方案**：`vdw doctor` 命令增加 `local` 路径有效性检测，并提供修复建议。

## 7. 验收标准

### 7.1 功能验收

| # | 验收项 | 预期结果 |
|---|--------|---------|
| 1 | 新格式 `.vdwrc` 解析 | `vdw doctor` 正确解析 `remote` + `local` 字段 |
| 2 | 旧格式 `.vdwrc` 兼容 | 旧 `repo` 字段自动映射，不报错 |
| 3 | `vdw link` 写回 local | link 后 `.vdwrc` 中 dependencies.local 被正确填充 |
| 4 | AutoOpenService 简化 | 不读 `state.json`，直接从 `dep.local` 获取路径 |
| 5 | `addRepo.ts` 本地仓库 | 选择本地目录后，`.vdwrc` 写入 `remote: ''` + `local: <path>` |
| 6 | `addRepo.ts` 远程仓库 | 初始 `local` 为空，`vdw link` 后填充 |
| 7 | `vdw unlink` 清空 local | unlink 后 `.vdwrc` 的 `local` 字段被清空 |
| 8 | Extension 树视图 | 显示 `remote` 作为仓库 URL（若无则显示 `local`） |

### 7.2 代码验收

- Rust CLI 所有测试通过
- VS Code 扩展编译无错误
- `state.json` 中 `worktrees` 不再被写入（保留 `links`）

# VDW 多工作空间共享修复 - 产品需求文档

**文档版本**: v1.1  
**创建日期**: 2026-06-05  
**关联文档**: [VDW虚拟工作空间_需求文档_v1.2](./VDW虚拟工作空间_需求文档_v1.0.md)  
**优先级**: P0（影响数据隔离安全）

---

## 一、 问题概述

### 1.1 背景

VDW 使用全局内容存储池（`~/.vdw/store/`）实现跨工作空间的仓库去重。对于只读依赖，多个工作空间共享同一份裸仓库数据是安全的。但对于 **editable（可编辑）** 依赖，当前实现存在以下共享隔离问题：

### 1.2 问题场景

#### 场景 1：同一仓库同一分支，多工作空间同时 editable

```
项目A: repo-X, branch=main, editable  → worktrees/main/
项目B: repo-X, branch=main, editable  → worktrees/main/  （复用同一目录！）
```

**当前行为**：`link.rs` 检测到 `worktrees/main/` 已存在时直接复用，导致项目 A 和 B 的 editable 链接指向同一个 worktree 目录。两个工作空间的修改互相可见，无法隔离。

**期望行为**：项目 B 添加 repo-X 时，检测到已有工作空间以 editable 方式引用相同分支，只允许只读模式，并提示已有 editable 设定的工作空间目录。

#### 场景 3：同一仓库同一分支，同一工作空间重复声明

```
项目A: repo-X, branch=main, editable
项目A: repo-X, branch=main, editable  # 同一 .vdwrc 中重复声明
```

**当前行为**：`repo add` 命令的重复检测逻辑（`repo.rs` 第 78-89 行）仅在当前工作空间的 `.vdwrc` 内检查。但检测条件使用 `dep.remote` 或 `dep.local` 匹配，**不考虑 branch 和 editable 的组合**。在某些边界条件下（如 remote 为空但 local 不同时），重复条目可能被写入 `.vdwrc`，导致配置文件出现冗余。

**期望行为**：同一工作空间内，相同仓库 + 分支 + editable 的组合只能存在一条配置。重复添加时静默忽略（不写入 `.vdwrc`）。

---

## 二、 需求定义

### 2.1 F-WS1：全局 Editable 锁注册表

**优先级**: P0

**功能描述**：

新增全局配置文件 `~/.vdw/editable_locks.json`，记录当前系统中所有 editable 链接的归属信息。每次添加 editable 依赖时，先查询该注册表，如果同一仓库 + 分支已被其他工作空间锁定为 editable，则强制当前添加为只读模式。

**数据模型**：

```json
// ~/.vdw/editable_locks.json
{
  "version": 1,
  "locks": [
    {
      "repo_id": "repo-X",           // 仓库标识（remote URL 或 local path）
      "branch": "main",               // 分支名
      "workspace_path": "/Users/dev/project-a",  // 锁定该 editable 的工作空间路径
      "worktree_path": "/Users/tbsg/.vdw/store/objects/repo-X-abc123/bare.git/worktrees/main",  // VDW 管理的 worktree 绝对路径
      "created_at": "2026-06-05T10:00:00Z"
    }
  ]
}
```

**字段说明**：
- `workspace_path`：用于冲突检测时的提示信息（"已在 XX 工作空间 editable"）
- `worktree_path`：VDW 管理的实际 worktree 路径，**作为锁有效性的主要判据**（该路径由 VDW 控制，用户无法手动重命名）

**锁粒度**：`repo_id + branch` 组合唯一。同一仓库的不同分支可以分别在不同工作空间中 editable。

**锁生命周期**：

| 事件 | 操作 |
|------|------|
| `vdw repo add` (editable) | 检查锁 → 无冲突则写入锁 |
| `vdw link` (editable) | 检查锁 → 无冲突则写入锁 |
| `vdw repo remove` | 释放对应锁 |
| `vdw unlink` | 释放对应锁 |
| 工作空间路径不存在 | 根据 `worktree_path` 判断：worktree 仍存在则锁仍有效（用户可能重命名了工作空间目录）；worktree 也不存在则清理锁 |
| worktree 路径不存在 | 锁视为过期，自动清理 |

**验收标准**：
- [ ] `~/.vdw/editable_locks.json` 文件格式正确，可被 JSON 解析
- [ ] 添加 editable 依赖前检查锁注册表
- [ ] 锁冲突时输出提示信息，包含已有 editable 的工作空间路径
- [ ] 移除依赖或 unlink 时正确释放锁
- [ ] 过期锁（工作空间路径不存在）在下次操作时自动清理

---

### 2.2 F-WS2：跨工作空间 Editable 冲突检测与降级

**优先级**: P0

**功能描述**：

当用户尝试以 editable 模式添加一个已被其他工作空间锁定的仓库 + 分支时：

1. **CLI (`vdw repo add`)**：
   - 输出警告：`"repo-X branch 'main' is already editable in workspace: /Users/dev/project-a"`
   - 自动降级为只读模式（`editable: false`）
   - 继续正常完成添加流程

2. **VS Code 扩展（Add Repo 向导）**：
   - 在选择 editable/read-only 步骤前，先检查锁注册表
   - 如果存在冲突锁，**不显示 editable 选项**
   - 显示信息提示：`"This repository (branch: main) is already editable in workspace: /Users/dev/project-a. Only read-only mode is available."`
   - 自动选择只读模式

**锁检查逻辑（伪代码）**：

```rust
fn check_editable_lock(repo_id: &str, branch: &str, current_workspace: &Path) -> Result<Option<LockInfo>> {
    let locks = load_editable_locks()?;
    
    for lock in &locks {
        if lock.repo_id == repo_id && lock.branch == branch {
            // 同一工作空间内的 editable 不算冲突（允许 relink）
            if lock.workspace_path != current_workspace {
                // 以 worktree_path 为主要有效性判据（VDW 管理，用户无法重命名）
                if !Path::new(&lock.worktree_path).exists() {
                    // worktree 已不存在，锁过期，清理
                    remove_stale_lock(&lock)?;
                    continue;
                }
                return Ok(Some(lock.clone()));
            }
        }
    }
    Ok(None)
}
```

**验收标准**：
- [ ] CLI 模式下冲突时输出警告并自动降级为只读
- [ ] VS Code 扩展冲突时隐藏 editable 选项并显示提示
- [ ] 提示信息包含锁定该 editable 的工作空间路径
- [ ] 同一工作空间内的 relink 不触发冲突检测

---

### 2.3 F-WS3：同一工作空间重复声明过滤

**优先级**: P0

**功能描述**：

增强 `vdw repo add` 命令的重复检测逻辑。当同一 `.vdwrc` 中已存在相同 `repo_id + branch` 的依赖时，静默忽略重复添加（不写入配置文件，不创建新的 worktree 或链接）。

**当前实现分析**：

`repo.rs` 第 78-89 行的重复检测仅匹配 `dep.remote` 或 `dep.local`：

```rust
// 当前逻辑 - repo.rs
if !remote.is_empty() {
    if config.dependencies.iter().any(|dep| dep.remote == remote) {
        println!("Repository already exists in configuration");
        return Ok(());
    }
}
```

**改进方案**：

将重复检测条件扩展为 `repo_id + branch` 组合匹配：

```rust
fn is_duplicate(config: &VDWConfig, remote: &str, local: &str, branch: &Option<String>) -> bool {
    config.dependencies.iter().any(|dep| {
        let same_repo = if !remote.is_empty() {
            dep.remote == remote
        } else if !local.is_empty() {
            dep.local == local
        } else {
            false
        };
        let same_branch = dep.branch == *branch;
        same_repo && same_branch
    })
}
```

**行为**：

| 场景 | 行为 |
|------|------|
| 同 repo + 同 branch | 静默忽略，不写入 .vdwrc |
| 同 repo + 不同 branch | 允许添加（视为不同依赖） |
| 同 repo + 同 branch + 不同 editable | 静默忽略（以首次添加为准） |

**验收标准**：
- [ ] 重复添加时 `.vdwrc` 不出现冗余条目
- [ ] 不同分支的同名仓库可以正常添加
- [ ] `vdw link` 不创建重复的 worktree

---

## 三、 涉及修改的代码模块

### 3.1 CLI 端（Rust）

| 文件 | 修改内容 |
|------|---------|
| `vdw/src/types/config.rs` | 新增 `EditableLock`、`EditableLockRegistry` 结构体 |
| `vdw/src/store/manager.rs` | 新增 `editable_locks.json` 的读写、锁检查、锁清理方法 |
| `vdw/src/commands/repo.rs` | `repo add` 增加锁检查 + 重复声明过滤逻辑 |
| `vdw/src/commands/link.rs` | `link` 命令增加锁写入（成功创建 editable 后注册锁） |
| `vdw/src/commands/worktree.rs` | `worktree delete` 释放对应锁 |

### 3.2 VS Code 扩展端（TypeScript）

| 文件 | 修改内容 |
|------|---------|
| `vdw-vscode/src/commands/addRepo.ts` | 选择 editable 前读取锁注册表，冲突时隐藏选项 |
| `vdw-vscode/src/services/cliService.ts` | 新增读取 `~/.vdw/editable_locks.json` 的方法 |

### 3.3 新增文件

| 文件 | 说明 |
|------|------|
| `~/.vdw/editable_locks.json` | 全局 editable 锁注册表（运行时生成） |

---

## 四、 详细设计

### 4.1 锁注册表数据结构

```rust
// vdw/src/types/config.rs

#[derive(Debug, Serialize, Deserialize)]
pub struct EditableLockRegistry {
    pub version: u32,
    pub locks: Vec<EditableLock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableLock {
    pub repo_id: String,        // remote URL 或 local path
    pub branch: String,         // 分支名
    pub workspace_path: String, // 工作空间绝对路径（用于提示信息）
    pub worktree_path: String,  // VDW 管理的 worktree 路径（锁有效性主判据）
    pub created_at: String,     // ISO 8601 时间戳
}
```

### 4.2 锁操作方法

```rust
// vdw/src/store/manager.rs

impl StoreManager {
    /// 加载全局 editable 锁注册表
    pub fn load_editable_locks() -> Result<EditableLockRegistry>;
    
    /// 保存全局 editable 锁注册表
    pub fn save_editable_locks(registry: &EditableLockRegistry) -> Result<()>;
    
    /// 检查指定 repo+branch 是否已被其他工作空间锁定为 editable
    /// 返回 Some(lock) 表示冲突，None 表示无冲突
    pub fn check_editable_lock(
        repo_id: &str, 
        branch: &str, 
        current_workspace: &Path
    ) -> Result<Option<EditableLock>>;
    
    /// 注册一个新的 editable 锁
    pub fn acquire_editable_lock(
        repo_id: &str, 
        branch: &str, 
        workspace_path: &Path
    ) -> Result<()>;
    
    /// 释放指定工作空间的 editable 锁
    pub fn release_editable_lock(
        repo_id: &str, 
        branch: &str, 
        workspace_path: &Path
    ) -> Result<()>;
    
    /// 清理所有过期锁（工作空间路径不存在）
    pub fn cleanup_stale_locks() -> Result<Vec<EditableLock>>;
}
```

### 4.3 CLI 添加流程变更

```
vdw repo add <url> --branch <branch> --editable

1. 解析参数
2. 【新增】加载 editable_locks.json
3. 【新增】检查锁注册表
   ├─ 无冲突 → 继续
   └─ 有冲突 → 打印警告，自动降级为 editable=false
4. 【修改】重复检测：repo_id + branch 组合匹配
   ├─ 重复 → 静默忽略，return Ok(())
   └─ 不重复 → 继续
5. 更新 .vdwrc
6. 【新增】如果 editable=true，写入锁注册表
```

### 4.4 VS Code 扩展变更

```typescript
// addRepo.ts - 选择 editable 前
async function checkEditableLock(repoId: string, branch: string): Promise<EditableLock | null> {
    const locksPath = path.join(os.homedir(), '.vdw', 'editable_locks.json');
    if (!fs.existsSync(locksPath)) return null;
    
    const registry = JSON.parse(fs.readFileSync(locksPath, 'utf-8'));
    const currentWorkspace = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    
    for (const lock of registry.locks) {
        if (lock.repo_id === repoId && lock.branch === branch) {
            if (lock.workspace_path !== currentWorkspace) {
                // 以 worktree_path 为主要有效性判据
                if (!fs.existsSync(lock.worktree_path)) continue;
                return lock;
            }
        }
    }
    return null;
}

// 使用
const conflictLock = await checkEditableLock(repoUrl, branch);
if (conflictLock) {
    vscode.window.showInformationMessage(
        `This repository (branch: ${branch}) is already editable in workspace: ${conflictLock.workspace_path}. Only read-only mode is available.`
    );
    editable = false;  // 强制只读
} else {
    // 正常显示 editable 选项
    const editableChoice = await vscode.window.showQuickPick(...);
}
```

---

## 五、 风险与应对

| 风险 | 影响 | 概率 | 应对策略 |
|------|------|------|---------|
| **过期锁未清理** | 中 | 高 | 以 `worktree_path`（VDW 管理）为主要有效性判据，而非 `workspace_path`（用户可重命名）；worktree 不存在时自动清理锁 |
| **用户重命名工作空间目录** | 中 | 中 | `workspace_path` 失效但 `worktree_path` 仍有效，锁不会被误清理；冲突提示信息中显示旧路径，用户可通过 `vdw doctor` 修复 |
| **并发写入锁文件** | 中 | 低 | 使用文件锁（`fs2::FileExt::lock_exclusive`）或 `.lock` 文件互斥 |
| **向后兼容** | 中 | 中 | 首次运行时，扫描所有已知工作空间的 `.vdw/state.json`，自动填充锁注册表 |
| **锁文件损坏** | 低 | 低 | JSON 解析失败时自动重建空注册表，日志警告 |
| **用户手动移动工作空间** | 中 | 低 | 以 `worktree_path` 为有效性判据（VDW 管理，不受工作空间重命名影响）；`workspace_path` 仅用于提示信息 |
| **workspace_path 与实际不符** | 低 | 中 | 提供 `vdw doctor --fix` 命令扫描并更新锁注册表中的 workspace_path |

---

## 六、 验收测试用例

| # | 测试场景 | 操作步骤 | 预期结果 |
|---|---------|---------|---------|
| 1 | 跨工作空间 editable 冲突 | 项目 A 添加 repo-X main editable → 项目 B 添加 repo-X main editable | B 自动降级为只读，提示 A 的路径 |
| 2 | 同仓库不同分支 | 项目 A 添加 repo-X main editable → 项目 B 添加 repo-X dev editable | 两者都成功，各自 editable |
| 3 | 同一工作空间重复添加 | 项目 A 添加 repo-X main editable → 项目 A 再次添加 repo-X main editable | 静默忽略，.vdwrc 无冗余 |
| 4 | 锁释放 | 项目 A 移除 repo-X → 项目 B 添加 repo-X main editable | B 成功获取 editable |
| 5 | 工作空间重命名后锁有效性 | 手动重命名项目 A 目录 → 项目 B 尝试添加 repo-X main editable | 锁仍有效（worktree_path 存在），B 降级为只读 |
| 6 | 过期锁清理（worktree 已删除） | 项目 A unlink repo-X → 项目 B 添加 repo-X main editable | 自动清理过期锁，B 成功获取 editable |
| 7 | VS Code 冲突提示 | 项目 A 已有 editable → 项目 B 在 VS Code 中添加同仓库 | 不显示 editable 选项，显示信息提示 |
| 8 | 并发安全 | 两个终端同时 `vdw repo add --editable` | 只有一个成功获取锁，另一个降级为只读 |

---

## 七、 实施计划

### Task 1：新增锁注册表数据结构与读写方法

- 文件：`types/config.rs`、`store/manager.rs`
- 内容：`EditableLock`、`EditableLockRegistry` 结构体，锁文件 CRUD 操作
- 预计改动：~100 行

### Task 2：CLI `repo add` 集成锁检查与重复过滤

- 文件：`commands/repo.rs`
- 内容：添加锁检查、重复声明过滤（repo_id + branch）、冲突降级
- 预计改动：~50 行

### Task 3：CLI `link`/`unlink`/`worktree delete` 集成锁写入与释放

- 文件：`commands/link.rs`、`commands/worktree.rs`
- 内容：link 成功后写锁，unlink/delete 后释放锁
- 预计改动：~30 行

### Task 4：VS Code 扩展集成锁检查

- 文件：`addRepo.ts`、`cliService.ts`
- 内容：读取锁注册表，冲突时隐藏 editable 选项
- 预计改动：~40 行

### Task 5：过期锁清理与向后兼容

- 文件：`store/manager.rs`
- 内容：锁检查时自动验证路径，首次运行扫描现有状态
- 预计改动：~30 行

---

## 八、 变更记录

| 版本 | 日期 | 变更内容 | 作者 |
|------|------|---------|------|
| v1.0 | 2026-06-05 | 初始版本：多工作空间共享修复（场景 1、场景 3） | AI Agent |
| v1.1 | 2026-06-05 | 新增 `worktree_path` 字段作为锁有效性主判据，解决用户重命名工作空间目录导致锁误清理的问题；新增 `vdw doctor --fix` 修复方案 | AI Agent |

---

**文档结束**

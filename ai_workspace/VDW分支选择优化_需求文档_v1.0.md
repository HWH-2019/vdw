# VDW分支选择优化_需求文档_v1.0

---

## 1. 改动背景

### 1.1 现状分析

VDW VS Code 插件的"新增仓库托管"流程中，分支选择使用 `vscode.window.showInputBox` 自由文本输入，存在以下问题：

1. **远程仓库无检测**：分支默认硬编码为 `'main'`，不调用任何 Git 命令检测远程仓库的默认分支或可用分支列表
2. **本地仓库检测粗糙**：使用内联 `require('child_process')` 执行 `git branch --show-current`，仅获取当前分支，不列出所有可用分支
3. **无分支验证**：用户输入不存在的分支名时，系统静默接受，直到 `vdw link` 阶段才报错
4. **detached HEAD 处理缺失**：本地仓库处于 detached HEAD 状态时，`git branch --show-current` 返回空字符串，静默回退到 `'main'`
5. **代码风格不一致**：本地分支检测使用内联 `require('child_process')`，与项目中已有的 `spawn` 模式（vdwCliService.ts）不一致

### 1.2 业务诉求

- 将分支输入框改为可搜索的选择框，展示仓库中实际存在的分支
- 支持模糊匹配筛选，方便在大量分支中快速定位
- 对用户输入的不存在分支名进行二次确认，避免配置错误
- 修复现有分支检测的 Bug

---

## 2. 改动目标

### 2.1 核心目标

- 将分支选择从 `showInputBox`（自由文本）改为 `showQuickPick`（可搜索选择框）
- 本地仓库：先 `git fetch` 更新远程追踪分支，再列出所有分支
- 远程仓库：通过 `git ls-remote --heads` 获取远程分支列表
- 默认展示前 10 个分支，用户输入时实时模糊匹配并展示匹配的前 10 个
- 用户输入不存在的分支名时，弹出模态确认框进行二次确认
- 修复 4 个已知 Bug（B1-B4）

### 2.2 非目标

- 不修改 CLI 侧 Rust 代码
- 不修改 `vdw repo add` 的 CLI 接口
- 不改变现有的 editable/scope 选择流程

---

## 3. 功能需求

### 3.1 分支列表获取

#### 3.1.1 本地仓库分支获取

**触发时机**：用户选择"Local Repository"并选定本地路径后，在展示分支选择框之前执行。

**执行步骤**：

1. 对选定的本地仓库执行 `git fetch --all`，更新远程追踪分支
   - 超时：15 秒
   - 失败处理：记录日志但不阻塞，继续使用本地已有分支
2. 执行 `git branch -a --format='%(refname:short)'` 获取所有分支（本地 + 远程追踪）
   - 本地分支格式：`main`、`develop`
   - 远程追踪分支格式：`origin/main`、`origin/develop`
3. 去除 `origin/` 前缀的远程追踪分支（与本地分支去重）
4. 检测当前分支（`git branch --show-current`），标记为默认选中项
5. 若当前处于 detached HEAD 状态（`--show-current` 为空），使用 `git rev-parse --abbrev-ref HEAD` 获取 commit hash 作为描述

**输出**：`BranchItem[]` 数组，每项包含：

```typescript
interface BranchItem {
    label: string;          // 分支名，如 "main"
    description: string;    // 标记，如 "$(star-full) Current" 或 "remote"
    detail?: string;        // 额外信息，如 "origin/main"
}
```

#### 3.1.2 远程仓库分支获取

**触发时机**：用户选择"Remote Repository"并输入 URL 后，在展示分支选择框之前执行。

**执行步骤**：

1. 执行 `git ls-remote --heads <url>` 获取远程分支列表
   - 超时：15 秒
   - 失败处理：回退到自由文本输入模式，显示警告消息"无法获取远程分支列表，请手动输入"
2. 解析输出：每行格式为 `<commit-hash>\trefs/heads/<branch-name>`
3. 提取分支名并排序
4. 尝试检测默认分支：执行 `git ls-remote --symref <url> HEAD`，解析 `ref: refs/heads/<name>` 获取默认分支名

**输出**：同 `BranchItem[]`，默认分支标记为 `$(star-full) Default`。

#### 3.1.3 性能要求

- 分支获取使用 `withProgress` 显示加载状态（位置：`ProgressLocation.Notification`）
- 加载提示文案：`"Fetching branches..."`
- 超时上限：15 秒，超时后使用已获取的数据（可能为空）

### 3.2 分支选择框

#### 3.2.1 交互设计

将 `showInputBox` 替换为 `showQuickPick`，配置如下：

```typescript
vscode.window.showQuickPick(branchItems, {
    placeHolder: 'Select or type a branch name',
    matchOnDescription: true,
    matchOnDetail: true,
    canPickMany: false,
});
```

#### 3.2.2 展示规则

- 初始展示：显示前 10 个分支（按字母序排列），默认/当前分支置顶
- 模糊匹配：用户输入文本时，VS Code 原生 `showQuickPick` 自动进行模糊过滤
- 匹配结果上限：最多展示匹配的前 10 个结果

#### 3.2.3 默认选中

- 本地仓库：当前分支（`$(star-full) Current` 标记）为默认选中
- 远程仓库：默认分支（`$(star-full) Default` 标记）为默认选中；若无法检测默认分支，以 `main` 或 `master` 为默认

### 3.3 不存在分支的二次确认

#### 3.3.1 方案选择

**主方案（方案 A）：模态确认框**

当用户在 QuickPick 中输入了一个不在列表中的分支名并按 Enter 确认时：

1. 弹出 `vscode.window.showWarningMessage` 模态对话框：
   - 文案：`"Branch 'xxx' does not exist in the repository. A new branch will be created. Continue?"`
   - 按钮：`"Yes, Create New Branch"` / `"Cancel"`
2. 用户点击 "Yes" → 使用输入的分支名继续流程
3. 用户点击 "Cancel" → 返回分支选择框

**备选方案 B：内联创建选项**

在 QuickPick 列表底部添加特殊选项：`"+ Create new branch: <user-input>"`，点击即视为确认。

**备选方案 C：验证拒绝 + 重新输入**

在 QuickPick 的 `validateInput` 中检测输入是否匹配，不匹配时显示黄色警告文本，用户需按两次 Enter 确认。

#### 3.3.2 实现要点

```typescript
// 方案 A 伪代码
const selected = await vscode.window.showQuickPick(branchItems, {
    placeHolder: 'Select or type a branch name',
    matchOnDescription: true,
});

if (selected) {
    branch = selected.label;
} else {
    // User typed custom value and pressed Enter
    const customBranch = /* get typed value */;
    const confirm = await vscode.window.showWarningMessage(
        `Branch '${customBranch}' does not exist. Create new branch?`,
        { modal: true },
        'Yes, Create New Branch'
    );
    if (confirm === 'Yes, Create New Branch') {
        branch = customBranch;
    } else {
        return false; // cancelled
    }
}
```

> **注意**：VS Code 的 `showQuickPick` 不直接支持获取用户输入的自定义文本。需要使用以下模式之一：
> - 在 items 列表中动态添加 "Create new branch: ..." 选项
> - 或使用 `createInputBox()` 自定义 UI 组件
> - 或先 `showQuickPick`，若取消则 fallback 到 `showInputBox` 并执行验证

### 3.4 Bug 修复

#### B1：远程仓库默认分支检测缺失

**现状**：远程仓库分支硬编码为 `'main'`（addRepo.ts L42）

**修复**：通过 `git ls-remote --symref <url> HEAD` 检测默认分支。若检测失败，按优先级尝试 `main` → `master`。

#### B2：本地分支检测代码风格不一致

**现状**：使用内联 `require('child_process')`（addRepo.ts L113）

**修复**：提取为独立的工具函数 `getGitBranches(repoPath: string): Promise<BranchItem[]>`，使用 `child_process.exec` 的 Promise 封装（与项目其他部分一致）。

#### B3：无分支名验证

**现状**：任意文本输入均被接受（addRepo.ts L148-153）

**修复**：通过 3.3 节的二次确认流程，对不在列表中的分支名进行确认。

#### B4：detached HEAD 状态处理缺失

**现状**：`git branch --show-current` 返回空时静默回退到 `'main'`（addRepo.ts L114）

**修复**：当 `--show-current` 返回空时，使用 `git rev-parse --short HEAD` 获取当前 commit hash 作为描述，并在分支列表中标记为 `"detached at <hash>"`。

---

## 4. 技术方案

### 4.1 新增文件/函数

#### `vdw-vscode/src/utils/git.ts`（新建）

```typescript
import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

export interface BranchItem {
    label: string;
    description: string;
    detail?: string;
}

/**
 * 获取本地仓库的所有分支（含远程追踪分支）
 * 先执行 git fetch --all 更新远程追踪分支
 */
export async function getLocalBranches(repoPath: string): Promise<BranchItem[]> {
    // 1. git fetch --all (best-effort, 15s timeout)
    // 2. git branch -a --format='%(refname:short)'
    // 3. 去重 + 标记当前分支
    // 4. 返回 BranchItem[]
}

/**
 * 获取远程仓库的分支列表
 * 通过 git ls-remote --heads 获取
 */
export async function getRemoteBranches(repoUrl: string): Promise<BranchItem[]> {
    // 1. git ls-remote --heads <url> (15s timeout)
    // 2. 解析输出提取分支名
    // 3. git ls-remote --symref <url> HEAD 检测默认分支
    // 4. 返回 BranchItem[]
}

/**
 * 获取当前分支名，detached HEAD 返回 commit hash
 */
export async function getCurrentBranch(repoPath: string): Promise<{
    branch: string;
    isDetached: boolean;
}> {
    // git branch --show-current
    // 若为空 → git rev-parse --short HEAD
}
```

### 4.2 修改文件

#### `vdw-vscode/src/commands/addRepo.ts`

**修改范围**：

1. 删除内联 `require('child_process')` 和 `git branch --show-current` 调用（L111-117）
2. 本地仓库流程中，选择路径后调用 `getLocalBranches(selectedPath)` 获取分支列表
3. 远程仓库流程中，输入 URL 后调用 `getRemoteBranches(urlInput)` 获取分支列表
4. 将分支 `showInputBox`（L143-160）替换为 `showQuickPick` + 二次确认逻辑
5. 使用 `vscode.window.withProgress` 包裹分支获取过程

### 4.3 执行流程图

```
用户点击 "Add Repository"
        │
        ▼
  选择仓库类型 (Remote/Local)
        │
   ┌────┴────┐
   ▼         ▼
 Remote     Local
   │         │
 输入 URL   选择目录
   │         │
   ▼         ▼
 getRemoteBranches()   getLocalBranches()
 (git ls-remote)       (git fetch + git branch -a)
   │         │
   ▼         ▼
 分支列表获取完成
   │         │
   └────┬────┘
        ▼
 showQuickPick (分支选择框)
 ├─ 显示前 10 个分支
 ├─ 支持模糊搜索
 └─ 默认/当前分支置顶
        │
        ▼
  用户选择的分支是否存在于列表中？
   ├─ Yes → 使用该分支名
   └─ No  → 弹出模态确认框
            ├─ "Yes, Create" → 使用该分支名
            └─ "Cancel" → 返回选择框
        │
        ▼
 继续后续流程 (editable / scope / CLI 执行)
```

---

## 5. 数据模型

### 5.1 BranchItem 接口

```typescript
export interface BranchItem extends vscode.QuickPickItem {
    label: string;          // 分支名
    description: string;    // "$(star-full) Current" | "$(star-full) Default" | "remote" | ""
    detail?: string;        // "origin/main" (原始远程追踪分支名)
}
```

### 5.2 无新增数据模型

本需求不新增数据库表或持久化数据模型，所有数据均为运行时临时获取。

---

## 6. 测试要点

### 6.1 功能测试

| 场景 | 预期行为 |
|------|---------|
| 本地仓库，正常分支 | 显示分支列表，当前分支默认选中 |
| 本地仓库，detached HEAD | 显示分支列表 + "detached at <hash>" 描述 |
| 本地仓库，git fetch 失败 | 不阻塞，使用本地已有分支列表 |
| 远程仓库，正常分支 | 显示远程分支列表，默认分支标记并选中 |
| 远程仓库，SSH 认证失败 | 回退到自由文本输入，显示警告 |
| 远程仓库，超时（>15s） | 回退到自由文本输入 |
| 输入不存在的分支名 | 弹出模态确认框 |
| 确认后创建新分支 | 流程正常继续，CLI 使用该分支名 |
| 取消确认 | 返回分支选择框 |
| 分支数 > 10 | 初始显示前 10 个，输入筛选后显示匹配的前 10 个 |

### 6.2 异常测试

| 场景 | 预期行为 |
|------|---------|
| 空仓库（无任何分支） | 显示空列表，允许用户手动输入分支名 |
| 非 Git 目录（本地仓库） | 显示错误提示，不允许继续 |
| 网络断开（远程仓库） | 15s 超时后回退到自由文本 |

---

## 7. 版本与影响范围

- **影响文件**：`addRepo.ts`（修改）、`utils/git.ts`（新建）
- **CLI 变更**：无
- **配置变更**：无
- **版本号**：vdw-vscode 插件版本从当前版本递增 patch 版本号

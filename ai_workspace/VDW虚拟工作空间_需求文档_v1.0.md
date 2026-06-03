# VDW (Virtual Dev Workspace) - 产品需求文档

**文档版本**: v1.0  
**创建日期**: 2026-06-02  
**产品名称**: Virtual Dev Workspace (VDW)  
**产品定位**: 跨仓库全栈开发中间层 - 物理去重存储 + 逻辑工作空间 + AI 上下文智能管理

---

## 一、 产品概述

### 1.1 背景与痛点

在全栈开发场景中，开发者经常需要同时操作多个物理隔离的代码仓库。当前的开发工具和工作流存在以下核心痛点：

1. **代码冗余**：IDE 仅能打开同目录下的文件，当需要关联跨目录仓库时，必须手动拉取到同一目录下，导致本地多份代码冗余
2. **AI 上下文爆炸**：多仓库全量索引导致 AI Agent 上下文窗口溢出，影响代码建议质量和响应速度
3. **依赖管理混乱**：开发过程中发现需要改动额外仓库时，缺乏系统化的依赖声明和版本管理机制
4. **工作空间割裂**：物理隔离的仓库无法提供统一的开发视图，降低开发效率

### 1.2 产品愿景

借鉴 `pnpm` 的内容寻址存储和硬链接机制，构建一个**逻辑工作空间层（Logical Workspace Layer）**，实现：
- **物理层**：全局去重存储，相同内容只存一份
- **逻辑层**：按需组装的开发工作空间，提供统一的文件树视图
- **AI 层**：智能上下文裁剪，精准投喂相关代码片段

### 1.3 目标用户

| 用户类型 | 使用场景 | 核心价值 |
|---------|---------|---------|
| **个人全栈开发者** | 同时开发前端、后端、移动端等多个仓库 | 减少磁盘占用，统一开发视图，提升 AI 编码效率 |
| **研发团队** | 多服务协作开发，微服务架构下的跨服务调试 | 标准化依赖管理，统一开发环境，降低新人上手成本 |

---

## 二、 核心功能需求

### 2.1 功能架构图

```
┌─────────────────────────────────────────────────────────┐
│                    IDE Bridge (VS Code 插件)              │
│  - 逻辑工作空间视图  - 上下文文件列表  - 一键添加依赖     │
└──────────────────────┬──────────────────────────────────┘
                       │ LSP Proxy / File Provider API
┌──────────────────────▼──────────────────────────────────┐
│              AI Context Filter (上下文管理器)             │
│  - AST 分析  - 调用图追踪  - 增量索引  - Scope 过滤      │
└──────────────────────┬──────────────────────────────────┘
                       │ Virtual File List API
┌──────────────────────▼──────────────────────────────────┐
│          Workspace Orchestrator (工作空间编排器)          │
│  - .vdwrc 解析  - 依赖图构建  - DAG 检测  - 动态挂载     │
└──────────────┬──────────────────────┬───────────────────┘
               │                      │
    ┌──────────▼──────────┐  ┌────────▼──────────┐
    │   Link Engine       │  │  Repo Registry    │
    │  (链接引擎)          │  │  (仓库注册表)      │
    │ - Hard Link         │  │ - 元数据管理       │
    │ - Symlink           │  │ - 版本依赖图       │
    │ - Junction (Win)    │  │ - 分支/Commit 跟踪 │
    └──────────┬──────────┘  └────────┬──────────┘
               │                      │
    ┌──────────▼──────────────────────▼──────────┐
    │          Content Store (内容存储池)          │
    │  - CAS (Content-Addressable Storage)        │
    │  - Git Object Sharing                       │
    │  - Blob Hashing & Deduplication             │
    └─────────────────────────────────────────────┘
```

### 2.2 核心功能清单

#### F1: 工作空间配置管理 (.vdwrc)

**优先级**: P0 (MVP 必须)

**功能描述**：
- 提供标准化的 YAML/JSON 配置文件格式，声明当前工作空间依赖的远程仓库
- 支持细粒度的 Scope 定义，控制哪些目录/文件暴露给 AI 索引
- 支持分支、Tag、Commit 级别的版本锁定

**配置示例**：
```yaml
workspace:
  name: "fullstack-app"
  version: "1.0.0"
  
dependencies:
  - repo: "git@github.com:org/frontend.git"
    path: "./apps/web"
    branch: "feat/new-ui"
    scope: 
      include: ["src/components/**", "src/hooks/**", "package.json"]
      exclude: ["node_modules", "dist", "*.test.ts"]
    editable: true  # 是否允许编辑
    
  - repo: "git@github.com:org/backend.git"
    path: "./services/api"
    branch: "main"
    commit: "abc123def"  # 锁定到特定 commit
    scope:
      include: ["**/*.proto", "src/controllers/**", "src/models/**"]
    editable: false  # 只读引用
    
  - repo: "git@github.com:org/shared-lib.git"
    path: "./packages/shared"
    tag: "v2.1.0"
    scope:
      include: ["**/*.ts"]
    editable: true
```

**验收标准**：
- [ ] 支持 YAML 和 JSON 两种格式
- [ ] 配置文件校验：repo URL 格式、path 唯一性、scope 语法合法性
- [ ] 支持环境变量引用（如 `${GITHUB_TOKEN}`）
- [ ] 提供 `vdw init` 命令交互式生成配置文件

---

#### F2: 内容存储与去重 (Content Store)

**优先级**: P0 (MVP 必须)

**功能描述**：
- 实现全局去重存储池，相同文件内容只存储一份
- 基于 Git Object Store 共享机制，而非文件级硬链接（更安全）
- 使用 Content-Addressable Storage (CAS) 策略，以 Blob Hash 为键存储

**技术实现**：
```
~/.vdw/
├── store/
│   ├── objects/          # Git 对象共享池
│   │   ├── <hash1>/      # 基于 commit hash 的目录
│   │   └── <hash2>/
│   └── cache/            # 元数据缓存
├── config.json           # 全局配置
└── lock                  # 并发锁文件
```

**核心机制**：
1. **Git Clone --reference**：新仓库克隆时引用全局对象池，避免重复下载 Git 对象
2. **Git Alternates**：通过 `.git/objects/info/alternates` 文件指向共享对象库
3. **惰性拉取**：仅拉取 `.vdwrc` 中声明的分支/commit，而非完整仓库历史

**验收标准**：
- [ ] 相同仓库的不同分支共享 Git 对象，磁盘占用减少 60%+
- [ ] 支持并发安全（多 vdw 实例同时操作时不冲突）
- [ ] 提供 `vdw store clean` 命令清理未引用的对象
- [ ] Store 损坏时可自动重建（从 remote 重新拉取）

---

#### F3: 链接引擎与逻辑视图构建 (Link Engine)

**优先级**: P0 (MVP 必须)

**功能描述**：
- 根据 `.vdwrc` 配置，将 Content Store 中的仓库映射到本地逻辑路径
- 跨平台兼容：macOS/Linux 使用 Hard Link，Windows 使用 Junction Points
- 提供类 Monorepo 的统一文件树视图

**链接策略**：

| 平台 | 首选方案 | 降级方案 | 备注 |
|------|---------|---------|------|
| macOS/Linux | Hard Link | Symlink | 同分区优先 Hard Link |
| Windows | Junction Point | Symlink | 需要开发者模式或管理员权限 |
| 跨分区 | Symlink | Reflink (CoW) | 依赖文件系统支持 |

**工作流程**：
```
1. 解析 .vdwrc → 生成依赖列表
2. 检查 Content Store 是否有所需 commit
   ├─ 有 → 跳到步骤 4
   └─ 无 → 从 remote 拉取到 Store（使用 --reference）
3. 在本地工作空间创建逻辑目录结构
4. 创建链接（Hard Link / Symlink / Junction）
5. 初始化 .git 目录（如果需要 editable 模式）
6. 生成虚拟文件树视图
```

**验收标准**：
- [ ] `vdw link` 命令在 10 秒内完成 5 个仓库的链接（每个仓库 < 500MB）
- [ ] 链接后的目录可被 IDE 正常打开和索引
- [ ] 支持 `vdw unlink` 命令安全解除链接（不删除源码）
- [ ] 链接失败时提供清晰的错误信息和恢复建议

---

#### F4: Git Worktree 集成 (Editable 模式)

**优先级**: P0 (MVP 必须)

**功能描述**：
- 对于标记为 `editable: true` 的仓库，使用 Git Worktree 提供可编辑的工作副本
- 每个 Worktree 共享同一个 Git 对象库，节省磁盘空间
- 独立的 HEAD、index、working tree，支持正常的 git add/commit/push 流程

**Worktree 结构**：
```
~/.vdw/store/<repo-hash>/
├── bare.git/             # 裸仓库（对象库）
└── worktrees/
    ├── main/             # main 分支的 worktree
    ├── feat-xxx/         # 特性分支的 worktree
    └── dev/              # 开发分支的 worktree

项目目录/
├── .vdwrc
├── apps/web -> ~/.vdw/store/<hash>/worktrees/feat-new-ui  # Symlink
└── services/api -> ~/.vdw/store/<hash2>/worktrees/main    # Symlink
```

**验收标准**：
- [ ] Worktree 支持所有标准 Git 操作（checkout、commit、push、pull）
- [ ] 多个 Worktree 可同时检出不同分支，互不干扰
- [ ] `vdw worktree list` 命令显示所有活跃 Worktree 状态
- [ ] Worktree 删除时自动清理关联的链接

---

#### F5: AI 上下文过滤器 (AI Context Filter)

**优先级**: P0 (MVP 必须)

**功能描述**：
- 根据 `.vdwrc` 中的 `scope` 配置，生成虚拟文件列表供 AI Agent 使用
- 提供 VS Code 插件的 `FileProvider` 接口，拦截 AI 插件的文件请求
- 支持基于 AST 的依赖分析，动态扩展上下文（可选，MVP 后可迭代）

**MVP 实现**：
```typescript
// VS Code 插件伪代码
class VDWFileProvider implements vscode.TextDocumentContentProvider {
  async provideTextDocumentContent(uri: vscode.Uri): Promise<string> {
    // 1. 检查文件是否在 .vdwrc 的 scope 内
    if (!isInScope(uri.path)) {
      return null; // 不暴露给 AI
    }
    
    // 2. 读取实际文件内容（通过链接访问）
    const content = await readFile(resolveVDWPath(uri.path));
    
    // 3. 返回给 AI Agent
    return content;
  }
  
  async listFiles(): Promise<string[]> {
    // 返回所有在 scope 内的文件列表
    return getScopedFiles();
  }
}
```

**上下文注入策略**：
```
用户打开: apps/web/src/components/Button.tsx

VDW 返回的上下文文件:
✅ apps/web/src/components/Button.tsx      # 当前编辑文件
✅ apps/web/src/components/Button.types.ts # 同目录类型定义
✅ packages/shared/types/index.ts          # .vdwrc scope 包含的共享库
✅ services/api/src/controllers/user.ts    # .vdwrc scope 包含的 API 定义

❌ apps/web/node_modules/**                # scope exclude
❌ services/api/src/tests/**               # scope exclude
❌ packages/shared/scripts/**              # scope exclude
```

**验收标准**：
- [ ] VS Code 插件注册自定义 `FileProvider`，AI 插件（如 Copilot）可正常访问
- [ ] 文件列表过滤准确率 100%（不在 scope 内的文件绝不暴露）
- [ ] 上下文文件列表生成时间 < 500ms（1000 文件规模）
- [ ] 提供 `vdw context preview` 命令预览当前上下文包含的文件

---

#### F6: VS Code 插件 (IDE Bridge)

**优先级**: P0 (MVP 必须)

**功能描述**：
- 在 VS Code 侧边栏显示"逻辑工作空间"视图
- 状态栏显示当前 AI 上下文包含的仓库列表和文件数量
- 一键添加远程仓库到当前工作空间（自动更新 `.vdwrc`）
- 提供命令面板操作：`VDW: Link`、`VDW: Unlink`、`VDW: Refresh Context`
- **自动打开 Git 仓库**：在 add、link、relink 操作后自动在 VS Code Source Control 中注册可编辑依赖的 Git 仓库

**自动打开 Git 仓库功能详解**：

**触发时机**：
1. **扩展激活时**：VS Code 打开工作区时，自动检测 `.vdwrc` 并打开所有 `editable: true` 的仓库
2. **vdw.link 成功后**：链接操作完成后，自动打开新增的可编辑仓库
3. **vdw.relink 成功后**：重新链接操作完成后，自动刷新 Git 仓库注册
4. **vdw.addRepo 成功后**：添加新仓库后，如果标记为 `editable: true`，自动打开其 Git 仓库

**实现机制**：
```typescript
// 从 .vdw/state.json 读取 worktree 路径
async function getWorktreePath(dep: Dependency): Promise<string | null> {
    const statePath = path.join(workspacePath, '.vdw', 'state.json');
    const state = JSON.parse(fs.readFileSync(statePath, 'utf-8'));
    const worktree = state.worktrees?.find(
        (wt: any) => wt.repo_url === dep.repo && wt.branch === dep.branch
    );
    return worktree?.path || null;
}

// 自动打开 Git 仓库（触发 VS Code 原生 Source Control 面板显示）
async function autoOpenGitRepositories(configService: ConfigService): Promise<void> {
    const config = configService.getConfig();
    
    for (const dep of config.dependencies) {
        if (dep.editable) {
            const worktreePath = await getWorktreePath(dep);
            if (worktreePath && fs.existsSync(worktreePath)) {
                try {
                    // 调用 Git 扩展的 API 触发原生 Source Control 面板显示该仓库
                    await vscode.commands.executeCommand(
                        'git.openRepository',
                        vscode.Uri.file(worktreePath)
                    );
                    log(`Auto-opened Git repository: ${worktreePath}`);
                } catch (err) {
                    logError(`Failed to auto-open Git repository: ${worktreePath}`, err);
                    // 不阻断流程
                }
                await new Promise(resolve => setTimeout(resolve, 500));
            }
        }
    }
}
```

**潜在问题与解决方案**：

| 问题 | 原因 | 解决方案 |
|------|------|---------||
| **Git 扩展未激活** | VS Code 启动时 Git 扩展可能延迟加载 | `git.openRepository` 命令自动触发 Git 扩展激活 |
| **多个仓库并发打开冲突** | 快速调用 `git.openRepository` 可能导致状态冲突 | 添加 500ms 延迟，串行打开仓库 |
| **worktree 路径不存在** | link 操作失败或 state.json 未更新 | 添加路径存在性检查，失败时记录日志但不阻断流程 |
| **用户不想自动打开** | 某些场景下用户希望手动控制 | 提供配置项 `vdw.autoOpenGitRepositories` (默认 true) |
| **Git 扩展 API 变更** | VS Code 更新可能导致 API 不兼容 | 添加 try-catch 错误处理，降级为手动提示 |

**用户配置**：
```json
// VS Code settings.json
{
  "vdw.autoOpenGitRepositories": true,  // 是否自动打开 Git 仓库（默认 true）
  "vdw.autoOpenDelay": 500              // 仓库间打开延迟（毫秒，默认 500）
}
```

**UI 设计**：
```
┌─ 资源管理器 ────────────────────────────┐
│ 📂 apps                                │
│   📂 web                               │
│     📂 src                             │
│       📂 components                    │
│         📄 Button.tsx                  │
│                                         │
│ 📂 services                            │
│   📂 api                               │
│     📂 src                             │
│       📂 controllers                   │
│         📄 user.ts                     │
│                                         │
│ 📂 packages                            │
│   📂 shared                            │
│     📂 types                           │
│       📄 index.ts                      │
└─────────────────────────────────────────┘

┌─ VDW 工作空间 ─────────────────────────┐
│ 🔗 frontend.git@feat/new-ui  [可编辑]   │
│    路径: ./apps/web                    │
│    Scope: src/components, src/hooks    │
│    状态: ✅ 已链接                      │
│                                         │
│ 🔗 backend.git@main          [只读]     │
│    路径: ./services/api                │
│    Scope: **/*.proto, src/controllers  │
│    状态: ✅ 已链接                      │
│                                         │
│ [+ 添加仓库]  [🔄 刷新]                 │
└─────────────────────────────────────────┘

┌─ 源代码管理 ───────────────────────────┐
│ 📦 vdw-test (main)                     │  ← 主仓库
│                                         │
│ 📦 frontend (feat/new-ui)              │  ← 自动打开的子仓库
│    Changes: 3 files modified           │
│    [✓ Commit] [↑ Push]                 │
│                                         │
│  shared-lib (main)                   │  ← 自动打开的子仓库
│    No changes                          │
└─────────────────────────────────────────┘

状态栏: [🤖 AI 上下文: 2 个仓库 | 156 文件]
```

**验收标准**：
- [ ] 插件安装后自动检测 `.vdwrc` 文件并激活
- [ ] 侧边栏视图实时更新链接状态
- [ ] 命令面板操作有明确的成功/失败反馈
- [ ] 与 VS Code 原生 Git 集成兼容（不冲突）
- [ ] **自动打开 Git 仓库功能**：
  - [ ] 扩展激活时自动打开所有 `editable: true` 的仓库
  - [ ] `vdw.link` 成功后自动打开新增的可编辑仓库
  - [ ] `vdw.relink` 成功后自动刷新 Git 仓库注册
  - [ ] `vdw.addRepo` 成功后自动打开新添加的可编辑仓库
  - [ ] 提供配置项 `vdw.autoOpenGitRepositories` 控制开关
  - [ ] VS Code Source Control 面板显示所有子仓库
  - [ ] 子仓库支持正常的 Git 操作（commit、push、pull）
  - [ ] 错误处理完善，失败时不阻断主流程
  - [ ] 多个仓库打开时添加延迟避免冲突

---

#### F7: 依赖图与循环依赖检测 (DAG Validation)

**优先级**: P1 (MVP 后可迭代)

**功能描述**：
- 解析所有依赖仓库的 `.vdwrc`，构建完整的依赖关系图
- 检测循环依赖并给出警告
- 可视化依赖关系（可选）

**验收标准**：
- [ ] `vdw validate` 命令输出依赖图
- [ ] 循环依赖时输出清晰的错误路径（A → B → C → A）
- [ ] 支持设置最大依赖深度（默认 5 层）

---

#### F8: 事件驱动索引 (Incremental Indexing)

**优先级**: P1 (MVP 后可迭代)

**功能描述**：
- 监听文件系统变更事件（inotify / FSEvents / ReadDirectoryChangesW）
- 仅重新索引变更的文件及其依赖链
- 维护"脏标记"，在 AI Prompt 中标注未保存的修改

**验收标准**：
- [ ] 文件保存后 1 秒内完成增量索引更新
- [ ] 内存占用 < 200MB（10000 文件规模）
- [ ] 提供 `vdw index status` 命令查看索引状态

---

### 2.3 功能优先级矩阵

| 功能 | 优先级 | MVP | V2 | V3 |
|------|--------|-----|----|----|
| F1: 工作空间配置管理 | P0 | ✅ | | |
| F2: 内容存储与去重 | P0 | ✅ | | |
| F3: 链接引擎与逻辑视图 | P0 | ✅ | | |
| F4: Git Worktree 集成 | P0 | ✅ | | |
| F5: AI 上下文过滤器 | P0 | ✅ | | |
| F6: VS Code 插件 | P0 | ✅ | | |
| F7: 依赖图与循环检测 | P1 | | ✅ | |
| F8: 事件驱动索引 | P1 | | ✅ | |
| F9: JetBrains 插件 | P2 | | | ✅ |
| F10: FUSE 虚拟文件系统 | P2 | | | ✅ |
| F11: LLM Gateway 集成 | P2 | | | ✅ |

---

## 三、 非功能需求

### 3.1 性能指标

| 指标 | 目标值 | 测试条件 |
|------|--------|---------|
| 链接创建时间 | < 10 秒 | 5 个仓库，每个 < 500MB |
| 上下文文件列表生成 | < 500ms | 1000 文件规模 |
| 增量索引更新 | < 1 秒 | 单文件修改 |
| 磁盘空间节省 | > 60% | 相同仓库多分支场景 |
| IDE 启动时间影响 | < 2 秒 | 相比无 VDW 场景 |
| 内存占用 | < 200MB | 10000 文件规模 |

### 3.2 兼容性要求

| 平台 | 最低版本 | 链接方案 | 备注 |
|------|---------|---------|------|
| macOS | 12.0+ (Monterey) | Hard Link / Symlink | 优先 Hard Link |
| Linux | Ubuntu 20.04+ | Hard Link / Symlink | 支持主流发行版 |
| Windows | Windows 10 20H2+ | Junction Point / Symlink | 需要开发者模式 |
| Node.js | 18+ | - | CLI 工具运行时 |
| VS Code | 1.85+ | - | 插件最低版本 |

### 3.3 安全要求

- [ ] `.vdwrc` 中的敏感信息（如 Token）支持环境变量引用，禁止明文存储
- [ ] Content Store 权限控制：仅当前用户可访问（chmod 700）
- [ ] Git 操作使用 SSH 密钥或 Credential Manager，VDW 不存储密码
- [ ] Symlink 创建时校验目标路径，防止符号链接攻击

### 3.4 可维护性

- [ ] 提供详细的操作日志（`~/.vdw/logs/vdw.log`）
- [ ] 所有命令支持 `--verbose` 模式输出调试信息
- [ ] 提供 `vdw doctor` 命令诊断常见问题
- [ ] 错误信息包含解决方案建议（而非仅报错）

---

## 四、 技术方案

### 4.1 技术栈选型

| 组件 | 技术 | 理由 |
|------|------|------|
| CLI 工具 | **Rust** | 高性能文件系统操作、跨平台编译、内存安全 |
| VS Code 插件 | **TypeScript** | VS Code 官方语言、生态完善 |
| 配置文件解析 | **serde + yaml-rust** (Rust) | 类型安全、性能好 |
| Git 操作 | **git2** (libgit2 Rust bindings) | 原生 Git 操作、避免 shell 调用 |
| 文件监听 | **notify** (Rust crate) | 跨平台文件系统事件监听 |
| 日志系统 | **tracing** (Rust) | 结构化日志、性能分析 |

### 4.2 核心数据结构

```rust
// .vdwrc 配置结构
#[derive(Debug, Serialize, Deserialize)]
pub struct VDWConfig {
    pub workspace: Workspace,
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Dependency {
    pub repo: String,          // Git 远程仓库 URL
    pub path: String,          // 本地逻辑路径
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub commit: Option<String>,
    pub scope: Scope,
    pub editable: bool,        // 是否可编辑
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Scope {
    pub include: Vec<String>,  // Glob 模式
    pub exclude: Vec<String>,  // Glob 模式
}

// Content Store 元数据
#[derive(Debug)]
pub struct StoreMetadata {
    pub repo_url: String,
    pub commit_hash: String,
    pub reference_count: u32,  // 被多少个工作空间引用
    pub last_accessed: DateTime,
    pub size_bytes: u64,
}
```

### 4.3 CLI 命令设计

```bash
# 初始化工作空间
vdw init                          # 交互式生成 .vdwrc
vdw init --template fullstack     # 使用预设模板

# 链接管理
vdw link                          # 根据 .vdwrc 创建链接
vdw unlink                        # 解除链接
vdw link status                   # 查看链接状态

# 仓库管理
vdw repo add <url> [options]      # 添加依赖仓库
vdw repo remove <name>            # 移除依赖仓库
vdw repo list                     # 列出所有依赖仓库
vdw repo update [name]            # 更新仓库到最新 commit

# Worktree 管理
vdw worktree list                 # 列出所有 Worktree
vdw worktree create <branch>      # 创建新 Worktree
vdw worktree delete <branch>      # 删除 Worktree

# Store 管理
vdw store info                    # 查看 Store 占用空间
vdw store clean                   # 清理未引用的对象
vdw store repair                  # 修复损坏的 Store

# 上下文管理
vdw context preview               # 预览当前 AI 上下文文件
vdw context stats                 # 查看上下文统计信息

# 诊断工具
vdw validate                      # 验证 .vdwrc 配置
vdw doctor                        # 诊断常见问题
vdw logs [--level debug]          # 查看日志

# 帮助
vdw --help
vdw <command> --help
```

---

## 五、 风险点与应对策略

### 5.1 技术风险

| 风险 | 影响 | 概率 | 应对策略 |
|------|------|------|---------||
| **Git 状态管理冲突** | 高 | 中 | 采用 Git Worktree 方案，每个逻辑目录保持独立的 `.git`，避免直接编辑 Store 中的文件 |
| **跨平台兼容性** | 中 | 高 | 抽象文件系统接口，提供平台特定的链接策略（Hard Link / Junction / Symlink），并设置降级方案 |
| **AI 上下文不一致** | 高 | 中 | 实现事件驱动索引，文件修改后 1 秒内更新索引；在 Prompt 中标注"脏文件" |
| **循环依赖导致死锁** | 高 | 低 | `.vdwrc` 解析阶段进行 DAG 检测，拒绝循环依赖配置 |
| **IDE 插件性能开销** | 中 | 中 | VDW 仅在初始化/切换分支时介入，日常编码时文件操作直接作用于 Overlay 层 |
| **自动打开 Git 仓库失败** | 低 | 中 | Git 扩展未安装或 API 变更时降级为手动提示，添加错误日志和重试机制 |

### 5.2 业务风险

| 风险 | 影响 | 概率 | 应对策略 |
|------|------|------|---------|
| **用户学习成本** | 中 | 高 | 提供交互式 `vdw init`、详细文档、视频教程；配置模板降低上手难度 |
| **与现有工具冲突** | 高 | 中 | MVP 阶段仅支持 VS Code + Git，避免与 Turborepo/Nx 等构建工具冲突 |
| **数据丢失** | 高 | 低 | 所有编辑操作在 Worktree 中进行，Store 只读；提供 `vdw backup` 命令 |
| **性能不达预期** | 中 | 中 | MVP 阶段聚焦核心链路性能优化，建立性能基准测试，持续监控 |

### 5.3 风险缓解措施

1. **灰度发布**：先在小范围开发者中测试 MVP，收集反馈后逐步扩大
2. **回滚机制**：所有 VDW 操作可逆，提供 `vdw rollback` 命令恢复到操作前状态
3. **监控告警**：集成 Sentry 等错误追踪工具，及时发现线上问题
4. **社区反馈**：建立 GitHub Discussions / Discord 频道，快速响应用户问题

---

## 六、 实施路线图

### 阶段 1: MVP (Month 1-2)

**目标**：验证核心价值 - 多仓库物理去重 + 逻辑聚合 + AI 上下文过滤

**交付物**：
- [ ] Rust CLI 工具（基础命令：`init`, `link`, `unlink`, `repo add/list`）
- [ ] Content Store 实现（Git Object Sharing）
- [ ] Git Worktree 集成（editable 模式）
- [ ] `.vdwrc` 解析与验证
- [ ] VS Code 插件（侧边栏视图、FileProvider、状态栏）
- [ ] **自动打开 Git 仓库功能**（扩展激活、link/relink/add 后自动注册）
- [ ] 基础文档（README、快速开始指南）

**技术指标**：
- 支持 macOS/Linux/Windows
- 5 个仓库链接时间 < 10 秒
- 磁盘空间节省 > 60%

---

### 阶段 2: 核心引擎增强 (Month 3-4)

**目标**：完善核心功能 - 依赖管理、循环检测、增量索引

**交付物**：
- [ ] 依赖图构建与循环依赖检测（`vdw validate`）
- [ ] 事件驱动索引（文件监听 + 增量更新）
- [ ] Store 管理命令（`clean`, `repair`, `info`）
- [ ] Worktree 管理命令（`create`, `delete`, `list`）
- [ ] VS Code 插件增强（一键添加仓库、上下文预览）
- [ ] 详细文档 + 视频教程

**技术指标**：
- 增量索引更新 < 1 秒
- 支持 10+ 仓库同时链接
- 内存占用 < 200MB

---

### 阶段 3: 生态集成 (Month 5-6)

**目标**：无缝融入开发流 - JetBrains 插件、LLM Gateway、FUSE

**交付物**：
- [ ] JetBrains IDE 插件（IntelliJ/WebStorm）
- [ ] LLM Gateway 集成（动态上下文注入）
- [ ] FUSE 虚拟文件系统（可选，完全透明）
- [ ] CI/CD 集成（GitHub Actions / GitLab CI）
- [ ] 企业级功能（SSO、权限管理、审计日志）

**技术指标**：
- 支持 3 大 IDE（VS Code、IntelliJ、WebStorm）
- FUSE 文件系统性能损失 < 5%
- 企业版支持 50+ 开发者并发使用

---

## 七、 验收标准

### 7.1 功能验收

| 场景 | 预期结果 | 验收方法 |
|------|---------|---------|
| 用户执行 `vdw init` 并添加 2 个仓库 | 生成 `.vdwrc`，执行 `vdw link` 后本地出现逻辑目录 | 手动测试 + 自动化 E2E 测试 |
| 用户在 VS Code 中打开 VDW 工作空间 | 侧边栏显示 2 个仓库，状态栏显示上下文文件数 | 手动测试 |
| AI Agent 请求上下文 | 仅返回 `.vdwrc` scope 内的文件 | 单元测试 + 集成测试 |
| 用户修改 editable 仓库的文件并执行 `git commit` | Git 操作正常，不影响其他仓库 | 手动测试 |
| 用户执行 `vdw unlink` | 逻辑目录消失，源码保留在 Store | 手动测试 |
| 配置循环依赖 | `vdw validate` 报错并输出依赖路径 | 单元测试 |

### 7.2 性能验收

| 指标 | 目标值 | 测试工具 | 验收方法 |
|------|--------|---------|---------|
| 链接创建时间 | < 10 秒 | `time vdw link` | 连续测试 10 次取平均值 |
| 上下文生成时间 | < 500ms | VS Code DevTools | 插件性能面板监控 |
| 磁盘空间节省 | > 60% | `du -sh` | 对比传统 clone 方案 |
| 内存占用 | < 200MB | `ps aux` / Activity Monitor | 运行 1 小时后检查 |

### 7.3 兼容性验收

| 平台 | 测试版本 | 验收标准 |
|------|---------|---------|
| macOS | 12.0, 13.0, 14.0 | 所有核心功能正常 |
| Linux | Ubuntu 20.04, 22.04 | 所有核心功能正常 |
| Windows | Windows 10 20H2, Windows 11 | 所有核心功能正常（Symlink 需要开发者模式） |
| VS Code | 1.85, 1.90, 1.95 | 插件正常加载，无报错 |

---

## 八、 后续迭代规划

### V2.0 (Month 7-9)

- **AST 分析**：基于代码调用关系动态扩展上下文（超越静态 scope）
- **智能推荐**：根据当前编辑的文件，AI 自动推荐需要加入上下文的仓库
- **分支管理**：可视化分支状态，一键切换/合并
- **性能优化**：Rust 异步 I/O、并行链接创建

### V3.0 (Month 10-12)

- **FUSE 虚拟文件系统**：完全透明的虚拟文件系统，IDE 无需感知 VDW
- **LLM Gateway**：在 Prompt 发送前拦截，动态注入跨仓库代码片段
- **团队协作**：共享 `.vdwrc` 配置，团队统一开发环境
- **企业功能**：SSO、权限管理、审计日志、私有化部署

---

## 九、 附录

### 9.1 术语表

| 术语 | 解释 |
|------|------|
| VDW | Virtual Dev Workspace，虚拟开发工作空间 |
| Content Store | 全局去重存储池，基于 CAS 策略 |
| Hard Link | 硬链接，同文件系统下的文件级引用 |
| Symlink | 符号链接，跨文件系统的指针文件 |
| Junction Point | Windows 特有的目录级链接 |
| Git Worktree | Git 原生功能，支持同一仓库多工作副本 |
| Git Object Sharing | 通过 `--reference` 或 `alternates` 共享 Git 对象库 |
| Copy-on-Write (CoW) | 写时复制，仅在写入时复制数据 |
| CAS | Content-Addressable Storage，内容寻址存储 |
| DAG | Directed Acyclic Graph，有向无环图 |
| AST | Abstract Syntax Tree，抽象语法树 |
| LSP | Language Server Protocol，语言服务器协议 |

### 9.2 参考项目

- **pnpm**: https://pnpm.io/ - 内容寻址存储和硬链接机制
- **Git Worktree**: https://git-scm.com/docs/git-worktree - Git 多工作副本
- **Git Alternates**: https://git-scm.com/docs/gitrepository-layout - Git 对象库共享
- **FUSE**: https://github.com/libfuse/libfuse - 用户空间文件系统
- **Turborepo**: https://turbo.build/repo - Monorepo 构建系统
- **Nx**: https://nx.dev/ - Monorepo 开发工具

### 9.3 变更记录

| 版本 | 日期 | 变更内容 | 作者 |
|------|------|---------|------|
| v1.0 | 2026-06-02 | 初始版本，MVP 需求定义 | AI Agent |
| v1.1 | 2026-06-02 | 新增自动打开 Git 仓库功能需求（F6 增强） | AI Agent |

---

**文档结束**

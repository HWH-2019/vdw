# VDW_Windows兼容改进_需求文档_v1.0

---

## 1. 改动背景

### 1.1 现状分析

当前 VDW 的链接策略在各平台不统一：

| 平台 | 当前默认策略 | 备选策略 | 问题 |
|------|-------------|---------|------|
| macOS | HardLink | Symlink | 稳定 |
| Linux | HardLink | Symlink | 稳定 |
| Windows | Junction（目录）→ Symlink（文件） | Symlink | **有实际缺陷** |

Windows 存在以下问题：

1. **Junction 从未被真实使用** — `link/engine.rs` 中 `select_strategy_for_file()` 对文件返回 Symlink，对目录也返回 Symlink（不走 Junction 路径）。`junction.rs` 中的代码是事实上的 dead code。
2. **Symlink 依赖高权限** — Windows 创建 Symlink 需要 `SeCreateSymbolicLinkPrivilege`，用户需开启开发者模式或以管理员身份运行，造成了使用门槛。
3. **三平台策略不一致** — macOS/Linux 用 HardLink（零额外空间、无需特殊权限），Windows 用户需额外配置，体验割裂。

### 1.2 业务诉求

- 消除 Windows 用户使用 VDW 前的额外配置步骤（开启开发者模式/管理员运行）
- 保持三平台使用体验的一致性
- HardLink 断链后能自动检测并恢复，保证安全可用
- 不破坏现有 macOS/Linux 的稳定支持

---

## 2. 改动目标

### 2.1 核心目标

- **Windows 默认链接策略从 Junction/Symlink 切换为 HardLink**，与 macOS/Linux 保持一致
- **提供 HardLink 断链的自动检测与恢复**，解决原子保存导致的断链问题
- **不修改 macOS/Linux 任何代码**，零回归风险

### 2.2 非目标（明确不做的内容）

- 不引入后台守护进程或系统服务（仅 `--watch` 前台模式）
- 不修改 macOS/Linux 的链接策略代码
- 短期内不移除 Junction 代码（仅标记废弃）
- 不做跨分区 HardLink 支持（降级 Symlink 即可）

---

## 3. 功能需求

### 3.1 Windows 默认策略切换

#### 功能描述

将 Windows 平台的默认链接策略从 `Junction` 改为 `HardLink`，使所有文件（不含目录）优先使用 HardLink 创建链接。目录的链接策略保持不变（始终使用 Symlink，因为 NTFS 不支持目录 HardLink）。

**变更位置**：[`link/platform.rs`](file:///d:/Vue/vdw/vdw/src/link/platform.rs)

```rust
// 修改前（windows 分支）
pub const DEFAULT_STRATEGY: LinkStrategy = LinkStrategy::Junction;
pub const FALLBACK_STRATEGY: LinkStrategy = LinkStrategy::Symlink;

// 修改后（windows 分支）
pub const DEFAULT_STRATEGY: LinkStrategy = LinkStrategy::HardLink;
pub const FALLBACK_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
```

**业务规则**：

| 场景 | 策略 | 说明 |
|------|------|------|
| Windows + 文件 + 同分区 | HardLink | 默认策略，无需特殊权限 |
| Windows + 目录 | Symlink | HardLink 不支持目录，自动降级 |
| Windows + 文件 + 跨分区 | Symlink | HardLink 无法跨分区，自动降级 |
| Windows + Symlink 权限不足 | 报错 + 提示 | 仅目录/Symlink 降级场景可能触发 |

#### 影响边界

- 修改范围仅为 `platform.rs` 中两行常量的值
- `select_strategy_for_file()` 函数的逻辑无需变更（Junction 分支本身从未被命中过）
- macOS/Linux 的 `cfg!` 分支完全不受影响
- `engine.rs` 中 `create_single_link()` 的 Junction 匹配分支后续不会执行到，由 `#[deprecated]` 标记

### 3.2 inode 元数据记录

#### 功能描述

在 `.vdw/state.json` 的 `LinkEntry` 中增加 inode 元数据，为后续断链检测提供比对基准。

**变更位置**：[`types/workspace.rs`](file:///d:/Vue/vdw/vdw/src/types/workspace.rs)

```rust
pub struct LinkEntry {
    pub source: PathBuf,
    pub target: PathBuf,
    pub link_type: LinkType,
    pub dependency_repo: String,

    // 新增（可选字段，向后兼容）
    /// 源文件的 inode/文件索引（用于 HardLink 断链检测）
    pub source_inode: Option<u64>,
    /// 源文件所在卷的序列号（Windows）或设备号（Unix）
    pub source_volume: Option<u64>,
}
```

**写入时机**：`vdw link` 创建 HardLink 时，在 `engine.rs` 的 `create_dependency_links()` 中获取源文件的 `(volume_serial, file_index)` 并写入 `LinkEntry.source_inode` / `LinkEntry.source_volume`。

**读取兼容性**：`Option<u64>` 确保旧版 `state.json`（不含此字段）可正常反序列化。`vdw doctor --check-links` 检测到 `None` 时提示用户重新执行 `vdw link`。

### 3.3 断链检测与告警（vdw link --watch）

#### 功能描述

为 `vdw link` 新增 `--watch` 参数，在链接创建完成后进入事件监听模式，实时检测 HardLink 断链并向用户报告。

采用 `notify` crate 做文件系统事件驱动（非轮询），监听工作区目录的 `Create` 和 `Rename` 事件。当受 VDW 管理的文件发生变更时，比对 `(volume_serial, file_index)` 判断是否断链。检测到断链后仅输出告警，**不自动重建**，提示用户执行 `vdw link --repo <url>` 手动修复。

#### 接口定义

| 项目 | 内容 |
|------|------|
| 命令 | `vdw link --watch` |
| 新增参数 | `--watch`：链接创建完成后启动文件监听，保持前台运行 |
| 退出方式 | `Ctrl+C` 终止进程 |
| 鉴权要求 | 无 |
| 平台 | 三平台均支持，但 Windows 是主要目标 |

#### 工作流程

```
vdw link --watch
  │
  ├─ 1. 执行完整 link 流程（创建 HardLink/Symlink，写入 state.json）
  │
  ├─ 2. 加载 state.json，收集所有 HardLink 条目的 (source, target, source_inode, source_volume)
  │
  ├─ 3. 注册 notify 递归监听（RecursiveMode::Recursive）
  │
  └─ 4. 事件循环（前台阻塞）:
       │
       ├─ 收到 Create(<target_path>) 事件
       │    └─ 匹配 VDW 管理文件?
       │         ├─ 否 → 忽略
       │         └─ 是 → 200ms 去抖后比对 inode
       │               ├─ 一致 → 忽略（非原子保存的正常写入）
       │               └─ 不一致 → 断链！
       │                     └─ stderr 输出告警（见下方）
       │
       ├─ 收到 Rename(<tmp>, <target_path>) 事件
       │    └─ 同上处理
       │
       └─ 用户 Ctrl+C → 退出
```

#### 去抖策略

收到事件后启动 200ms 定时器：
- 200ms 内无同一文件的连续事件 → 执行 inode 比对
- 200ms 内有同一文件的连续事件 → 重置定时器
- 防止 rename 未完成时误判

#### 告警输出格式

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
⚠️  HardLink 断链检测

   文件: libs/repo-a/src/main.rs
   原因: 编辑器原子保存导致 inode 变更

   修复: vdw link --repo https://github.com/example/repo-a.git

   建议: 关闭 VS Code 原子保存以避免反复断链
     设置 → "files.atomicSave": false
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

#### Store 源文件检查

检测到 inode 不一致后，先检查 Store 源文件是否存在：

| 状态 | 行为 |
|------|------|
| Store 源文件存在 | 输出告警 + 修复命令 |
| Store 源文件不存在（被删除） | 输出告警："Store 源文件已丢失，请执行 vdw repo update" |

#### 事件风暴防护

- 去抖窗口 200ms 合并连续事件
- 同一文件的连续断链：每分钟最多输出 1 次告警（避免高频重复刷屏）

### 3.4 全量断链扫描（vdw doctor --check-links）

#### 功能描述

独立的全量扫描命令，不依赖 `--watch` 模式。适用于：
- 用户未使用 `--watch` 但怀疑有断链
- `vdw --watch` 退出后重新检查
- 旧版 state.json（无 inode 记录）升级后的首次检查

#### 接口定义

| 项目 | 内容 |
|------|------|
| 命令 | `vdw doctor --check-links` |
| 扫描方式 | 一次性全量扫描（非持续监听） |
| 修复方式 | 根据报告中的提示执行 `vdw link --repo <url>` 手动修复 |

#### 输出格式

```
$ vdw doctor --check-links
🔍 正在检查 HardLink 健康状态...
   共 42 个 HardLink 条目，分布于 3 个仓库

✅ 所有 HardLink 连接正常

---

$ vdw doctor --check-links
🔍 正在检查 HardLink 健康状态...
   共 42 个 HardLink 条目，分布于 3 个仓库

⚠️ 发现 2 个断链文件:

  1. libs/repo-a/src/main.rs
     修复: vdw link --repo https://github.com/example/repo-a.git

  2. libs/repo-b/src/utils/helper.ts
     修复: vdw link --repo https://github.com/example/repo-b.git

💡 提示: 使用 `vdw link --watch` 可实时监控断链
   建议: 关闭 VS Code 原子保存 "files.atomicSave": false

---

$ vdw doctor --check-links
🔍 正在检查 HardLink 健康状态...
   无法检查: state.json 中缺少 inode 信息
   请先执行 `vdw link` 以记录 inode 元数据
```

#### 跨平台行为

| 平台 | 行为 | 说明 |
|------|------|------|
| Windows | 执行完整检测 | 主要目标平台 |
| macOS | 执行完整检测 | 兼容支持，无实际意义 |
| Linux | 执行完整检测 | 兼容支持，无实际意义 |

检测逻辑本身跨平台，但仅在 Windows 上有实际意义。

### 3.5 普通 `vdw link`（不带 `--watch`）的用户提示

#### 功能描述

`vdw link` 不带 `--watch` 时，在输出末尾添加提示，告知用户断链检测能力。

**输出内容**（仅 Windows 平台）：

```
💡 提示: 使用 `vdw link --watch` 实时监控 HardLink 断链
   检测断链: vdw doctor --check-links
   修复: vdw link --repo <url>
   建议: 关闭 VS Code 原子保存 "files.atomicSave": false
```

### 3.6 Junction 代码废弃标记

#### 功能描述

在 [`junction.rs`](file:///d:/Vue/vdw/vdw/src/link/junction.rs) 的模块级别和函数级别添加 `#[deprecated]` 标记。

```rust
// 模块级
#![deprecated(note = "Junction is no longer the default since v0.6.0. "
                     "Windows now uses HardLink. This module will be removed in v0.7.0.")]

// 函数级
#[deprecated(note = "Use HardLink instead")]
pub fn create_junction(source: &Path, target: &Path) -> Result<()> { ... }
```

---

## 4. 预期改动

### 4.1 文件级改动清单

#### 新增文件

| 文件 | 内容 | 说明 |
|------|------|------|
| `src/link/watcher.rs` | 事件监听 + 断链检测 + 自动恢复 | 基于 `notify` crate，约 200 行 |

#### 修改文件

| 文件 | 改动 | 说明 |
|------|------|------|
| `src/link/platform.rs` | Windows `DEFAULT_STRATEGY` → `HardLink` | 2 行，仅 Windows 分支 |
| `src/link/engine.rs` | `create_dependency_links()` 写入 inode 到 LinkEntry | 不影响 macOS/Linux 逻辑 |
| `src/link/mod.rs` | 导出 `watcher` 模块 | +1 行 |
| `src/types/workspace.rs` | `LinkEntry` 新增 `source_inode`, `source_volume` | `Option<u64>` 向后兼容 |
| `src/commands/link.rs` | 新增 `--watch` 参数处理 + `#[cfg(windows)]` 提示 | 新增 ~30 行 |
| `src/commands/doctor.rs` | 新增 `--check-links` 参数 | 新增 ~30 行 |
| `src/link/junction.rs` | 添加 `#[deprecated]` | ~3 行 |
| `Cargo.toml` | 新增 `notify` 依赖 | +1 行 |

#### 不做修改的文件

| 文件 | 原因 |
|------|------|
| `src/link/symlink.rs` | 仅作为目录/跨分区降级使用，逻辑不变 |
| `src/link/hardlink.rs` | HardLink 创建/删除逻辑完全不变 |
| `src/link/junction.rs` | 仅加 `#[deprecated]`，代码不删 |
| `src/config/` 下所有文件 | 不涉及配置变更 |
| `src/worktree/` 下所有文件 | editable 模式不受影响 |
| `src/store/` 下所有文件 | Store 逻辑不受影响 |
| macOS/Linux 专用代码 | 零改动 |

### 4.2 数据模型改动

**修改结构体：`LinkEntry`**

| 字段 | 类型 | 说明 | 变更类型 |
|------|------|------|----------|
| source | PathBuf | 源路径 | 不变 |
| target | PathBuf | 目标路径 | 不变 |
| link_type | LinkType | 链接类型 | 不变 |
| dependency_repo | String | 所属仓库 | 不变 |
| **source_inode** | **Option\<u64\>** | **源文件 inode/文件索引** | **新增** |
| **source_volume** | **Option\<u64\>** | **源卷序列号/设备号** | **新增** |

### 4.3 依赖变更

| crate | 版本 | 说明 |
|-------|------|------|
| `notify` | 6.1 | 跨平台文件系统事件监听。feature 默认值即可，不需要 `macos_fsevents` 等额外 feature |

### 4.4 部署注意事项

- `state.json` 向前兼容：旧版不含 `source_inode` / `source_volume`，反序列化为 `None`，`--watch` 和 `--check-links` 自动降级输出友好提示
- 不需要数据迁移
- `notify` crate 在 Windows 上使用 `ReadDirectoryChangesW`，是 OS 原生 API，无需额外运行时组件

---

## 5. 验收标准

### 5.1 功能验收

| # | 验收点 | 验收标准 | 优先级 |
|---|--------|----------|--------|
| 1 | Windows HardLink 默认策略 | `vdw link` 后文件使用 HardLink，验证 `fsutil hardlink list <file>` 或 `(Get-Item <path>).LinkType` | P0 |
| 2 | 无需管理员权限 | 普通用户（非 Admin，未开开发者模式）可正常执行 `vdw link` | P0 |
| 3 | state.json 记录 inode | `vdw link` 后 `.vdw/state.json` 中 HardLink 条目包含 `source_inode` 和 `source_volume` | P0 |
| 4 | `--watch` 检测断链并告警 | 用 VS Code 原子保存编辑文件后，`--watch` 在 2 秒内检测到断链并输出格式正确的告警信息 | P0 |
| 5 | `--watch` 修复指引 | 告警信息中包含 `vdw link --repo <url>` 修复命令 | P1 |
| 6 | `vdw doctor --check-links` 报告断链 | 对有断链的工作区执行全量扫描，正确报告所有断链文件 | P0 |
| 7 | `vdw doctor --check-links` 报告健康 | 无断链时输出 "所有 HardLink 连接正常" | P1 |
| 8 | 无断链时正常 | `--watch` 和 `--check-links` 在无断链时输出 "连接正常" | P1 |
| 9 | macOS/Linux 回归 | 现有 `cargo test` 全部通过，行为无变化 | P0 |
| 10 | 向后兼容旧 state.json | 旧版 state.json 加载无错误，`--watch` / `--check-links` 输出友好提示 | P1 |

### 5.2 非功能验收

| # | 检查项 | 验收标准 |
|---|--------|----------|
| 1 | 编译无错误 | `cargo build` 三平台无错误 |
| 2 | 现有测试通过 | `cargo test` 全部通过，无需修改现有测试 |
| 3 | macOS/Linux 回归 | macOS/Linux 上 `cargo test` 全部通过 |
| 4 | `--watch` 空闲 CPU 占用 | 无文件变更时 CPU 占用 ≈ 0%（事件驱动，非轮询） |
| 5 | `--watch` 内存占用 | < 20MB（仅维护文件名 → inode 的 HashMap） |

---

## 6. 预期风险

### 6.1 技术风险

| 风险项 | 风险描述 | 影响程度 | 应对方案 |
|--------|----------|----------|----------|
| notify Windows 缓冲区溢出 | 极端高频写入时 ReadDirectoryChangesW 可能丢事件 | 低 | `vdw doctor --check-links` 作为备用全量扫描手段可兜底 |
| 大量文件首次监听初始化慢 | 工作区文件 > 10 万时 notify 注册监听可能耗时数秒 | 低 | 首次初始化有进度提示，仅一次成本 |
| HardLink 跨分区失败 | Store 和工作区在不同分区时报错 | 中 | 现有 `is_same_partition()` 已覆盖，自动降级 Symlink |
| `#[deprecated]` 编译警告 | engine.rs 对 Junction 的引用产生警告 | 低 | 对应位置加 `#[allow(deprecated)]` |

### 6.2 业务风险

| 风险项 | 风险描述 | 影响程度 | 应对方案 |
|--------|----------|----------|----------|
| 用户关闭终端后 `--watch` 退出 | `--watch` 是前台进程，关闭终端后不再监控 | 中 | 这是设计意图，非风险；`vdw doctor --check-links` 作为替代手段 |
| Store 源文件被误删 | 源文件丢失后断链无法重建 | 低 | 告警提示用户执行 `vdw repo update` 重新获取 |
| 旧版 Junction 用户升级 | 旧版生成的 Junction 链接在新版下如何处理 | 低 | 执行 `vdw unlink`（移除 junction）后 `vdw link`（重建 HardLink）即可 |

### 6.3 回滚方案

1. **单用户回滚**：`vdw unlink` → 安装旧版二进制 → `vdw link`
2. **版本回退**：恢复 `platform.rs` 中 Windows 分支为 `DEFAULT_STRATEGY = Junction`，移除 `notify` 依赖

---

> **文档信息**
>
> | 项目 | 内容 |
> |------|------|
> | 创建日期 | 2026-06-10 |
> | 版本号 | v1.0 |
> | 状态 | 草稿 |

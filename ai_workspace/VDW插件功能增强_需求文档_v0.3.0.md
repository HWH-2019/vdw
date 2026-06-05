# VDW插件功能增强_需求文档_v0.3.0

---

## 1. 改动背景

### 1.1 现状分析

VDW VS Code 插件目前已实现核心工作流（add、link、unlink、remove），但在以下方面存在不足：

1. **Git 面板同步不完整**：插件激活后仅自动打开 Git 仓库，未刷新已有仓库状态，导致 Source Control 面板中可能出现已过时的仓库条目
2. **unlink 图标语义不清晰**：当前 unlink 的内联操作图标为用户自制的 SVG，视觉效果不够直观，用户无法快速识别为"断开链接"
3. **缺少路径快捷操作**：用户在 TreeView 中查看依赖的路径信息时，需要手动在终端中 cd 到对应目录，工作流不够流畅

### 1.2 业务诉求

- 插件启动后自动同步 Git 仓库状态，确保 Source Control 面板与当前工作空间一致
- 改进 unlink 操作的视觉标识，使其更符合用户心智模型
- 在 TreeView 路径行提供一键打开终端并跳转的功能，提升开发效率

---

## 2. 改动目标

### 2.1 核心目标

- 插件激活时自动触发 `refreshGitRepositories`，确保 Git 面板与工作空间同步
- 将 unlink 图标替换为 VS Code 内置 `$(unlink)` 图标，提升语义清晰度
- 在 TreeView 依赖路径行末尾添加终端图标，点击后自动创建终端并跳转到对应目录
- 版本号从 `0.2.0` 升级到 `0.3.0`

### 2.2 非目标

- 不修改 VS Code TreeView 的布局或交互逻辑
- 不改动 CLI 侧的 Rust 代码
- 不新增后端接口或数据模型

---

## 3. 功能需求

### 3.1 插件激活时触发 Git 仓库刷新

#### 功能描述

在 VS Code 检测到工作空间包含 `.vdwrc` 文件并激活 VDW 插件时，在执行完配置加载和自动打开 Git 仓库后，触发 `autoOpenService.refreshGitRepositories()`，传入所有 editable 依赖的候选路径（dep.local、dep.origin、workspace+dep.path），确保 Git 面板中不残留已失效的仓库条目。

#### 触发时机

- 插件 `activate()` 执行末尾，`autoOpenService.openAllEditableRepositories()` 之前或之后
- `.vdwrc` 文件变更时（`onDidChange`、`onDidCreate`）的 reload 流程

#### 业务规则

- 仅在 `configService.loadConfig()` 返回非空配置时执行
- 仅收集 `dep.editable === true` 的依赖路径
- 使用现有的 `autoOpenService.refreshGitRepositories(...)` 方法
- 调用必须在 `autoOpenService` 初始化完成之后

### 3.2 unlink 图标更换

#### 功能描述

将 `vdw.unlinkRepo` 命令的图标从自定义 SVG 文件替换为 VS Code 内置的 `$(unlink)` 图标。

#### 当前配置

```json
{
    "command": "vdw.unlinkRepo",
    "title": "Unlink Repository",
    "icon": {
        "light": "resources/icons/unlink-light.svg",
        "dark": "resources/icons/unlink-dark.svg"
    }
}
```

#### 目标配置

```json
{
    "command": "vdw.unlinkRepo",
    "title": "Unlink Repository",
    "icon": "$(unlink)"
}
```

#### 额外操作

- 删除不再使用的 SVG 资源文件：`resources/icons/unlink-light.svg`、`resources/icons/unlink-dark.svg`

### 3.3 路径行新增终端图标

#### 功能描述

在 VDW 依赖的路径子项（显示 `./packages/xxx` 的行）末尾添加一个终端图标，点击后自动创建 VS Code 终端并跳转到该依赖的完整路径。

#### 交互流程

1. 用户在 VDW Workspace TreeView 中展开某个依赖
2. 看到路径行（如 `./packages/alsc.sales_aoi_service`）
3. 在路径行末尾有一个终端图标 `$(terminal)`（自定义 SVG）
4. 用户点击终端图标
5. VS Code 创建新终端，自动 cd 到该依赖的完整路径
6. 终端自动获得焦点

#### 技术实现

**前端改动（`workspaceTreeView.ts`）：**

- Path 子项构造时传入父级的 `dependency` 对象
- 在 `WorkspaceTreeItem` 构造函数中，如果 `contextType === 'path'` 且 `dependency` 存在
- 设置 `item.command` 为一个 `vdw.openInTerminal` 命令，并将路径信息传入

**命令注册（`registerCommands` 在 `commands/index.ts`）：**

- 新增 `vdw.openInTerminal` 命令
- 命令接收树节点作为参数
- 从树节点的 `dependency` 获取 `dep.path`
- 拼出完整路径：`workspaceRoot + '/' + dep.path`
- 调用 `vscode.window.createTerminal({ cwd: fullPath, name: repoName })`
- 调用 `terminal.show()` 获取焦点

**图标与菜单配置（`package.json`）：**

- 在 `contributes.commands` 中注册 `vdw.openInTerminal`，使用 `$(terminal)` 图标
- 在 `contributes.menus['view/item/context']` 中添加：
  ```json
  {
      "command": "vdw.openInTerminal",
      "when": "view == vdwWorkspace && viewItem == path",
      "group": "inline@2"
  }
  ```

**终端 SVG 图标：**

- 创建 `resources/icons/terminal-light.svg` 和 `resources/icons/terminal-dark.svg`
- 风格与现有 VDW 图标一致（简洁线条风格）
- 亮色主题使用深色线条，暗色主题使用浅色线条

---

## 4. 预期改动

### 4.1 VS Code 插件改动

#### `extension.ts`

- 在 `activate()` 函数的步骤 10（auto-open Git repositories）之前，增加 `refreshGitRepositories` 调用
- 收集所有 editable dep 的候选路径，传入 `autoOpenService.refreshGitRepositories()`

#### `package.json`

- `version` 字段从 `"0.2.0"` 改为 `"0.3.0"`
- `vdw.unlinkRepo` 命令的 `icon` 从自定义 SVG 改为 `"$(unlink)"`
- `contributes.commands` 中新增 `vdw.openInTerminal` 命令
- `contributes.menus['view/item/context']` 中新增 `vdw.openInTerminal` 的 inline 菜单项

#### `resources/icons/`

- 删除 `unlink-light.svg`、`unlink-dark.svg`
- 新增 `terminal-light.svg`、`terminal-dark.svg`

#### `providers/workspaceTreeView.ts`

- Path 子项构造时传入父级的 `dependency` 参数
- 使 path item 的 `dependency` 属性不再为 `undefined`

#### `commands/index.ts`

- 新增 `vdw.openInTerminal` 命令注册
- 命令逻辑：获取依赖路径 → 创建终端 → 跳转路径

### 4.2 配置变更

| 配置项 | 变更 | 说明 |
|--------|------|------|
| `package.json` version | `0.2.0` → `0.3.0` | 版本升级 |
| `vdw.unlinkRepo` icon | SVG → `$(unlink)` | 图标替换 |
| 新增 `vdw.openInTerminal` | 命令 + 菜单项 | 终端快捷操作 |

### 4.3 部署注意事项

- 修改后需重新编译 TypeScript：`npm run compile`
- 重新打包 VSIX：`npm run package`
- 无需重启 VS Code，重新安装 VSIX 即可生效

---

## 5. 验收标准

### 5.1 功能验收

| # | 验收点 | 验收标准 | 优先级 |
|---|--------|----------|--------|
| 1 | 激活时刷新 Git 仓库 | 打开含有 `.vdwrc` 的工作空间，VDW 插件激活后，Source Control 面板自动刷新，无残留失效仓库 | P0 |
| 2 | unlink 图标语义 | unlink 内联按钮显示为断裂链条图标，鼠标悬停显示 "Unlink Repository" | P0 |
| 3 | 终端图标可见 | 在依赖展开后的路径行末尾显示终端图标 | P0 |
| 4 | 点击终端图标创建终端 | 点击终端图标后，VS Code 创建新终端，自动 cd 到对应路径 | P0 |
| 5 | `.vdwrc` 变更时 refresh | 修改 `.vdwrc` 后，插件自动 reload 并刷新 Git 仓库 | P1 |
| 6 | 图标暗色主题兼容 | 暗色主题和亮色主题下终端图标均清晰可见 | P1 |

### 5.2 非功能验收

| # | 检查项 | 验收标准 |
|---|--------|----------|
| 1 | 编译 | `npm run compile` 无错误 |
| 2 | 打包 | `npm run package` 成功生成 VSIX |
| 3 | 激活无异常 | VS Code Output 面板无未捕获异常 |

---

## 6. 预期风险

### 6.1 技术风险

| 风险项 | 风险描述 | 影响程度 | 应对方案 |
|--------|----------|----------|----------|
| `$(unlink)` 图标兼容性 | 低版本 VS Code 可能不识别 | 低 | 目标 Engine 为 `^1.85.0`，完全支持 |
| 终端重复创建 | 每次点击都创建新终端，可能累积 | 低 | 可考虑复用已有终端，但首次实现保持简单 |

### 6.2 业务风险

| 风险项 | 风险描述 | 影响程度 | 应对方案 |
|--------|----------|----------|----------|
| 终端路径错误 | dep.path 为相对路径，拼接错误 | 低 | 使用 `path.join(workspaceRoot, dep.path)` 确保正确 |

### 6.3 回滚方案

- 恢复 `package.json` 中 `version` 为 `0.2.0`
- 恢复 `.vscodeignore`、`package.json` 等配置文件
- 使用旧版 VSIX 重新安装

---

> **文档信息**
>
> | 项目 | 内容 |
> |------|------|
> | 创建日期 | 2026-06-05 |
> | 版本号 | v0.3.0 |
> | 状态 | 已定稿 |

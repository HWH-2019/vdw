# VDW 打包与分发 — 需求文档 v1.0

| 字段 | 内容 |
|------|------|
| **文档版本** | v1.0 |
| **创建日期** | 2026-06-05 |
| **最后更新** | 2026-06-05 |
| **关联设计文档** | [VDW 技术设计文档 v1.0](VDW虚拟工作空间_技术设计文档_v1.0.md) |

---

## 1. 需求概述

当前 VDW 项目缺少正式的安装包构建流程和服务端打包脚本：
- `vdw` CLI 仅有源码编译安装（`make install`），无 macOS `.dmg` 安装包
- `vdw-vscode` VSIX 命名不含平台/目标信息，与 CLI 产物命名风格不一致
- CI release.yml 产物命名缺少版本号（`vdw-aarch64-apple-darwin` → 应为 `vdw_darwin_arm64_0.5.0.tar.gz`）
- 项目根目录无统一的打包脚本入口

### 目标

1. 构建 macOS `.dmg` 安装包，支持拖拽安装 `vdw` 到 `/usr/local/bin`
2. 统一 CLI 和 VSIX 打包产物的命名规范（下划线分隔）
3. 在项目根 `scripts/` 目录集中管理所有打包脚本
4. 架构上预留 Linux (`tar.gz`) / Windows (`zip`) 扩展点

---

## 2. 统一命名规范

### 2.1 规则

所有打包产物的文件名遵循统一格式：

```
{name}_{os}_{arch}_{version}.{ext}
```

| 段 | 说明 | 示例 |
|----|------|------|
| `name` | 产品名 | `vdw`, `vdw_vscode` |
| `os` | 操作系统 | `darwin`, `linux`, `win32` |
| `arch` | CPU 架构 | `arm64`, `x86_64` |
| `version` | 三段版本号 | `0.5.0` |
| `ext` | 压缩/打包格式 | `dmg`, `tar.gz`, `zip`, `vsix` |

### 2.2 各产物命名示例

| 产物 | 命名 |
|------|------|
| vdw CLI macOS DMG | `vdw_darwin_arm64_0.5.0.dmg` |
| vdw CLI Linux tar.gz | `vdw_linux_x86_64_0.5.0.tar.gz` |
| vdw CLI Windows zip | `vdw_win32_x86_64_0.5.0.zip` |
| vdw-vscode VSIX | `vdw_vscode_0.5.0.vsix` |

> **VSIX 说明**：VS Code 扩展本身跨平台，`os` / `arch` 字段省略。用 `vdw_vscode` 而非 `vdw-vscode` 以保持下划线命名一致。

### 2.3 版本号来源

| 产物 | 版本号来源 | 提取方式 |
|------|-----------|---------|
| vdw CLI | `vdw/Cargo.toml` → `package.version` | `grep '^version' Cargo.toml \| sed ...` 或 `cargo metadata` |
| vdw-vscode | `vdw-vscode/package.json` → `version` | `jq -r '.version'` 或 `node -e` |

---

## 3. 功能详解

### 3.1 项目根 scripts/ 目录结构

在项目根 `/Users/tbsg/Desktop/codes/agent-vdm/scripts/` 创建以下文件：

```
scripts/
├── package_vdw.sh            # vdw CLI 打包主脚本（跨平台）
├── package_vdw_dmg.sh        # macOS .dmg 构建（调用 package_vdw.sh + hdiutil）
├── package_vscode.sh         # vdw-vscode VSIX 打包（编译 + vsce + 重命名）
├── package_all.sh            # 一键打包所有产物
└── common.sh                 # 共享函数（版本号提取、命名生成、日志等）
```

### 3.2 package_vdw.sh — CLI 通用打包

**输入**：`target_os` `target_arch`

**流程**：
1. 从 `vdw/Cargo.toml` 提取版本号
2. 调用 `make build`（或 `cargo build --release`）
3. 按命名规范生成产物文件名：`vdw_{os}_{arch}_{version}.tar.gz`
4. 将 `vdw/target/release/vdw` 打包为 `.tar.gz`
5. 输出产物路径

**抽象设计**（为跨平台预留扩展）：
```bash
# 用法: package_vdw.sh <os> <arch>
# 示例: package_vdw.sh darwin arm64
#       package_vdw.sh linux x86_64
#       package_vdw.sh win32 x86_64
```

### 3.3 package_vdw_dmg.sh — macOS DMG 构建

**前置条件**：macOS 环境，`hdiutil` 可用（系统自带）

**流程**：
1. 调用 `package_vdw.sh darwin {arch}` 生成 `.tar.gz`
2. 创建临时 DMG 源目录，布局如下：
   ```
   /tmp/vdw_dmg_root/
   ├── vdw                    # 二进制文件
   └── Applications/          # 快捷方式（符号链接到 /Applications）
   ```
   > 注：实际安装目标是 `/usr/local/bin`，但 `.dmg` 约定是拖拽到 `Applications` 的符号链接。在 DMG 背景图中说明安装路径。
3. 使用 `hdiutil create` 生成 `.dmg`
4. 按命名规范输出：`vdw_darwin_{arch}_{version}.dmg`
5. 清理临时文件

**DMG 可选增强**（后续迭代）：
- 自定义背景图
- `.dmg` 体积压缩 (`hdiutil convert -format UDZO`)
- Apple 代码签名

### 3.4 package_vscode.sh — VSIX 打包

**流程**：
1. 从 `vdw-vscode/package.json` 提取版本号
2. 调用 `npm run package` 生成默认 `.vsix`（命名：`vdw-vscode-{version}.vsix`）
3. 按命名规范重命名：`vdw_vscode_{version}.vsix`
4. 输出产物路径

> `npm run package` 内部执行 `npm run generate-icon && npm run compile && vsce package`，无需额外处理。

### 3.5 package_all.sh — 一键打包

```bash
#!/usr/bin/env bash
# 一键打包所有产物
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# macOS 打包
if [[ "$(uname -s)" == "Darwin" ]]; then
    ARCH=$(uname -m)
    [[ "$ARCH" == "arm64" ]] && ARCH="arm64" || ARCH="x86_64"
    bash "$SCRIPT_DIR/package_vdw_dmg.sh" "$ARCH"
fi

# Linux 打包
if [[ "$(uname -s)" == "Linux" ]]; then
    ARCH=$(uname -m)
    [[ "$ARCH" == "aarch64" ]] && ARCH="arm64" || ARCH="x86_64"
    bash "$SCRIPT_DIR/package_vdw.sh" linux "$ARCH"
fi

# VSIX 打包
bash "$SCRIPT_DIR/package_vscode.sh"
```

### 3.6 Makefile 入口

在项目根创建简洁的 `Makefile`（或扩展 `vdw/Makefile`），对外暴露统一命令：

```makefile
.PHONY: package package-vdw package-vdw-dmg package-vscode

package:
	scripts/package_all.sh

package-vdw:
	scripts/package_vdw.sh darwin arm64     # 可传参覆盖

package-vdw-dmg:
	scripts/package_vdw_dmg.sh arm64

package-vscode:
	scripts/package_vscode.sh
```

> **方案选择**：不修改 `vdw/Makefile`（它专注开发构建），在根 `scripts/` 独立管理打包流程。用户也可在根 Makefile 中提供快捷 target。

---

## 4. CI/CD 集成

### 4.1 更新 release.yml 命名

**当前**（`release.yml` L57）：
```yaml
ARCHIVE="vdw-${{ matrix.target }}${{ matrix.suffix }}"
```

**修改为**：
```yaml
- name: Extract version
  run: |
    VERSION=$(grep '^version' vdw/Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
    echo "version=$VERSION" >> $GITHUB_ENV

- name: Package binary
  run: |
    # Map target to os/arch
    case "${{ matrix.target }}" in
      aarch64-apple-darwin)   OS=darwin; ARCH=arm64 ;;
      x86_64-apple-darwin)    OS=darwin; ARCH=x86_64 ;;
      x86_64-unknown-linux-gnu) OS=linux; ARCH=x86_64 ;;
      aarch64-unknown-linux-gnu) OS=linux; ARCH=arm64 ;;
      x86_64-pc-windows-msvc) OS=win32; ARCH=x86_64 ;;
    esac
    ARCHIVE="vdw_${OS}_${ARCH}_${VERSION}"
    # tar for unix, zip for windows
    if [ "$OS" = "win32" ]; then
      cp target/${{ matrix.target }}/release/vdw.exe "${ARCHIVE}.exe"
      zip "${ARCHIVE}.zip" "${ARCHIVE}.exe"
      echo "artifact=${ARCHIVE}.zip" >> $GITHUB_ENV
    else
      cp target/${{ matrix.target }}/release/vdw "${ARCHIVE}"
      tar -czf "${ARCHIVE}.tar.gz" "${ARCHIVE}"
      echo "artifact=${ARCHIVE}.tar.gz" >> $GITHUB_ENV
    fi
```

### 4.2 VSIX CI 构建（预留）

```yaml
  build-vsix:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
      - run: npm ci
        working-directory: vdw-vscode
      - run: npm run package
        working-directory: vdw-vscode
      - name: Rename with version
        run: |
          VERSION=$(node -e "console.log(require('./vdw-vscode/package.json').version)")
          mv vdw-vscode/vdw-vscode-*.vsix "vdw_vscode_${VERSION}.vsix"
      - uses: actions/upload-artifact@v4
        with:
          name: vdw-vscode-vsix
          path: vdw_vscode_*.vsix
```

---

## 5. 跨平台扩展架构

`package_vdw.sh` 接收 `os` + `arch` 参数，内部根据参数选择打包格式：

| OS | 打包格式 | 工具 |
|----|---------|------|
| `darwin` | `.tar.gz`（基础）/ `.dmg`（增强） | `tar` + `hdiutil` |
| `linux` | `.tar.gz` | `tar` |
| `win32` | `.zip` | `zip` |

未来新增平台只需：
1. 在 `package_vdw.sh` 的 case 分支新增格式处理
2. 在 `package_all.sh` 新增对应平台的入口
3. 在 CI matrix 中新增 target

无需修改命名规范或其他模块。

---

## 6. 产物目录

打包产物统一输出到 `dist/`（项目根）：

```
dist/
├── vdw_darwin_arm64_0.5.0.dmg
├── vdw_darwin_arm64_0.5.0.tar.gz
├── vdw_darwin_x86_64_0.5.0.dmg
├── vdw_darwin_x86_64_0.5.0.tar.gz
├── vdw_linux_x86_64_0.5.0.tar.gz
├── vdw_linux_arm64_0.5.0.tar.gz
├── vdw_win32_x86_64_0.5.0.zip
└── vdw_vscode_0.5.0.vsix
```

`.gitignore` 中添加 `dist/`。

---

## 7. 验收标准

### AC-1: 命名规范统一
- [ ] `vdw` CLI 产物名遵循 `vdw_{os}_{arch}_{version}.{ext}`
- [ ] vdw-vscode VSIX 产物名遵循 `vdw_vscode_{version}.vsix`
- [ ] CI release.yml 产物与本地打包命名一致

### AC-2: macOS DMG 安装包
- [ ] 执行 `scripts/package_vdw_dmg.sh arm64` 生成 `dist/vdw_darwin_arm64_0.5.0.dmg`
- [ ] `.dmg` 可挂载，包含 `vdw` 二进制文件
- [ ] 安装后 `vdw --version` 输出正确版本号

### AC-3: scripts/ 目录
- [ ] 项目根 `scripts/` 目录存在，含 `package_vdw.sh` / `package_vdw_dmg.sh` / `package_vscode.sh` / `package_all.sh` / `common.sh`
- [ ] 所有脚本 `set -euo pipefail`，有 `--help` 选项

### AC-4: 一键打包
- [ ] `scripts/package_all.sh` 在 macOS 上生成 DMG + VSIX
- [ ] 产物统一输出到 `dist/`

### AC-5: 跨平台预留
- [ ] `package_vdw.sh` 支持 `linux` / `win32` 参数（即使在 macOS 上调用也给出清晰提示）
- [ ] `package_all.sh` 根据 `uname` 自动选择打包格式

### AC-6: 不影响现有流程
- [ ] `vdw/Makefile` 现有 target 行为不变
- [ ] `vdw-vscode/package.json` scripts 不变
- [ ] 现有 `install.sh` 功能不受影响

---

## 8. 风险与注意事项

| 风险 | 等级 | 应对 |
|------|------|------|
| DMG 构建依赖 `hdiutil`（仅 macOS） | 低 | `package_vdw_dmg.sh` 仅限 macOS 调用，其他平台跳过 |
| Cargo.toml / package.json 版本不同步 | 中 | 在 `package_all.sh` 增加版本一致性检查 |
| `hdiutil` create 失败（磁盘空间不足） | 低 | 脚本中增加磁盘空间预检 |
| VSIX 命名含下划线影响 vsce 兼容性 | 低 | 仅重命名最终产物，不改变 `vsce package` 内部命名 |

---

## 9. 变更记录

| 版本 | 日期 | 变更内容 |
|------|------|---------|
| v1.0 | 2026-06-05 | 初始版本：统一命名规范、macOS DMG 打包、scripts/ 目录、跨平台扩展架构 |

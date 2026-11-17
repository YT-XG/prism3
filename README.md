# Prism3

Windows 桌面应用脚手架：Rust 后端（Tauri v2）+ React / TypeScript / Tailwind CSS v4 / shadcn/ui 前端。

当前是**空壳 + 常用基础设施**，不含业务功能。

## 环境要求

| 依赖 | 说明 |
| --- | --- |
| Node.js ≥ 20 | 已用 24.x 验证 |
| pnpm | 包管理器 |
| Rust | **必须是 MSVC 工具链**（Tauri 在 Windows 上的官方推荐） |
| Microsoft C++ Build Tools | 勾选 "Desktop development with C++"（含 Windows SDK） |
| Microsoft Edge WebView2 Runtime | Windows 10 1803+ 已内置 |

### Rust 工具链

全局默认仍是 GNU 工具链，本项目通过 **目录级 override** 使用 MSVC，互不影响：

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc
rustup override set stable-x86_64-pc-windows-msvc   # 在仓库根目录执行
```

`rustup override list` 可查看当前生效的 override。

### crates.io 镜像

官方稀疏索引在国内拉取极慢，仓库根目录已通过 `.cargo/config.toml` 把 `crates-io` 替换为 rsproxy 镜像，仅对本项目生效。删除该文件即回退官方源；若想让所有 Rust 项目都走镜像，把同样内容放到 `%USERPROFILE%\.cargo\config.toml`。

## 常用命令

```powershell
pnpm install            # 安装前端依赖
pnpm tauri dev          # 开发模式（Vite devServer + Rust 热重载）
pnpm build              # 仅前端：tsc 类型检查 + vite 构建
pnpm tauri build        # 打包，产物见下

cd src-tauri
cargo check             # 仅检查 Rust 代码
cargo fmt / cargo clippy
```

打包产物：

- 可执行文件 `src-tauri\target\release\prism3.exe`
- 安装包 `src-tauri\target\release\bundle\nsis\*-setup.exe`

## 运行时数据位置

路径基于应用标识 `com.prism3.desktop`：

| 内容 | 路径 |
| --- | --- |
| 配置文件 | `%APPDATA%\com.prism3.desktop\config.json` |
| 日志文件 | `%LOCALAPPDATA%\com.prism3.desktop\logs\prism3.log` |
| 更新暂存 | `%LOCALAPPDATA%\com.prism3.desktop\cache\update-staging\<version>\`（下载中的 `.part` 与校验通过的 `*-setup.exe`） |

日志超过 5 MB 会轮转，历史文件带日期后缀保留（`RotationStrategy::KeepAll`）。
默认级别在 [lib.rs](src-tauri/src/lib.rs) 的 `tauri_plugin_log::Builder` 里配置：全局 `Info`，本应用模块 `prism3_lib` 为 `Debug`。

## 目录约定

```
src/                    React 前端
  components/ui/        shadcn/ui 组件（由 CLI 生成，可改）
  components/app/       应用级组合组件（含自定义标题栏 title-bar.tsx）
  hooks/                React hooks
  lib/ipc.ts            Rust 命令的类型化封装（改命令时同步这里）
  stores/               zustand 全局状态
src-tauri/src/          Rust 后端
  lib.rs                Builder 组装：插件 / 托盘 / 状态 / 命令
  error.rs              统一错误类型 AppError
  config.rs             AppConfig 持久化
  tray.rs               系统托盘
  updater.rs            自动更新：清单 / 下载 / SHA-256 / NSIS 静默安装
  commands.rs           暴露给前端的 #[tauri::command]
```

### 新增一个 Rust 命令

1. 在 `src-tauri/src/commands.rs` 写 `#[tauri::command]` 函数，错误统一返回 `AppResult<T>`；
2. 注册到 `lib.rs` 的 `generate_handler!`；
3. 在 `src/lib/ipc.ts` 补一个封装；
4. 若用到了新插件的 JS 能力，在 `src-tauri/capabilities/default.json` 加对应 `permission`。

## 内置基础设施

- **配置持久化**：Rust 侧强类型 `AppConfig`，写盘采用「临时文件 + rename」原子替换；前端通过 `get_config` / `set_config` / `reset_config` 读写，单一数据源。
- **日志**：`tauri-plugin-log` 作为唯一日志出口，Rust 用 `log::info!` 等宏、前端用 `@tauri-apps/plugin-log`，两者的记录都写进同一个 `prism3.log`。`attachConsole()` 可在 devtools 里查看 Rust 日志。
- **错误处理**：`AppError`（thiserror）手写 `Serialize`，可直接作为命令返回类型。
- **自定义标题栏**：窗口为无边框（`decorations: false`），标题栏由前端自绘，一体式外壳（标题栏与侧边栏同色连成 L 形）。拖动、双击最大化由 `data-tauri-drag-region="deep"` 提供，窗口三键调用 `@tauri-apps/api/window`，所需权限见 `src-tauri/capabilities/default.json`。原「设置 / 切换主题」已从原生菜单移到标题栏按钮。
- **系统托盘**：显示主窗口 / 退出，左键单击唤起窗口。
- **窗口状态记忆**：`tauri-plugin-window-state` 记住位置与尺寸。
- **单实例保护**：重复启动时唤起已有窗口。
- **自动更新**：发布版启动 3s 后静默检查；Gitee 锚点只放「跳板指针」，真正的清单与安装包在第三方服务器（Cloudflare R2），下载后经 SHA-256 校验，用 NSIS `/S /UPDATE /R` 静默覆盖安装并自动重启；GitHub Release 的同名清单作兜底。开发模式不检查、也不安装（联调设 `PRISM3_UPDATE_ANCHOR`）。原理、清单格式、发版与排错见 [docs/自动更新.md](../docs/自动更新.md)。

## 发版（自动更新）

1. 三处版本号一起改：`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`。
2. 提交后打 tag：

   ```powershell
   git add -A
   git commit -m "chore: v0.2.0"
   git tag v0.2.0
   git push origin main --tags
   ```

3. GitHub Actions（[.github/workflows/release.yml](.github/workflows/release.yml)）会构建 NSIS 安装包，
   发布到 GitHub Release，上传到 R2，并把跳板指针推到 Gitee；详细的前置配置（secrets / variables）、
   清单格式与验收步骤见 [docs/自动更新.md](../docs/自动更新.md)。
4. 本地生成清单（自托管 / 联调）：

   ```powershell
   pnpm manifest -- --base https://cdn.example.com/prism3 --version 0.2.0
   ```

## 替换应用图标

准备 1024×1024 的 PNG，然后：

```powershell
pnpm tauri icon path\to\icon.png
```

会自动重新生成 `src-tauri/icons/` 下的全套图标。

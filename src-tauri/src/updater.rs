//! 应用自动更新：Gitee 锚点 → 第三方服务器 → NSIS 静默安装。
//!
//! ## 为什么自建，而不是 `tauri-plugin-updater`
//!
//! 官方插件要求 minisign 公私钥、清单格式也不同，而且没有本仓库想要的
//! `redirect` 语义（prism2 的 `latest.json` 契约，见下）。这里沿用 prism2 的
//! 「Gitee 跳板 + 第三方服务器 + SHA-256」方案，安装层换成 Tauri 的 NSIS：
//!
//! ```text
//! Gitee raw latest.json
//!   └─ redirect（≤3 层）→ 第三方 latest.json
//!        └─ binaries[platform=win] → 下载 → SHA-256 → 保存到 cache
//!             └─ 用户点「安装」→ installer.exe /S /UPDATE /R → app.exit(0)
//!                  └─ NSIS 静默覆盖安装 → /R 自动拉起新版
//! ```
//!
//! ## 清单契约（与 prism2 兼容，未知字段忽略）
//!
//! ```json
//! {
//!   "version": "0.2.0",
//!   "notes": "",
//!   "redirect": null,
//!   "mirrors": [],
//!   "binaries": [
//!     { "platform": "win", "arch": "x64", "url": "https://…/Prism3_0.2.0_x64-setup.exe", "sha256": "…" }
//!   ]
//! }
//! ```
//!
//! - Gitee 锚点只放「指针」：`{version, redirect: "<第三方>/latest.json", binaries: []}`；
//! - GitHub 兜底清单是**自包含**的（`url` 指向 GitHub Release）。
//! - `sha256` **必填**：缺失即失败关闭（比 prism2 的「有则校验」更严格）。
//!
//! ## 状态机与线程模型
//!
//! - 检查/下载全在独立 `std::thread` 里跑（与 `file_index.rs` 一致）；命令本身只改状态、
//!   起线程后立即返回，进度经 `update:state` 事件广播。
//! - 用 `generation: AtomicU64` 作废旧线程：cancel / 新的 check / resume 都会自增；
//!   所有状态写入都先核对代号，过期线程不得覆盖新状态（prism2 的 `checkSeq` 同款）。
//! - 暂停保留 `.part` 与已下载字节；恢复用 `Range` 续传（服务器不支持 Range 时从头下）。
//!
//! ## 开发模式
//!
//! `cfg!(debug_assertions)` 下默认不自动检查、也**不执行安装**（避免 `tauri dev` 里
//! 真去覆盖一个已安装版本）。要联调检查/下载，设置环境变量 `PRISM3_UPDATE_ANCHOR`
//! 指向本地清单即可（发布版也认，方便做完整安装链路测试；仍强制 HTTPS，回环 HTTP 例外）。

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};

/// 更新状态广播的事件名（载荷与 `get_update_state` 返回值同构）。
pub const UPDATE_EVENT: &str = "update:state";

/// Gitee 锚点仓库（owner/repo）——构建期常量，**不读持久化设置**。
/// 必须与 CI 的 `GITEE_ANCHOR_REPO` secret 完全一致；Gitee 没有 GitHub 的
/// 「latest」别名，固定走 `raw/master/latest.json`（CI 每次发版 force-push 覆盖）。
const GITEE_ANCHOR_REPO: &str = "yt-xg/prism3";
const GITEE_ANCHOR_BRANCH: &str = "master";
const GITEE_ANCHOR_FILE: &str = "latest.json";

/// GitHub 兜底仓库（owner/repo）：`releases/latest/download/latest.json` 永远指向
/// 最新正式 release，其清单自包含（二进制指向 GitHub Release）。
const UPDATE_ANCHOR_REPO: &str = "YT-XG/prism3";

/// 测试/镜像用的源地址覆盖：非空时**只**用该地址（不读 Gitee/GitHub）。
const ANCHOR_ENV: &str = "PRISM3_UPDATE_ANCHOR";

/// 发布版启动后延迟检查，避开首帧与启动阶段的磁盘 / 网络竞争。
const STARTUP_DELAY: Duration = Duration::from_secs(3);
/// 清单请求的总超时（只有几十 KB，30s 足够）。
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(30);
/// 安装包下载的总超时（兜底：连接卡死时不会永久挂起线程）。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// 清单体积上限，防止被恶意/损坏的响应撑爆内存。
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
/// 安装包体积上限（Tauri NSIS 安装包通常只有几 MB）。
const MAX_INSTALLER_BYTES: u64 = 256 * 1024 * 1024;
/// `redirect` 最多跟随的层数。
const REDIRECT_LIMIT: usize = 3;
/// 下载进度的广播节流：每 100ms 最多一次。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

const DEV_MESSAGE: &str =
    "开发模式不检查更新；设置 PRISM3_UPDATE_ANCHOR 指向本地 latest.json 可联调检查与下载";

/// 更新状态。序列化为 camelCase，前端 `UpdateStatus` 联合类型与之一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateStatus {
    /// 尚未检查 / 已取消 / 开发模式拒绝。
    #[default]
    Idle,
    /// 正在拉清单 / 比版本。
    Checking,
    /// 已是最新。
    UpToDate,
    /// 发现新版本（紧接着会自动开始下载）。
    Available,
    /// 正在下载（`progress` 有值）。
    Downloading,
    /// 下载已暂停（`.part` 与已下载字节保留）。
    Paused,
    /// 已下载并通过 SHA-256，可安装。
    Downloaded,
    /// 检查 / 下载 / 校验 / 安装失败。
    Error,
}

/// 更新状态快照（命令返回值与 `update:state` 事件载荷同构）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    pub status: UpdateStatus,
    /// 当前应用版本（`tauri.conf.json` 的 version）。
    pub current_version: String,
    /// 目标版本（Available / Downloading / Paused / Downloaded / Error 时有值）。
    pub version: Option<String>,
    /// 发布说明（清单里的 `notes`）。
    pub notes: Option<String>,
    /// 下载进度 0-100；总长度未知时为 `None`。
    pub progress: Option<u32>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    /// 失败原因（`error` 状态）。
    pub error: Option<String>,
    /// 补充说明（开发模式提示 / 已取消 / 已是最新）。
    pub message: Option<String>,
    /// 已下载安装包的本地路径（供「打开安装包」兜底）。
    pub installer_path: Option<String>,
    /// 安装包下载链接（下载失败时供「下载安装包」兜底）。
    pub download_url: Option<String>,
}

/// 一个平台二进制项（字段名与 prism2 的 `latest.json` 一致）。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct UpdateBinary {
    platform: String,
    /// 仅保留给未来多架构选择：Windows 目前只取第一个 `win` 项。
    #[allow(dead_code)]
    arch: String,
    url: String,
    sha256: String,
}

/// 更新清单（`latest.json`）。未知字段（例如 prism2 的 `mirrors`）会被忽略。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct UpdateManifest {
    version: String,
    notes: Option<String>,
    /// 指向下一份清单的完整 URL；非空且非自指时跟随（深度 ≤ `REDIRECT_LIMIT`）。
    redirect: Option<String>,
    binaries: Vec<UpdateBinary>,
}

/// 本次检查/下载的取消与暂停开关。暂停与取消是两回事：
/// 暂停保留 `.part`，取消则让线程把自己的 `.part` 删掉。
#[derive(Debug, Default)]
struct RunControl {
    cancelled: AtomicBool,
    paused: AtomicBool,
}

#[derive(Default)]
struct Inner {
    status: UpdateStatus,
    version: Option<String>,
    notes: Option<String>,
    progress: Option<u32>,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    error: Option<String>,
    message: Option<String>,
    installer_path: Option<PathBuf>,
    download_url: Option<String>,
    binary: Option<UpdateBinary>,
    partial_path: Option<PathBuf>,
    final_path: Option<PathBuf>,
    control: Option<Arc<RunControl>>,
}

impl Inner {
    fn to_snapshot(&self, current_version: &str) -> UpdateSnapshot {
        UpdateSnapshot {
            status: self.status,
            current_version: current_version.to_string(),
            version: self.version.clone(),
            notes: self.notes.clone(),
            progress: self.progress,
            downloaded_bytes: self.downloaded_bytes,
            total_bytes: self.total_bytes,
            error: self.error.clone(),
            message: self.message.clone(),
            installer_path: self
                .installer_path
                .as_ref()
                .map(|path| path.display().to_string()),
            download_url: self.download_url.clone(),
        }
    }
}

/// 由 Tauri 托管的更新状态（应用级单例）。
pub struct UpdateState {
    current_version: String,
    inner: Mutex<Inner>,
    /// 每次 check / cancel / resume 自增；在途线程写入状态前必须核对它。
    generation: AtomicU64,
}

impl UpdateState {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            current_version: app.package_info().version.to_string(),
            inner: Mutex::new(Inner::default()),
            generation: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 当前快照（命令返回值与事件载荷共用）。
    pub fn snapshot(&self) -> UpdateSnapshot {
        self.lock().to_snapshot(&self.current_version)
    }

    /// 在代号仍有效时修改状态；代号过期（被取消/被新一轮取代）时返回 `None`。
    fn mutate(&self, generation: u64, update: impl FnOnce(&mut Inner)) -> Option<UpdateSnapshot> {
        {
            let mut inner = self.lock();
            if self.generation.load(Ordering::SeqCst) != generation {
                return None;
            }
            update(&mut inner);
        }
        Some(self.snapshot())
    }

    fn is_busy(&self) -> bool {
        matches!(
            self.snapshot().status,
            UpdateStatus::Checking | UpdateStatus::Downloading | UpdateStatus::Paused
        )
    }
}

/// 托管更新状态。必须在 `schedule_startup_check` 之前调用。
pub fn init(app: &AppHandle) {
    app.manage(UpdateState::new(app));
}

/// 发布版启动 3s 后静默检查一次（开发模式且未设 `PRISM3_UPDATE_ANCHOR` 时跳过）。
pub fn schedule_startup_check(app: &AppHandle) {
    if !can_check() {
        log::debug!("开发模式：跳过启动时自动检查更新");
        return;
    }

    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("prism3-update-startup".into())
        .spawn(move || {
            std::thread::sleep(STARTUP_DELAY);
            check(&app, false);
        });
    if let Err(err) = spawned {
        log::warn!("启动更新检查线程失败：{err}");
    }
}

/// 当前快照。
pub fn snapshot(app: &AppHandle) -> UpdateSnapshot {
    app.state::<UpdateState>().snapshot()
}

/// 检查更新；`force` 为真时跳过版本比较并强制重新下载（「重新安装最新版本」）。
///
/// 命令立即返回「当前快照」，真正的网络请求与下载在后台线程里进行，结果经
/// `update:state` 广播。
pub fn check(app: &AppHandle, force: bool) -> UpdateSnapshot {
    let state = app.state::<UpdateState>();

    if !can_check() {
        let generation = state.generation.load(Ordering::SeqCst);
        if let Some(snapshot) = state.mutate(generation, |inner| {
            inner.status = UpdateStatus::Idle;
            inner.message = Some(DEV_MESSAGE.to_string());
            inner.error = None;
        }) {
            broadcast(app, &snapshot);
        }
        return state.snapshot();
    }

    if state.is_busy() {
        return state.snapshot();
    }

    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    if let Some(snapshot) = state.mutate(generation, |inner| {
        inner.status = UpdateStatus::Checking;
        inner.version = None;
        inner.notes = None;
        inner.progress = None;
        inner.downloaded_bytes = 0;
        inner.total_bytes = None;
        inner.error = None;
        inner.message = None;
        inner.installer_path = None;
        inner.download_url = None;
        inner.binary = None;
        inner.partial_path = None;
        inner.final_path = None;
        inner.control = Some(Arc::new(RunControl::default()));
    }) {
        broadcast(app, &snapshot);
    }

    let app_handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("prism3-update-check".into())
        .spawn(move || run_check(app_handle, generation, force));
    if let Err(err) = spawned {
        fail(
            app,
            generation,
            format!("启动更新检查线程失败：{err}"),
            None,
        );
    }

    state.snapshot()
}

/// 暂停下载：保留 `.part` 与已下载字节，等待 `resume`。
pub fn pause(app: &AppHandle) -> UpdateSnapshot {
    let state = app.state::<UpdateState>();
    let snapshot = {
        let mut inner = state.lock();
        if inner.status != UpdateStatus::Downloading {
            return inner.to_snapshot(&state.current_version);
        }
        if let Some(control) = &inner.control {
            control.paused.store(true, Ordering::SeqCst);
        }
        inner.status = UpdateStatus::Paused;
        inner.message = None;
        inner.to_snapshot(&state.current_version)
    };
    broadcast(app, &snapshot);
    snapshot
}

/// 恢复暂停的下载：从 `.part` 的当前大小发 `Range` 续传。
pub fn resume(app: &AppHandle) -> UpdateSnapshot {
    let state = app.state::<UpdateState>();
    // 先作废旧线程（暂停线程可能还在退出途中），再改状态。
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;

    let (snapshot, task) = {
        let mut inner = state.lock();
        if inner.status != UpdateStatus::Paused {
            return inner.to_snapshot(&state.current_version);
        }

        let task = match (
            inner.binary.clone(),
            inner.version.clone(),
            inner.partial_path.clone(),
            inner.final_path.clone(),
        ) {
            (Some(binary), Some(version), Some(partial), Some(final_path)) => {
                let control = Arc::new(RunControl::default());
                let start = fs::metadata(&partial).map(|meta| meta.len()).unwrap_or(0);
                inner.status = UpdateStatus::Downloading;
                inner.error = None;
                inner.message = None;
                inner.downloaded_bytes = start;
                inner.progress = inner.total_bytes.map(|total| percent_of(start, total));
                inner.control = Some(control.clone());
                Some((binary, version, partial, final_path, control))
            }
            _ => None,
        };

        match task {
            Some(task) => (inner.to_snapshot(&state.current_version), Some(task)),
            None => {
                inner.status = UpdateStatus::Error;
                inner.error = Some("下载状态已丢失，请重新检查更新".into());
                inner.control = None;
                (inner.to_snapshot(&state.current_version), None)
            }
        }
    };
    broadcast(app, &snapshot);

    let Some((binary, version, partial, final_path, control)) = task else {
        return snapshot;
    };

    let app_handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("prism3-update-resume".into())
        .spawn(move || {
            let outcome = run_download(
                &app_handle,
                generation,
                &binary,
                &partial,
                &final_path,
                control,
            );
            finish_download(
                &app_handle,
                generation,
                &version,
                &binary,
                &final_path,
                outcome,
            );
        });
    if let Err(err) = spawned {
        fail(app, generation, format!("启动下载线程失败：{err}"), None);
    }

    snapshot
}

/// 取消在途检查/下载：作废代号、删掉 `.part`、回到可重试状态。
pub fn cancel(app: &AppHandle) -> UpdateSnapshot {
    let state = app.state::<UpdateState>();
    // 先自增代号：此后任何在途线程的写入都会被 `mutate` 丢弃。
    state.generation.fetch_add(1, Ordering::SeqCst);

    let snapshot = {
        let mut inner = state.lock();
        if !matches!(
            inner.status,
            UpdateStatus::Checking | UpdateStatus::Downloading | UpdateStatus::Paused
        ) {
            return inner.to_snapshot(&state.current_version);
        }
        if let Some(control) = &inner.control {
            control.cancelled.store(true, Ordering::SeqCst);
        }
        inner.status = if inner.version.is_some() {
            UpdateStatus::Available
        } else {
            UpdateStatus::Idle
        };
        inner.progress = None;
        inner.downloaded_bytes = 0;
        inner.total_bytes = None;
        inner.partial_path = None;
        inner.control = None;
        inner.error = None;
        inner.message = Some("已取消".to_string());
        inner.to_snapshot(&state.current_version)
    };
    broadcast(app, &snapshot);
    snapshot
}

/// 安装已下载的更新并退出应用。
///
/// 只接受 `downloaded` 状态；开发模式拒绝（见模块头的「开发模式」）。
/// 安装器以 `/S /UPDATE /R` 启动后立即 `app.exit(0)`：
/// - `/S` 静默，`/UPDATE` 不重建快捷方式，`/R` 安装成功后自动拉起新版；
/// - 不使用 `/D=`：NSIS 的 `/D=` 不能带引号，Rust `Command` 会自动加引号；
///   Tauri 安装器本来就会从注册表恢复上次安装目录。
pub fn install(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<UpdateState>();
    let (status, installer) = {
        let inner = state.lock();
        (inner.status, inner.installer_path.clone())
    };

    if status != UpdateStatus::Downloaded {
        return Err(AppError::Other("更新尚未下载完成".into()));
    }
    if !install_allowed() {
        return Err(AppError::Other(
            "开发模式不执行安装；请用发布版验证（检查/下载可继续用 PRISM3_UPDATE_ANCHOR 联调）"
                .into(),
        ));
    }
    let Some(installer) = installer else {
        return Err(AppError::Other("安装包路径缺失，请重新检查更新".into()));
    };
    if !installer.is_file() {
        return Err(AppError::Other("安装包已不存在，请重新检查更新".into()));
    }

    spawn_installer(&installer)?;
    log::info!("更新安装器已启动，应用即将退出：{}", installer.display());
    app.exit(0);
    Ok(())
}

/// 在资源管理器中定位已下载的安装包（安装失败兜底）。
pub fn open_installer_folder(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<UpdateState>();
    let installer = {
        let inner = state.lock();
        inner.installer_path.clone()
    };
    let Some(installer) = installer else {
        return Err(AppError::Other("暂未下载安装包".into()));
    };
    if !installer.exists() {
        return Err(AppError::Other("安装包已不存在，请重新检查更新".into()));
    }
    app.opener()
        .reveal_item_in_dir(&installer)
        .map_err(|err| AppError::Other(format!("打开安装包位置失败：{err}")))
}

/// 用系统浏览器打开当前安装包的下载地址（下载失败兜底）。
pub fn open_download_url(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<UpdateState>();
    let url = {
        let inner = state.lock();
        inner.download_url.clone()
    };
    let Some(url) = url else {
        return Err(AppError::Other("下载链接不可用，请重新检查更新".into()));
    };
    let url = ensure_allowed_url(&url).map_err(AppError::Other)?;
    app.opener()
        .open_url(url.to_string(), None::<&str>)
        .map_err(|err| AppError::Other(format!("打开下载链接失败：{err}")))
}

// ---------------------------------------------------------------------------
// 检查与下载
// ---------------------------------------------------------------------------

fn run_check(app: AppHandle, generation: u64, force: bool) {
    let state = app.state::<UpdateState>();
    let current_version = state.current_version.clone();
    log::info!("开始检查更新（当前版本 {current_version}，force={force}）");

    let manifest = match resolve_manifest(&manifest_sources(), fetch_manifest) {
        Ok(manifest) => manifest,
        Err(err) => {
            fail(&app, generation, err, None);
            return;
        }
    };

    let version = manifest
        .version
        .trim()
        .trim_start_matches('v')
        .trim()
        .to_string();
    if version.is_empty() {
        fail(&app, generation, "更新清单缺少版本号".into(), None);
        return;
    }

    if !force && !is_newer(&version, &current_version) {
        log::info!("已是最新版本：清单 {version}，当前 {current_version}");
        if let Some(snapshot) = state.mutate(generation, |inner| {
            inner.status = UpdateStatus::UpToDate;
            inner.version = None;
            inner.error = None;
            inner.message = Some("已是最新版本".to_string());
            inner.progress = None;
            inner.downloaded_bytes = 0;
            inner.total_bytes = None;
        }) {
            broadcast(&app, &snapshot);
        }
        return;
    }

    let Some(mut binary) = pick_win_binary(&manifest).cloned() else {
        fail(
            &app,
            generation,
            "更新清单中没有 Windows 安装包".into(),
            None,
        );
        return;
    };
    if binary.sha256.trim().is_empty() {
        fail(
            &app,
            generation,
            "更新清单缺少 sha256，已拒绝安装".into(),
            Some(binary.url.clone()),
        );
        return;
    }
    let Ok(url) = ensure_allowed_url(&binary.url) else {
        fail(
            &app,
            generation,
            format!("更新清单里的下载地址不合法：{}", binary.url),
            None,
        );
        return;
    };
    binary.url = url.to_string();

    let Ok(cache_dir) = app.path().app_cache_dir() else {
        fail(&app, generation, "无法定位应用缓存目录".into(), None);
        return;
    };
    let version_dir = cache_dir
        .join("update-staging")
        .join(sanitize_version_for_path(&version));
    if let Err(err) = fs::create_dir_all(&version_dir) {
        fail(
            &app,
            generation,
            format!("创建更新暂存目录失败：{err}"),
            None,
        );
        return;
    }

    let file_name = basename_from_url(&binary.url)
        .filter(|name| name.to_ascii_lowercase().ends_with(".exe"))
        .unwrap_or_else(|| {
            format!(
                "Prism3_{}_x64-setup.exe",
                sanitize_version_for_path(&version)
            )
        });
    let final_path = version_dir.join(&file_name);

    // 已经下载过同一个包（且校验通过）时直接进入「可安装」，不重复下载。
    if !force && final_path.is_file() {
        if let Ok(hash) = sha256_file(&final_path) {
            if hash.eq_ignore_ascii_case(binary.sha256.trim()) {
                let size = fs::metadata(&final_path)
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                if let Some(snapshot) = state.mutate(generation, |inner| {
                    inner.status = UpdateStatus::Downloaded;
                    inner.version = Some(version.clone());
                    inner.notes = manifest.notes.clone();
                    inner.progress = Some(100);
                    inner.downloaded_bytes = size;
                    inner.total_bytes = Some(size);
                    inner.error = None;
                    inner.message = None;
                    inner.installer_path = Some(final_path.clone());
                    inner.download_url = Some(binary.url.clone());
                    inner.binary = Some(binary.clone());
                    inner.final_path = Some(final_path.clone());
                    inner.partial_path = None;
                    inner.control = None;
                }) {
                    broadcast(&app, &snapshot);
                }
                prune_staging(&app, &sanitize_version_for_path(&version));
                return;
            }
        }
    }

    if force {
        let _ = fs::remove_file(&final_path);
    }
    // 本次是新下载（非续传）：清掉本版本目录里遗留的 `.part`。
    remove_partials(&version_dir);
    let partial_path = version_dir.join(format!("{file_name}.{generation}.part"));
    let control = Arc::new(RunControl::default());

    if let Some(snapshot) = state.mutate(generation, |inner| {
        inner.status = UpdateStatus::Available;
        inner.version = Some(version.clone());
        inner.notes = manifest.notes.clone();
        inner.progress = None;
        inner.downloaded_bytes = 0;
        inner.total_bytes = None;
        inner.error = None;
        inner.message = None;
        inner.installer_path = None;
        inner.download_url = Some(binary.url.clone());
        inner.binary = Some(binary.clone());
        inner.partial_path = Some(partial_path.clone());
        inner.final_path = Some(final_path.clone());
        inner.control = Some(control.clone());
    }) {
        broadcast(&app, &snapshot);
    } else {
        return;
    }

    if let Some(snapshot) = state.mutate(generation, |inner| {
        inner.status = UpdateStatus::Downloading;
        inner.progress = Some(0);
    }) {
        broadcast(&app, &snapshot);
    } else {
        return;
    }

    let outcome = run_download(
        &app,
        generation,
        &binary,
        &partial_path,
        &final_path,
        control,
    );
    finish_download(&app, generation, &version, &binary, &final_path, outcome);
}

#[derive(Debug)]
enum DownloadOutcome {
    Completed,
    Paused,
    Cancelled,
    Failed(String),
}

/// 下载安装包到 `.part`，校验 SHA-256，再重命名成最终文件。
///
/// `start` 从 `.part` 现有大小推算：>0 时发 `Range` 续传；服务器返回 200（不支持
/// Range）或 416（区间不可用）时从头重下。
fn run_download(
    app: &AppHandle,
    generation: u64,
    binary: &UpdateBinary,
    partial_path: &Path,
    final_path: &Path,
    control: Arc<RunControl>,
) -> DownloadOutcome {
    let state = app.state::<UpdateState>();
    let expected = binary.sha256.trim().to_ascii_lowercase();
    let mut start = fs::metadata(partial_path)
        .map(|meta| meta.len())
        .unwrap_or(0);

    let client = match http_client(DOWNLOAD_TIMEOUT) {
        Ok(client) => client,
        Err(err) => return DownloadOutcome::Failed(err),
    };

    let request = |start: u64| -> Result<reqwest::blocking::Response, String> {
        let mut request = client.get(&binary.url);
        if start > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={start}-"));
        }
        request
            .send()
            .map_err(|err| format!("下载安装包失败：{err}"))
    };

    let mut response = match request(start) {
        Ok(response) => response,
        Err(err) => return DownloadOutcome::Failed(err),
    };

    if start > 0 && response.status().as_u16() == 416 {
        // 部分文件与服务器对不上（服务端对象已换）：丢掉旧文件从头来。
        drop(response);
        let _ = fs::remove_file(partial_path);
        start = 0;
        response = match request(0) {
            Ok(response) => response,
            Err(err) => return DownloadOutcome::Failed(err),
        };
    }
    if start > 0 && response.status().as_u16() == 200 {
        // 服务器忽略了 Range：从头写。
        start = 0;
    }
    if !response.status().is_success() {
        return DownloadOutcome::Failed(format!("下载安装包失败：HTTP {}", response.status()));
    }
    // 跟随重定向后的最终地址也要过协议校验（防止 https 被降级到 http）。
    if ensure_allowed_url(response.url().as_str()).is_err() {
        return DownloadOutcome::Failed("下载地址不是 HTTPS（或不是回环 HTTP），已拒绝".into());
    }

    let total = response.content_length().map(|len| len + start);
    if let Some(total) = total {
        if total > MAX_INSTALLER_BYTES {
            return DownloadOutcome::Failed(format!(
                "安装包超过大小上限（{} MiB）",
                MAX_INSTALLER_BYTES / 1024 / 1024
            ));
        }
    }

    let file = if start > 0 {
        fs::OpenOptions::new().append(true).open(partial_path)
    } else {
        fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(partial_path)
    };
    let mut file = match file {
        Ok(file) => file,
        Err(err) => return DownloadOutcome::Failed(format!("创建安装包文件失败：{err}")),
    };

    let mut downloaded = start;
    let mut last_emit: Option<Instant> = None;
    let mut buffer = vec![0u8; 64 * 1024];

    loop {
        // 取消或代号过期：删掉本次的 `.part`，不写任何状态（cancel 已经写好）。
        if control.cancelled.load(Ordering::SeqCst)
            || state.generation.load(Ordering::SeqCst) != generation
        {
            drop(file);
            let _ = fs::remove_file(partial_path);
            return DownloadOutcome::Cancelled;
        }
        if control.paused.load(Ordering::SeqCst) {
            let _ = file.flush();
            return DownloadOutcome::Paused;
        }

        let read = match response.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(err) => {
                drop(file);
                let _ = fs::remove_file(partial_path);
                return DownloadOutcome::Failed(format!("读取安装包失败：{err}"));
            }
        };

        downloaded += read as u64;
        if downloaded > MAX_INSTALLER_BYTES {
            drop(file);
            let _ = fs::remove_file(partial_path);
            return DownloadOutcome::Failed(format!(
                "安装包超过大小上限（{} MiB）",
                MAX_INSTALLER_BYTES / 1024 / 1024
            ));
        }
        if let Err(err) = file.write_all(&buffer[..read]) {
            drop(file);
            let _ = fs::remove_file(partial_path);
            return DownloadOutcome::Failed(format!("写入安装包失败：{err}"));
        }

        let should_emit = last_emit.is_none_or(|last| last.elapsed() >= PROGRESS_INTERVAL);
        if should_emit {
            last_emit = Some(Instant::now());
            if let Some(snapshot) = state.mutate(generation, |inner| {
                // 只更新字节数/进度，**不改状态**：`pause()` 可能刚把状态写成 Paused，
                // 这里再写 Downloading 会把暂停覆盖掉（线程随后就停，界面却显示下载中）。
                inner.downloaded_bytes = downloaded;
                inner.total_bytes = total;
                inner.progress = total.map(|total| percent_of(downloaded, total));
            }) {
                broadcast(app, &snapshot);
            }
        }
    }

    if let Some(total) = total {
        if downloaded != total {
            drop(file);
            let _ = fs::remove_file(partial_path);
            return DownloadOutcome::Failed("下载不完整（与服务器声明的长度不一致）".into());
        }
    }
    if let Err(err) = file.flush() {
        drop(file);
        let _ = fs::remove_file(partial_path);
        return DownloadOutcome::Failed(format!("写入安装包失败：{err}"));
    }
    drop(file);

    let actual = match sha256_file(partial_path) {
        Ok(hash) => hash,
        Err(err) => {
            let _ = fs::remove_file(partial_path);
            return DownloadOutcome::Failed(format!("校验安装包失败：{err}"));
        }
    };
    if actual != expected {
        let _ = fs::remove_file(partial_path);
        return DownloadOutcome::Failed(format!(
            "SHA-256 校验失败（期望 {expected}，实际 {actual}）"
        ));
    }

    if final_path.exists() {
        let _ = fs::remove_file(final_path);
    }
    if let Err(err) = fs::rename(partial_path, final_path) {
        let _ = fs::remove_file(partial_path);
        return DownloadOutcome::Failed(format!("保存安装包失败：{err}"));
    }

    DownloadOutcome::Completed
}

fn finish_download(
    app: &AppHandle,
    generation: u64,
    version: &str,
    binary: &UpdateBinary,
    final_path: &Path,
    outcome: DownloadOutcome,
) {
    match outcome {
        DownloadOutcome::Completed => {
            let state = app.state::<UpdateState>();
            if let Some(snapshot) = state.mutate(generation, |inner| {
                inner.status = UpdateStatus::Downloaded;
                inner.progress = Some(100);
                inner.error = None;
                inner.message = None;
                inner.installer_path = Some(final_path.to_path_buf());
                inner.partial_path = None;
                inner.control = None;
            }) {
                broadcast(app, &snapshot);
            }
            prune_staging(app, &sanitize_version_for_path(version));
            log::info!("更新已下载并通过 SHA-256 校验：{version}");
        }
        // 暂停状态由 `pause()` 写入；取消状态由 `cancel()` 写入。
        DownloadOutcome::Paused | DownloadOutcome::Cancelled => {}
        DownloadOutcome::Failed(message) => {
            fail(app, generation, message, Some(binary.url.clone()));
        }
    }
}

fn fail(app: &AppHandle, generation: u64, message: String, download_url: Option<String>) {
    log::error!("自动更新失败：{message}");
    let state = app.state::<UpdateState>();
    if let Some(snapshot) = state.mutate(generation, |inner| {
        inner.status = UpdateStatus::Error;
        inner.error = Some(message.clone());
        inner.message = None;
        inner.progress = None;
        inner.downloaded_bytes = 0;
        inner.total_bytes = None;
        inner.partial_path = None;
        inner.control = None;
        if download_url.is_some() {
            inner.download_url = download_url.clone();
        }
    }) {
        broadcast(app, &snapshot);
    }
}

fn broadcast(app: &AppHandle, snapshot: &UpdateSnapshot) {
    if let Err(err) = app.emit(UPDATE_EVENT, snapshot) {
        log::warn!("广播更新状态失败：{err}");
    }
}

// ---------------------------------------------------------------------------
// 安装（Windows NSIS）
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn spawn_installer(installer: &Path) -> AppResult<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    use windows::Win32::System::Threading::CREATE_NO_WINDOW;

    std::process::Command::new(installer)
        // /S 静默、/UPDATE 更新模式（不重建快捷方式）、/R 安装成功后自动拉起新版。
        // 不使用 /D=：NSIS 要求裸写且不能带引号，Tauri 安装器会从注册表恢复安装目录。
        .args(["/S", "/UPDATE", "/R"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .map(|_| ())
        .map_err(|err| AppError::Other(format!("启动更新安装器失败：{err}")))
}

#[cfg(not(windows))]
fn spawn_installer(_installer: &Path) -> AppResult<()> {
    Err(AppError::Other("自动更新仅支持 Windows".into()))
}

// ---------------------------------------------------------------------------
// 清单来源 / 解析
// ---------------------------------------------------------------------------

/// 开发模式是否允许检查（发布版恒允许；开发版需要显式 `PRISM3_UPDATE_ANCHOR`）。
fn can_check() -> bool {
    !cfg!(debug_assertions) || anchor_override().is_some()
}

/// 开发模式是否允许真正安装（避免 `tauri dev` 去覆盖已安装版本）。
fn install_allowed() -> bool {
    !cfg!(debug_assertions)
}

fn anchor_override() -> Option<String> {
    std::env::var(ANCHOR_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn manifest_sources() -> Vec<String> {
    if let Some(anchor) = anchor_override() {
        return vec![anchor];
    }
    vec![
        format!(
            "https://gitee.com/{GITEE_ANCHOR_REPO}/raw/{GITEE_ANCHOR_BRANCH}/{GITEE_ANCHOR_FILE}"
        ),
        format!("https://github.com/{UPDATE_ANCHOR_REPO}/releases/latest/download/latest.json"),
    ]
}

fn http_client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(format!("prism3-updater/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(timeout)
        .build()
        .map_err(|err| format!("初始化 HTTP 客户端失败：{err}"))
}

fn fetch_manifest(url: &str) -> Result<UpdateManifest, String> {
    let client = http_client(MANIFEST_TIMEOUT)?;
    let response = client
        .get(url)
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .send()
        .map_err(|err| format!("请求更新清单失败：{err}"))?;

    // 跟随 HTTP 重定向后的最终地址同样要做协议校验。
    ensure_allowed_url(response.url().as_str())?;
    if !response.status().is_success() {
        return Err(format!("更新源响应失败：HTTP {}", response.status()));
    }

    let mut body = Vec::new();
    read_limited(response, &mut body, MAX_MANIFEST_BYTES)?;
    parse_manifest_bytes(&body)
}

fn parse_manifest_bytes(bytes: &[u8]) -> Result<UpdateManifest, String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    serde_json::from_slice(bytes).map_err(|err| format!("更新清单格式错误：{err}"))
}

fn read_limited<R: Read>(mut reader: R, buffer: &mut Vec<u8>, limit: u64) -> Result<(), String> {
    let mut chunk = [0u8; 8 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|err| format!("读取更新清单失败：{err}"))?;
        if read == 0 {
            return Ok(());
        }
        if buffer.len() as u64 + read as u64 > limit {
            return Err(format!("更新清单超过大小上限（{limit} 字节）"));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

/// 依次尝试各个源；每个源内部跟随 `redirect`，失败才换下一个源。
fn resolve_manifest<F>(sources: &[String], fetch: F) -> Result<UpdateManifest, String>
where
    F: Fn(&str) -> Result<UpdateManifest, String>,
{
    let mut errors = Vec::new();
    for source in sources {
        match resolve_source(source, &fetch) {
            Ok(manifest) => return Ok(manifest),
            Err(err) => {
                log::warn!("更新源不可用，尝试下一个：{source}（{err}）");
                errors.push(format!("{source}：{err}"));
            }
        }
    }
    Err(format!("所有更新源均不可用（{}）", errors.join("；")))
}

fn resolve_source<F>(start: &str, fetch: &F) -> Result<UpdateManifest, String>
where
    F: Fn(&str) -> Result<UpdateManifest, String>,
{
    let mut current = ensure_allowed_url(start)?.to_string();
    let mut seen = HashSet::new();

    for _ in 0..=REDIRECT_LIMIT {
        if !seen.insert(current.clone()) {
            return Err("更新清单重定向出现循环".into());
        }
        let manifest = fetch(&current)?;

        let redirect = manifest
            .redirect
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(redirect) = redirect {
            let target = ensure_allowed_url(redirect)?;
            if target.as_str() == current.as_str() {
                return Err("更新清单指向自身".into());
            }
            current = target.to_string();
            continue;
        }

        if manifest.binaries.is_empty() {
            return Err("更新清单没有可用的安装包".into());
        }
        return Ok(manifest);
    }

    Err("更新清单重定向层数过多".into())
}

/// 只允许 HTTPS；回环地址（本地联调）例外，允许 HTTP。
fn ensure_allowed_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|err| format!("更新地址无效：{err}"))?;
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback(&url) => Ok(url),
        "http" => Err(format!("拒绝非 HTTPS 的更新地址：{url}")),
        other => Err(format!("不支持的更新地址协议：{other}")),
    }
}

fn is_loopback(url: &reqwest::Url) -> bool {
    matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
}

fn pick_win_binary(manifest: &UpdateManifest) -> Option<&UpdateBinary> {
    manifest
        .binaries
        .iter()
        .find(|binary| binary.platform.eq_ignore_ascii_case("win"))
}

/// 清单版本是否比当前版本新。版本串非法时退化为「字符串不相等」。
fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(value: &str) -> Option<Version> {
        Version::parse(value.trim().trim_start_matches('v')).ok()
    }

    match (parse(candidate), parse(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => candidate.trim() != current.trim(),
    }
}

/// 从下载 URL 提取安全的本地文件名（只保留 `[A-Za-z0-9._-]`）。
fn basename_from_url(raw: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw).ok()?;
    let last = url.path_segments()?.rfind(|segment| !segment.is_empty())?;
    let cleaned: String = last
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// 版本号用作目录名时的白名单化（防 `..` / 分隔符注入）。
fn sanitize_version_for_path(version: &str) -> String {
    let cleaned: String = version
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    sha256_reader(File::open(path)?)
}

fn sha256_reader<R: Read>(mut reader: R) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn percent_of(downloaded: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    ((downloaded.min(total) as u128 * 100 / total as u128) as u32).min(100)
}

/// 清掉本版本目录里遗留的 `.part`（新下载不复用旧的部分文件）。
fn remove_partials(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "part") {
            let _ = fs::remove_file(path);
        }
    }
}

/// 清理其它版本的暂存目录，保留当前待安装版本（供「打开安装包」兜底）。
fn prune_staging(app: &AppHandle, keep: &str) {
    let Ok(cache_dir) = app.path().app_cache_dir() else {
        return;
    };
    let root = cache_dir.join("update-staging");
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy() == keep {
            continue;
        }
        let _ = fs::remove_dir_all(entry.path());
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn win_binary() -> UpdateBinary {
        UpdateBinary {
            platform: "win".into(),
            arch: "x64".into(),
            url: "https://cdn.test/Prism3_0.2.0_x64-setup.exe".into(),
            sha256: "ab".repeat(32),
        }
    }

    #[test]
    fn manifest_parses_pointer_and_full_payloads() {
        let pointer: UpdateManifest = serde_json::from_str(
            r#"{"version":"0.2.0","redirect":"https://cdn.test/latest.json","binaries":[]}"#,
        )
        .expect("指针清单应当可解析");
        assert_eq!(pointer.version, "0.2.0");
        assert_eq!(
            pointer.redirect.as_deref(),
            Some("https://cdn.test/latest.json")
        );
        assert!(pointer.binaries.is_empty());

        let full: UpdateManifest = serde_json::from_str(
            r#"{
                "version":"0.2.0",
                "notes":"修复若干问题",
                "redirect":null,
                "mirrors":["https://mirror.test"],
                "binaries":[{"platform":"win","arch":"x64",
                             "url":"https://cdn.test/Prism3_0.2.0_x64-setup.exe",
                             "sha256":"abab"}]
            }"#,
        )
        .expect("全量清单应当可解析（未知字段 mirrors 忽略）");
        assert_eq!(full.notes.as_deref(), Some("修复若干问题"));
        assert_eq!(full.binaries.len(), 1);
        assert_eq!(full.binaries[0].platform, "win");
    }

    #[test]
    fn manifest_bom_is_tolerated() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"version":"0.2.0","binaries":[]}"#);
        let manifest = parse_manifest_bytes(&bytes).expect("带 BOM 的清单应当可解析");
        assert_eq!(manifest.version, "0.2.0");
    }

    #[test]
    fn redirect_chain_follows_to_the_final_manifest() {
        let sources = vec!["https://anchor.test/latest.json".to_string()];
        let fetch = |url: &str| match url {
            "https://anchor.test/latest.json" => Ok(UpdateManifest {
                version: "0.2.0".into(),
                redirect: Some("https://cdn.test/latest.json".into()),
                ..UpdateManifest::default()
            }),
            "https://cdn.test/latest.json" => Ok(UpdateManifest {
                version: "0.2.0".into(),
                binaries: vec![win_binary()],
                ..UpdateManifest::default()
            }),
            other => Err(format!("unexpected url {other}")),
        };

        let manifest = resolve_manifest(&sources, fetch).expect("应当跟随 redirect 拿到全量清单");
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.binaries.len(), 1);
    }

    #[test]
    fn redirect_chain_rejects_self_and_cycles() {
        let sources = vec!["https://anchor.test/latest.json".to_string()];
        let self_reference = |url: &str| {
            assert_eq!(url, "https://anchor.test/latest.json");
            Ok(UpdateManifest {
                version: "0.2.0".into(),
                redirect: Some("https://anchor.test/latest.json".into()),
                ..UpdateManifest::default()
            })
        };
        let err = resolve_manifest(&sources, self_reference).expect_err("自指必须报错");
        assert!(err.contains("所有更新源均不可用"));

        let a_to_b = |url: &str| match url {
            "https://anchor.test/latest.json" => Ok(UpdateManifest {
                version: "0.2.0".into(),
                redirect: Some("https://anchor.test/b.json".into()),
                ..UpdateManifest::default()
            }),
            _ => Ok(UpdateManifest {
                version: "0.2.0".into(),
                redirect: Some("https://anchor.test/latest.json".into()),
                ..UpdateManifest::default()
            }),
        };
        let err = resolve_manifest(&sources, a_to_b).expect_err("环必须报错");
        assert!(err.contains("循环") || err.contains("所有更新源均不可用"));
    }

    #[test]
    fn redirect_chain_has_a_depth_limit() {
        let sources = vec!["https://anchor.test/0".to_string()];
        let fetch = |url: &str| {
            let index: usize = url.rsplit('/').next().unwrap().parse().unwrap();
            Ok(UpdateManifest {
                version: "0.2.0".into(),
                redirect: Some(format!("https://anchor.test/{}", index + 1)),
                ..UpdateManifest::default()
            })
        };
        let err = resolve_manifest(&sources, fetch).expect_err("超层必须报错");
        assert!(err.contains("层数过多") || err.contains("所有更新源均不可用"));
    }

    #[test]
    fn source_fallback_uses_the_next_source() {
        let sources = vec![
            "https://anchor.test/latest.json".to_string(),
            "https://backup.test/latest.json".to_string(),
        ];
        let fetch = |url: &str| match url {
            "https://anchor.test/latest.json" => Err("boom".to_string()),
            "https://backup.test/latest.json" => Ok(UpdateManifest {
                version: "0.2.0".into(),
                binaries: vec![win_binary()],
                ..UpdateManifest::default()
            }),
            other => Err(format!("unexpected url {other}")),
        };
        let manifest = resolve_manifest(&sources, fetch).expect("第一个源失败后应回退第二个源");
        assert_eq!(manifest.binaries.len(), 1);
    }

    #[test]
    fn version_comparison_handles_v_prefix_and_invalid_values() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        // 非法版本串：退化为「字符串不相等」
        assert!(is_newer("nightly", "0.1.0"));
        assert!(!is_newer("nightly", "nightly"));
        // 预发布版本低于同号正式版
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
    }

    #[test]
    fn only_win_binaries_are_selected() {
        let mac_only = UpdateManifest {
            version: "0.2.0".into(),
            binaries: vec![UpdateBinary {
                platform: "mac".into(),
                ..win_binary()
            }],
            ..UpdateManifest::default()
        };
        assert!(pick_win_binary(&mac_only).is_none());

        let manifest = UpdateManifest {
            version: "0.2.0".into(),
            binaries: vec![
                UpdateBinary {
                    platform: "mac".into(),
                    ..win_binary()
                },
                win_binary(),
            ],
            ..UpdateManifest::default()
        };
        assert_eq!(pick_win_binary(&manifest).unwrap().platform, "win");
    }

    #[test]
    fn basename_extraction_is_safe() {
        assert_eq!(
            basename_from_url("https://cdn.test/a/Prism3_0.2.0_x64-setup.exe?v=1").as_deref(),
            Some("Prism3_0.2.0_x64-setup.exe")
        );
        assert_eq!(
            basename_from_url("https://cdn.test/a/..%2F..%2Fevil.exe").as_deref(),
            Some("_2F.._2Fevil.exe")
        );
        assert_eq!(basename_from_url("https://cdn.test/"), None);
        assert_eq!(basename_from_url("not a url"), None);
    }

    #[test]
    fn version_path_segment_is_sanitized() {
        assert_eq!(sanitize_version_for_path("0.2.0"), "0.2.0");
        assert_eq!(sanitize_version_for_path("../../evil"), "evil");
        assert_eq!(sanitize_version_for_path(""), "unknown");
    }

    #[test]
    fn sha256_reader_matches_known_vector() {
        let hash = sha256_reader(Cursor::new(b"abc")).expect("应当可计算 SHA-256");
        assert_eq!(
            hash,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn percent_is_clamped() {
        assert_eq!(percent_of(0, 100), 0);
        assert_eq!(percent_of(50, 100), 50);
        assert_eq!(percent_of(300, 100), 100);
        assert_eq!(percent_of(0, 0), 0);
        assert_eq!(percent_of(u64::MAX, u64::MAX), 100);
    }

    #[test]
    fn read_limited_rejects_oversized_manifests() {
        let mut buffer = Vec::new();
        read_limited(Cursor::new([0u8; 32]), &mut buffer, 32).expect("刚好到上限应当通过");
        assert_eq!(buffer.len(), 32);

        let mut buffer = Vec::new();
        let err =
            read_limited(Cursor::new([0u8; 32]), &mut buffer, 16).expect_err("超过上限必须报错");
        assert!(err.contains("大小上限"));
    }

    #[test]
    fn allowed_urls_are_https_or_loopback_http() {
        assert!(
            ensure_allowed_url("https://gitee.com/yt-xg/prism3/raw/master/latest.json").is_ok()
        );
        assert!(ensure_allowed_url("http://127.0.0.1:8123/latest.json").is_ok());
        assert!(ensure_allowed_url("http://localhost:8123/latest.json").is_ok());
        assert!(ensure_allowed_url("http://example.com/latest.json").is_err());
        assert!(ensure_allowed_url("ftp://example.com/latest.json").is_err());
        assert!(ensure_allowed_url("not a url").is_err());
    }

    #[test]
    fn fetch_manifest_works_over_loopback_http() {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定回环端口");
        let port = listener.local_addr().expect("读取本地地址").port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("接受连接");
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request);
            let body = r#"{"version":"9.9.9","binaries":[{"platform":"win","arch":"x64","url":"http://127.0.0.1/app.exe","sha256":"ab"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).expect("写响应");
        });

        let manifest = fetch_manifest(&format!("http://127.0.0.1:{port}/latest.json"))
            .expect("应当能通过回环 HTTP 拉到清单");
        assert_eq!(manifest.version, "9.9.9");
        assert_eq!(manifest.binaries.len(), 1);
        server.join().expect("服务器线程");
    }
}

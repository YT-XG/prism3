//! 剪贴板历史：采集 → 落盘 → 内存查询 → 状态广播。
//!
//! 设计要点（与 `file_index.rs` 同一套路，只是数据量小得多）：
//! - **按需采集**：只有配置启用时才起采集线程，禁用即停表（沿用既有的「按需采集」模型）。
//! - **变更检测靠剪贴板序列号**：Windows 下每 ~400ms 读一次
//!   `GetClipboardSequenceNumber`，序列号没变就完全不碰剪贴板内容（几乎零开销）。
//! - **查询是纯内存线性扫描**：历史条数在百量级，不需要数据库。
//! - **落盘**：元数据写 `clipboard-v1.json`（原子写），图片写 `clipboard-images\<id>.png`
//!   （另有 `<id>.thumb.png` 缩略图，列表只加载缩略图）。
//! - 采集线程只做「读剪贴板 → 入库 → 落盘 → 广播」，不阻塞 UI 线程。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::config::AppConfig;
use crate::error::{AppError, AppResult};

/// 剪贴板历史变化的广播事件名。
///
/// 广播给所有窗口：主窗口「剪贴板」页与快捷小窗「剪贴板」页共用同一份历史。
pub const CLIPBOARD_EVENT: &str = "clipboard:state";

/// 历史元数据文件名。
const INDEX_FILE_NAME: &str = "clipboard-v1.json";
/// 图片目录名（位于配置目录下）。
const IMAGE_DIR_NAME: &str = "clipboard-images";
/// 落盘格式版本；变更即忽略旧文件重建。
const INDEX_VERSION: u32 = 1;

/// 采集轮询间隔：只读一次序列号，命中变化才真正读内容。
const POLL_INTERVAL: Duration = Duration::from_millis(400);
/// 缩略图最长边（像素）：列表只加载缩略图，原图仅在需要时读。
const THUMB_MAX: u32 = 256;
/// 文本预览长度（字符数）。
const PREVIEW_CHARS: usize = 200;
/// 单条文本的上限：超过就整条丢弃（不截断——截断会让「粘贴出来的内容」与原文不符）。
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
/// 查询的默认条数与硬上限。
const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 500;

/// 一条历史的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipKind {
    #[default]
    Text,
    Image,
}

/// 图片记录的文件信息（相对 `clipboard-images` 目录）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageMeta {
    pub width: u32,
    pub height: u32,
    /// 原图文件名。
    pub file: String,
    /// 缩略图文件名。
    pub thumb: String,
    /// 原图字节数（用于展示占用）。
    pub bytes: u64,
}

/// 一条剪贴板历史（内存 + 持久化）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClipEntry {
    pub id: u64,
    pub kind: ClipKind,
    /// 文本内容（`kind == Text` 时存在）。
    pub text: Option<String>,
    /// 图片信息（`kind == Image` 时存在）。
    pub image: Option<ImageMeta>,
    /// 去重键：文本按字节、图片按尺寸 + 原始像素。
    pub digest: String,
    /// 入库时间（秒级时间戳），保留策略按它计算。
    pub created_at: u64,
    /// 体积（文本为字节数、图片为原图文件字节数）。
    pub size: u64,
}

/// 发给前端的条目元数据：**不含正文**（正文按需单独取，避免广播里塞大文本）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipEntryMeta {
    pub id: u64,
    pub kind: ClipKind,
    /// 列表展示用预览（文本截断 / 图片写尺寸）。
    pub preview: String,
    /// 文本长度；图片为 0。
    pub text_length: usize,
    pub image_width: Option<u32>,
    pub image_height: Option<u32>,
    pub size: u64,
    pub created_at: u64,
}

/// 剪贴板历史状态快照：命令返回值与事件载荷共用同一个结构。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipState {
    /// 配置里是否启用了剪贴板历史。
    pub enabled: bool,
    pub count: usize,
    /// 每次历史变化自增；前端据此重查列表（事件本身不带条目）。
    pub revision: u64,
    pub retention_days: u32,
    pub capture_images: bool,
    pub paste_on_click: bool,
    /// 已占用的存储（文本字节 + 图片文件字节）。
    pub storage_bytes: u64,
}

/// 查询时的类型筛选。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKindFilter {
    All,
    Text,
    Image,
}

impl ClipKindFilter {
    /// 解析前端传来的筛选值；无法识别一律当作「全部」。
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim) {
            Some("text") => Self::Text,
            Some("image") => Self::Image,
            _ => Self::All,
        }
    }

    fn accepts(self, kind: ClipKind) -> bool {
        match self {
            Self::All => true,
            Self::Text => kind == ClipKind::Text,
            Self::Image => kind == ClipKind::Image,
        }
    }
}

/// 采集到的一份内容。
enum Captured {
    Text(String),
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

/// 历史在内存中的权威副本（最新在前）。
struct Inner {
    enabled: bool,
    entries: Vec<ClipEntry>,
    next_id: u64,
    retention_days: u32,
    capture_images: bool,
    paste_on_click: bool,
    revision: u64,
}

/// 由 Tauri 托管的剪贴板历史状态。
pub struct ClipboardState {
    inner: RwLock<Inner>,
    /// 采集线程是否应当继续运行；禁用配置即置 false 让线程自行退出。
    running: AtomicBool,
    /// 自己写入的剪贴板序列号（`0` 表示无）。
    self_write_seq: AtomicU32,
}

impl ClipboardState {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner {
                enabled: false,
                entries: Vec::new(),
                next_id: 1,
                retention_days: AppConfig::default().clipboard_retention_days,
                capture_images: true,
                paste_on_click: true,
                revision: 0,
            }),
            running: AtomicBool::new(false),
            self_write_seq: AtomicU32::new(0),
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.write().enabled = enabled;
    }

    /// 配置同步：保留天数 / 是否采集图片 / 是否点击即粘贴。
    /// 返回是否因保留天数收紧而删掉了条目（调用方据此落盘）。
    pub fn configure(&self, app: &AppHandle, config: &AppConfig) -> bool {
        let mut inner = self.write();
        let retention_changed = inner.retention_days != config.clipboard_retention_days;
        inner.retention_days = config.clipboard_retention_days;
        inner.capture_images = config.clipboard_capture_images;
        inner.paste_on_click = config.clipboard_paste_on_click;

        let removed = if retention_changed {
            prune_locked(&mut inner, now_secs())
        } else {
            Vec::new()
        };
        if !removed.is_empty() {
            inner.revision += 1;
        }
        drop(inner);

        delete_image_files(app, &removed);
        !removed.is_empty()
    }

    pub fn snapshot(&self) -> ClipState {
        let inner = self.read();
        ClipState {
            enabled: inner.enabled,
            count: inner.entries.len(),
            revision: inner.revision,
            retention_days: inner.retention_days,
            capture_images: inner.capture_images,
            paste_on_click: inner.paste_on_click,
            storage_bytes: inner.entries.iter().map(|entry| entry.size).sum(),
        }
    }

    /// 主动广播一次当前状态（配置开关等由外部触发的时机）。
    pub fn broadcast(&self, app: &AppHandle) {
        if let Err(err) = app.emit(CLIPBOARD_EVENT, self.snapshot()) {
            log::warn!("广播剪贴板历史状态失败：{err}");
        }
    }

    /// 记录「这份内容是应用自己写的」：采集线程读到该序列号时跳过。
    pub fn note_self_write(&self, seq: Option<u32>) {
        if let Some(seq) = seq.filter(|seq| *seq != 0) {
            self.self_write_seq.store(seq, Ordering::SeqCst);
        }
    }

    /// 消费一次「自写序列号」（命中即清空，避免长期残留）。
    fn take_self_write(&self, seq: Option<u32>) -> bool {
        let Some(seq) = seq.filter(|seq| *seq != 0) else {
            return false;
        };
        if self.self_write_seq.load(Ordering::SeqCst) != seq {
            return false;
        }
        self.self_write_seq.store(0, Ordering::SeqCst);
        true
    }

    /// 采集一次内容并入库；返回历史是否真的变化。
    fn accept(&self, app: &AppHandle, captured: Captured) -> bool {
        match captured {
            Captured::Text(text) => {
                if text.len() > MAX_TEXT_BYTES {
                    log::warn!("剪贴板文本过大（{} 字节），已跳过", text.len());
                    return false;
                }
                let digest = digest_text(&text);
                let entry = ClipEntry {
                    id: 0,
                    kind: ClipKind::Text,
                    text: Some(text.clone()),
                    image: None,
                    digest,
                    created_at: now_secs(),
                    size: text.len() as u64,
                };
                self.install(app, entry)
            }
            Captured::Image {
                width,
                height,
                rgba,
            } => {
                if !self.read().capture_images {
                    return false;
                }
                let digest = digest_image(width, height, &rgba);
                if self.bump_existing(app, &digest) {
                    return true;
                }

                let id = self.alloc_id();
                let image = match write_image_files(app, id, width, height, rgba) {
                    Ok(image) => image,
                    Err(err) => {
                        log::warn!("保存剪贴板图片失败：{err}");
                        return false;
                    }
                };
                let entry = ClipEntry {
                    id,
                    kind: ClipKind::Image,
                    text: None,
                    size: image.bytes,
                    image: Some(image),
                    digest,
                    created_at: now_secs(),
                };
                self.install(app, entry)
            }
        }
    }

    fn alloc_id(&self) -> u64 {
        let mut inner = self.write();
        let id = inner.next_id;
        inner.next_id += 1;
        id
    }

    /// 同一内容已存在时把它提到最前（复用原 id，不新增一条）。
    fn bump_existing(&self, app: &AppHandle, digest: &str) -> bool {
        {
            let mut inner = self.write();
            let Some(pos) = inner
                .entries
                .iter()
                .position(|entry| entry.digest == digest)
            else {
                return false;
            };
            let mut entry = inner.entries.remove(pos);
            entry.created_at = now_secs();
            inner.entries.insert(0, entry);
            inner.revision += 1;
        }
        self.persist(app);
        self.broadcast(app);
        true
    }

    /// 入库：同摘要则「提到最前」并复用原 id；否则插到最前（`id == 0` 时自动分配）。
    fn install(&self, app: &AppHandle, mut entry: ClipEntry) -> bool {
        let removed = {
            let mut inner = self.write();
            if let Some(pos) = inner
                .entries
                .iter()
                .position(|old| old.digest == entry.digest)
            {
                let old = inner.entries.remove(pos);
                entry.id = old.id;
                if entry.image.is_none() {
                    entry.image = old.image;
                }
                if entry.size == 0 {
                    entry.size = old.size;
                }
            } else if entry.id == 0 {
                entry.id = inner.next_id;
                inner.next_id += 1;
            } else {
                inner.next_id = inner.next_id.max(entry.id + 1);
            }

            inner.entries.insert(0, entry);
            let removed = prune_locked(&mut inner, now_secs());
            inner.revision += 1;
            removed
        };

        delete_image_files(app, &removed);
        self.persist(app);
        self.broadcast(app);
        true
    }

    /// 查询历史元数据。空关键词返回「最近入库」的条目。
    pub fn query(
        &self,
        raw_query: &str,
        kind: ClipKindFilter,
        limit: Option<usize>,
    ) -> Vec<ClipEntryMeta> {
        let needle = raw_query.trim().to_lowercase();
        let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

        self.read()
            .entries
            .iter()
            .filter(|entry| kind.accepts(entry.kind))
            .filter(|entry| {
                needle.is_empty()
                    || entry
                        .text
                        .as_deref()
                        .is_some_and(|text| text.to_lowercase().contains(&needle))
            })
            .take(limit)
            .map(ClipEntryMeta::from)
            .collect()
    }

    /// 删除一条历史（图片条目连文件一起删）。
    pub fn delete(&self, app: &AppHandle, id: u64) -> AppResult<()> {
        let removed = {
            let mut inner = self.write();
            let Some(pos) = inner.entries.iter().position(|entry| entry.id == id) else {
                return Ok(());
            };
            let entry = inner.entries.remove(pos);
            inner.revision += 1;
            image_files_of(&entry)
        };
        delete_image_files(app, &removed);
        self.persist(app);
        self.broadcast(app);
        Ok(())
    }

    /// 清空全部历史（连同图片文件）。
    pub fn clear(&self, app: &AppHandle) -> AppResult<()> {
        let names = {
            let mut inner = self.write();
            if inner.entries.is_empty() {
                return Ok(());
            }
            let names = inner
                .entries
                .iter()
                .flat_map(image_files_of)
                .collect::<Vec<_>>();
            inner.entries.clear();
            inner.revision += 1;
            names
        };
        delete_image_files(app, &names);
        self.persist(app);
        self.broadcast(app);
        Ok(())
    }

    /// 按保留策略清理一次过期条目。
    pub fn prune(&self, app: &AppHandle) -> AppResult<()> {
        let removed = {
            let mut inner = self.write();
            let removed = prune_locked(&mut inner, now_secs());
            if !removed.is_empty() {
                inner.revision += 1;
            }
            removed
        };
        if removed.is_empty() {
            return Ok(());
        }
        delete_image_files(app, &removed);
        self.persist(app);
        self.broadcast(app);
        Ok(())
    }

    /// 读取一条文本记录的完整正文（编辑 / 预览时按需拉取）。
    pub fn text_of(&self, id: u64) -> AppResult<String> {
        self.read()
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .and_then(|entry| entry.text.clone())
            .ok_or_else(|| AppError::Other("文本记录不存在".into()))
    }

    /// 修改一条文本记录的正文。
    pub fn update_text(&self, app: &AppHandle, id: u64, text: &str) -> AppResult<()> {
        {
            let mut inner = self.write();
            let Some(entry) = inner.entries.iter_mut().find(|entry| entry.id == id) else {
                return Err(AppError::Other("记录不存在".into()));
            };
            if entry.kind != ClipKind::Text {
                return Err(AppError::Other("只能编辑文本记录".into()));
            }
            entry.text = Some(text.to_string());
            entry.digest = digest_text(text);
            entry.size = text.len() as u64;
            inner.revision += 1;
        }
        self.persist(app);
        self.broadcast(app);
        Ok(())
    }

    /// 取一条记录的内容（用于写剪贴板）。
    pub fn content(&self, app: &AppHandle, id: u64) -> AppResult<crate::paste::PasteContent> {
        let entry = self
            .read()
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| AppError::Other("记录不存在".into()))?;

        match entry.kind {
            ClipKind::Text => Ok(crate::paste::PasteContent::Text(
                entry.text.unwrap_or_default(),
            )),
            ClipKind::Image => {
                let meta = entry
                    .image
                    .ok_or_else(|| AppError::Other("图片记录缺少文件".into()))?;
                let (width, height, rgba) = load_image_rgba(app, &meta)?;
                Ok(crate::paste::PasteContent::Image {
                    width,
                    height,
                    rgba,
                })
            }
        }
    }

    /// 图片记录的 data URL（`thumb` 为真时给列表用的缩略图）。
    pub fn image_data_url(&self, app: &AppHandle, id: u64, thumb: bool) -> AppResult<String> {
        let meta = self
            .read()
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .and_then(|entry| entry.image.clone())
            .ok_or_else(|| AppError::Other("图片记录不存在".into()))?;

        let name = if thumb { meta.thumb } else { meta.file };
        let bytes = std::fs::read(image_dir(app)?.join(name))?;
        Ok(format!("data:image/png;base64,{}", base64_encode(&bytes)))
    }

    /// 把历史（元数据）写回磁盘。
    fn persist(&self, app: &AppHandle) {
        let index = {
            let inner = self.read();
            PersistedIndex {
                version: INDEX_VERSION,
                next_id: inner.next_id,
                entries: inner.entries.clone(),
            }
        };
        if let Err(err) = write_index(app, &index) {
            log::warn!("写入剪贴板历史失败：{err}");
        }
    }
}

impl Default for ClipboardState {
    fn default() -> Self {
        Self::new()
    }
}

/// 落盘的历史文件结构。
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedIndex {
    version: u32,
    next_id: u64,
    #[serde(default)]
    entries: Vec<ClipEntry>,
}

/// 采集线程的运行标志：进程级单例（同一时刻只允许一个采集线程）。
static MONITOR_RUNNING: AtomicBool = AtomicBool::new(false);

/// 起采集线程（幂等）。配置启用时才调用。
fn spawn_monitor(app: &AppHandle) {
    if MONITOR_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<ClipboardState>();
        // 首次读到的序列号只当基线：**不**把启用之前剪贴板里已有的内容补录进历史
        // （可能是密码 / 敏感文本），只记录「启用之后新发生的复制」。
        let mut last_seq = capture::sequence_number();
        let mut fallback_digest: Option<String> = None;

        while state.running.load(Ordering::SeqCst) {
            std::thread::sleep(POLL_INTERVAL);
            if !state.running.load(Ordering::SeqCst) {
                break;
            }

            let seq = capture::sequence_number();
            let changed = match (seq, last_seq) {
                (Some(current), Some(previous)) => current != previous,
                // 拿不到序列号（非 Windows）时每次都读内容，靠摘要去重兜底。
                _ => true,
            };
            last_seq = seq;
            if !changed {
                continue;
            }
            if state.take_self_write(seq) {
                continue;
            }

            let allow_images = state.read().capture_images;
            let Some(captured) = capture::read(allow_images) else {
                continue;
            };
            if seq.is_none() {
                let digest = captured.digest();
                if fallback_digest.as_deref() == Some(digest.as_str()) {
                    continue;
                }
                fallback_digest = Some(digest);
            }
            state.accept(&app, captured);
        }

        MONITOR_RUNNING.store(false, Ordering::SeqCst);
        // 退出前再确认一次：刚刚置 false 的瞬间可能又被启用（此时本线程已是「老线程」，
        // 得由新线程接手），否则会出现「开关拨回开、采集却停了」。
        if state.running.load(Ordering::SeqCst) {
            spawn_monitor(&app);
        }
        log::info!("剪贴板历史采集线程已退出");
    });
}

/// 停采集线程（幂等）：置 false 后线程在下一个轮询周期自行退出。
fn stop_monitor(state: &ClipboardState) {
    state.running.store(false, Ordering::SeqCst);
    state.self_write_seq.store(0, Ordering::SeqCst);
}

/// 依配置同步剪贴板历史的存亡（与 `sync_file_index` / `sync_quick_panel_windows` 同一位置）。
pub fn sync(app: &AppHandle, config: &AppConfig) {
    let state = app.state::<ClipboardState>();
    state.set_enabled(config.clipboard_history);

    if config.clipboard_history {
        state.running.store(true, Ordering::SeqCst);
        spawn_monitor(app);
        crate::paste::start_tracker(app);
    } else {
        stop_monitor(&state);
        crate::paste::stop_tracker();
    }

    // 保留天数收紧会立刻裁掉过期条目（`configure` 返回是否有条目被删）。
    if state.configure(app, config) {
        state.persist(app);
    }
    state.broadcast(app);
}

/// 读取落盘的历史（启动时调用，与启用与否无关：重新启用能立刻看到旧历史）。
pub fn load_from_disk(app: &AppHandle, state: &ClipboardState) -> bool {
    let Ok(path) = index_path(app) else {
        return false;
    };
    if !path.is_file() {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(index) = serde_json::from_str::<PersistedIndex>(&content) else {
        log::warn!("读取剪贴板历史失败（将忽略）：JSON 解析失败");
        return false;
    };
    if index.version != INDEX_VERSION {
        log::warn!("剪贴板历史版本不符（{}），已忽略", index.version);
        return false;
    }

    let dir = image_dir(app).ok();
    let removed = {
        let mut inner = state.write();
        inner.entries = index
            .entries
            .into_iter()
            .filter(|entry| match &entry.image {
                // 图片文件已被外部删掉：这条记录不再可用。
                Some(image) => dir
                    .as_ref()
                    .is_some_and(|dir| dir.join(&image.file).is_file()),
                None => entry.kind == ClipKind::Text && entry.text.is_some(),
            })
            .collect();
        inner.next_id = inner.next_id.max(index.next_id).max(
            inner
                .entries
                .iter()
                .map(|entry| entry.id + 1)
                .max()
                .unwrap_or(1),
        );
        prune_locked(&mut inner, now_secs())
    };

    let count = state.read().entries.len();
    if !removed.is_empty() {
        delete_image_files(app, &removed);
    }
    log::info!("已载入剪贴板历史：{count} 条");
    true
}

impl Captured {
    /// 内容摘要（非 Windows 下没有序列号，用它去重）。
    fn digest(&self) -> String {
        match self {
            Captured::Text(text) => digest_text(text),
            Captured::Image {
                width,
                height,
                rgba,
            } => digest_image(*width, *height, rgba),
        }
    }
}

impl From<&ClipEntry> for ClipEntryMeta {
    fn from(entry: &ClipEntry) -> Self {
        let (preview, text_length, image_width, image_height) = match entry.kind {
            ClipKind::Text => {
                let text = entry.text.as_deref().unwrap_or_default();
                (text_preview(text), text.chars().count(), None, None)
            }
            ClipKind::Image => match &entry.image {
                Some(image) => (
                    format!("图片 {} × {}", image.width, image.height),
                    0,
                    Some(image.width),
                    Some(image.height),
                ),
                None => (String::from("图片"), 0, None, None),
            },
        };

        Self {
            id: entry.id,
            kind: entry.kind,
            preview,
            text_length,
            image_width,
            image_height,
            size: entry.size,
            created_at: entry.created_at,
        }
    }
}

/// 按保留天数裁掉过期条目，返回被删掉的图片文件名。
///
/// `retention_days == 0` 表示永久保留（此时不删任何条目）。
fn prune_locked(inner: &mut Inner, now: u64) -> Vec<String> {
    if inner.retention_days == 0 {
        return Vec::new();
    }
    let cutoff = now.saturating_sub(inner.retention_days as u64 * 86_400);

    let mut removed = Vec::new();
    inner.entries.retain(|entry| {
        if entry.created_at >= cutoff {
            return true;
        }
        removed.extend(image_files_of(entry));
        false
    });
    removed
}

/// 一条记录对应的图片文件（文本条目为空）。
fn image_files_of(entry: &ClipEntry) -> Vec<String> {
    match &entry.image {
        Some(image) => vec![image.file.clone(), image.thumb.clone()],
        None => Vec::new(),
    }
}

fn text_preview(text: &str) -> String {
    let mut preview = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= PREVIEW_CHARS {
            preview.push('…');
            break;
        }
        preview.push(ch);
    }
    preview
}

/// FNV-1a（64 位）：只需要「同内容同摘要」，不要求密码学强度。
fn fnv1a_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn digest_text(text: &str) -> String {
    format!(
        "t{:016x}",
        fnv1a_update(0xcbf2_9ce4_8422_2325, text.as_bytes())
    )
}

fn digest_image(width: u32, height: u32, rgba: &[u8]) -> String {
    let mut hash = fnv1a_update(0xcbf2_9ce4_8422_2325, &width.to_le_bytes());
    hash = fnv1a_update(hash, &height.to_le_bytes());
    hash = fnv1a_update(hash, rgba);
    format!("i{hash:016x}")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn index_path(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app.path().app_config_dir()?.join(INDEX_FILE_NAME))
}

/// 图片目录（不存在时由写入方创建）。
pub fn image_dir(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app.path().app_config_dir()?.join(IMAGE_DIR_NAME))
}

fn write_index(app: &AppHandle, index: &PersistedIndex) -> AppResult<()> {
    let path = index_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(index)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn write_image_files(
    app: &AppHandle,
    id: u64,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
) -> AppResult<ImageMeta> {
    let dir = image_dir(app)?;
    std::fs::create_dir_all(&dir)?;

    let image = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| AppError::Other("剪贴板图片尺寸与像素数据不匹配".into()))?;

    let file = format!("{id}.png");
    let thumb = format!("{id}.thumb.png");
    image.save_with_format(dir.join(&file), image::ImageFormat::Png)?;
    image::DynamicImage::ImageRgba8(image)
        .thumbnail(THUMB_MAX, THUMB_MAX)
        .save_with_format(dir.join(&thumb), image::ImageFormat::Png)?;

    let bytes = std::fs::metadata(dir.join(&file))
        .map(|metadata| metadata.len())
        .unwrap_or_default();

    Ok(ImageMeta {
        width,
        height,
        file,
        thumb,
        bytes,
    })
}

/// 读回一条图片记录的原图像素（粘贴时用）。
fn load_image_rgba(app: &AppHandle, meta: &ImageMeta) -> AppResult<(u32, u32, Vec<u8>)> {
    let path = image_dir(app)?.join(&meta.file);
    let image = image::open(&path)?.to_rgba8();
    Ok((image.width(), image.height(), image.into_raw()))
}

fn delete_image_files(app: &AppHandle, names: &[String]) {
    if names.is_empty() {
        return;
    }
    let Ok(dir) = image_dir(app) else {
        return;
    };
    for name in names {
        let path = dir.join(name);
        if let Err(err) = std::fs::remove_file(&path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::warn!("删除剪贴板图片失败 path={} err={err}", path.display());
            }
        }
    }
}

/// 标准 base64 编码（图片经 data URL 回传前端；只编码不解码，不值得为此引一个 crate）。
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let packed = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[(packed >> 18) as usize & 63] as char);
        out.push(TABLE[(packed >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(packed >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[packed as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(windows)]
mod capture {
    use windows::Win32::System::DataExchange::{
        GetClipboardSequenceNumber, IsClipboardFormatAvailable,
    };

    use super::Captured;

    /// DIB / DIBV5 格式号（`CF_BITMAP` 等其它图片格式交给 arboard 自行处理）。
    const CF_DIB: u32 = 8;
    const CF_DIBV5: u32 = 17;

    /// 剪贴板序列号：内容变化即自增。读不到时返回 `None`。
    pub fn sequence_number() -> Option<u32> {
        // SAFETY: 只读查询，不涉及任何句柄所有权。
        let seq = unsafe { GetClipboardSequenceNumber() };
        (seq != 0).then_some(seq)
    }

    fn has_image() -> bool {
        // SAFETY: 只读查询剪贴板格式是否可用。
        unsafe {
            IsClipboardFormatAvailable(CF_DIB).is_ok()
                || IsClipboardFormatAvailable(CF_DIBV5).is_ok()
        }
    }

    /// 读一次剪贴板内容：优先图片（截图 / 复制图片常常同时带文本），否则文本。
    pub fn read(allow_images: bool) -> Option<Captured> {
        let mut clipboard = arboard::Clipboard::new().ok()?;

        if allow_images && has_image() {
            if let Ok(image) = clipboard.get_image() {
                return Some(Captured::Image {
                    width: image.width as u32,
                    height: image.height as u32,
                    rgba: image.bytes.into_owned(),
                });
            }
        }

        match clipboard.get_text() {
            Ok(text) if !text.trim().is_empty() => Some(Captured::Text(text)),
            _ => None,
        }
    }
}

#[cfg(not(windows))]
mod capture {
    use super::Captured;

    /// 非 Windows 没有廉价的「剪贴板序列号」：每次都读内容，靠摘要去重兜底。
    pub fn sequence_number() -> Option<u32> {
        None
    }

    pub fn read(allow_images: bool) -> Option<Captured> {
        let mut clipboard = arboard::Clipboard::new().ok()?;
        if allow_images {
            if let Ok(image) = clipboard.get_image() {
                return Some(Captured::Image {
                    width: image.width as u32,
                    height: image.height as u32,
                    rgba: image.bytes.into_owned(),
                });
            }
        }
        match clipboard.get_text() {
            Ok(text) if !text.trim().is_empty() => Some(Captured::Text(text)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_entry(id: u64, text: &str, created_at: u64) -> ClipEntry {
        ClipEntry {
            id,
            kind: ClipKind::Text,
            text: Some(text.to_string()),
            image: None,
            digest: digest_text(text),
            created_at,
            size: text.len() as u64,
        }
    }

    fn inner_with(entries: Vec<ClipEntry>, retention_days: u32) -> Inner {
        Inner {
            enabled: true,
            entries,
            next_id: 100,
            retention_days,
            capture_images: true,
            paste_on_click: true,
            revision: 0,
        }
    }

    #[test]
    fn digest_is_stable_and_content_addressed() {
        assert_eq!(digest_text("hello"), digest_text("hello"));
        assert_ne!(digest_text("hello"), digest_text("hello "));
        // 同尺寸但像素不同必须是不同摘要（否则截图会被误当成同一条）。
        assert_ne!(
            digest_image(2, 1, &[0, 0, 0, 0, 1, 1, 1, 1]),
            digest_image(2, 1, &[0, 0, 0, 0, 2, 2, 2, 2])
        );
        // 尺寸参与摘要：同样的像素、不同尺寸不应相同。
        assert_ne!(
            digest_image(1, 2, &[1, 2, 3, 4, 5, 6, 7, 8]),
            digest_image(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8])
        );
    }

    #[test]
    fn prune_drops_entries_older_than_retention_and_keeps_the_rest() {
        let now = 1_000_000_000u64;
        let day = 86_400u64;
        let mut inner = inner_with(
            vec![
                text_entry(1, "今天", now - 10),
                text_entry(2, "昨天", now - day - 10),
                text_entry(3, "上个月", now - 40 * day),
            ],
            30,
        );

        assert!(
            prune_locked(&mut inner, now).is_empty(),
            "文本条目没有图片文件"
        );
        let ids: Vec<u64> = inner.entries.iter().map(|entry| entry.id).collect();
        assert_eq!(ids, vec![1, 2], "只保留 30 天内的条目");
    }

    #[test]
    fn retention_zero_keeps_everything() {
        let now = 2_000_000_000u64;
        let mut inner = inner_with(vec![text_entry(1, "很久以前", 1)], 0);
        prune_locked(&mut inner, now);
        assert_eq!(inner.entries.len(), 1, "0 = 永久保留，不裁任何条目");
    }

    #[test]
    fn prune_reports_image_files_so_they_can_be_deleted() {
        let now = 1_000_000_000u64;
        let mut entry = text_entry(7, "旧图", now - 40 * 86_400);
        entry.kind = ClipKind::Image;
        entry.text = None;
        entry.image = Some(ImageMeta {
            width: 4,
            height: 4,
            file: "7.png".into(),
            thumb: "7.thumb.png".into(),
            bytes: 128,
        });

        let mut inner = inner_with(vec![entry], 30);
        let removed = prune_locked(&mut inner, now);
        assert_eq!(
            removed,
            vec!["7.png".to_string(), "7.thumb.png".to_string()]
        );
        assert!(inner.entries.is_empty());
    }

    #[test]
    fn query_filters_by_kind_and_substring_case_insensitively() {
        let mut image = text_entry(1, "占位", 10);
        image.kind = ClipKind::Image;
        image.text = None;
        image.image = Some(ImageMeta {
            width: 100,
            height: 50,
            file: "1.png".into(),
            thumb: "1.thumb.png".into(),
            bytes: 10,
        });

        let state = ClipboardState::new();
        *state.write() = inner_with(
            vec![
                image,
                text_entry(2, "Hello World", 20),
                text_entry(3, "world 之外", 30),
            ],
            30,
        );

        let all: Vec<u64> = state
            .query("", ClipKindFilter::All, None)
            .iter()
            .map(|meta| meta.id)
            .collect();
        assert_eq!(all, vec![1, 2, 3]);

        let matched: Vec<u64> = state
            .query("WORLD", ClipKindFilter::All, None)
            .iter()
            .map(|meta| meta.id)
            .collect();
        assert_eq!(
            matched,
            vec![2, 3],
            "大小写不敏感的子串匹配，图片无正文不参与"
        );

        let images: Vec<u64> = state
            .query("", ClipKindFilter::Image, None)
            .iter()
            .map(|meta| meta.id)
            .collect();
        assert_eq!(images, vec![1]);
    }

    #[test]
    fn limit_is_clamped_to_the_hard_cap() {
        let entries = (1..=10)
            .map(|id| text_entry(id, &format!("text {id}"), id))
            .collect();
        let state = ClipboardState::new();
        *state.write() = inner_with(entries, 30);

        assert_eq!(state.query("", ClipKindFilter::All, Some(3)).len(), 3);
        assert_eq!(
            state.query("", ClipKindFilter::All, Some(0)).len(),
            1,
            "0 会夹到 1"
        );
        assert_eq!(
            state.query("", ClipKindFilter::All, Some(9999)).len(),
            10,
            "超出硬上限只会夹到上限，不会凭空补条目"
        );
    }

    #[test]
    fn preview_marks_truncation_and_counts_characters_not_bytes() {
        let long = "字".repeat(PREVIEW_CHARS + 5);
        let preview = text_preview(&long);
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), PREVIEW_CHARS + 1);

        let short = "短文本";
        assert_eq!(text_preview(short), short);
    }

    #[test]
    fn image_meta_preview_reports_dimensions() {
        let mut entry = text_entry(5, "x", 1);
        entry.kind = ClipKind::Image;
        entry.text = None;
        entry.image = Some(ImageMeta {
            width: 1920,
            height: 1080,
            file: "5.png".into(),
            thumb: "5.thumb.png".into(),
            bytes: 1024,
        });

        let meta = ClipEntryMeta::from(&entry);
        assert_eq!(meta.preview, "图片 1920 × 1080");
        assert_eq!(meta.text_length, 0);
        assert_eq!(meta.image_width, Some(1920));
    }

    #[test]
    fn base64_encoder_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // 含高位字节：必须按无符号处理，不能出现符号扩展。
        assert_eq!(base64_encode(&[0xFF, 0xFF, 0xFF]), "////");
    }

    #[test]
    fn persisted_index_round_trips_and_tolerates_missing_entries() {
        let index = PersistedIndex {
            version: INDEX_VERSION,
            next_id: 42,
            entries: vec![text_entry(1, "hi", 5)],
        };
        let json = serde_json::to_string(&index).expect("应当可序列化");
        let back: PersistedIndex = serde_json::from_str(&json).expect("应当可读");
        assert_eq!(back.version, INDEX_VERSION);
        assert_eq!(back.next_id, 42);
        assert_eq!(back.entries.len(), 1);
        assert_eq!(back.entries[0].text.as_deref(), Some("hi"));

        // 老版本文件缺 `entries` 时不该让整个文件报废。
        let sparse: PersistedIndex =
            serde_json::from_str(r#"{"version":1,"nextId":3}"#).expect("缺 entries 应当可读");
        assert!(sparse.entries.is_empty());
    }

    #[test]
    fn clip_kind_filter_parses_unknown_values_as_all() {
        assert_eq!(ClipKindFilter::parse(Some("text")), ClipKindFilter::Text);
        assert_eq!(ClipKindFilter::parse(Some("image")), ClipKindFilter::Image);
        assert_eq!(ClipKindFilter::parse(Some("image ")), ClipKindFilter::Image);
        assert_eq!(ClipKindFilter::parse(Some("nope")), ClipKindFilter::All);
        assert_eq!(ClipKindFilter::parse(None), ClipKindFilter::All);
    }
}

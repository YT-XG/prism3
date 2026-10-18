//! 本地文件搜索的**文件名索引**：扫描 → 落盘 → 内存查询 → 状态广播。
//!
//! 设计要点：
//! - **零外部依赖**：只用 `std::fs` 遍历 + 自制二进制索引文件，不依赖 Everything /
//!   Windows Search 索引，也不需要任何额外 crate。
//! - **按需采集**：只有配置启用时才建索引；禁用即取消在途扫描（沿用 `MonitorState`
//!   的「按需采集」模型），扫完落盘，下次启动先读盘（秒开）再后台刷新。
//! - **查询是纯内存线性扫描**：条目数在十万量级时耗时为毫秒级，因此不需要数据库 / 倒排表。
//! - 索引只在**独立线程**中构建，命令层负责起线程与广播，本模块不阻塞 UI。

use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{AppError, AppResult};

/// 索引状态变化（扫描进度 / 就绪 / 取消）的广播事件名。
///
/// 广播给所有窗口：主窗口搜索页与快捷小窗搜索页都要显示同一份状态。
pub const SEARCH_INDEX_EVENT: &str = "search:index-state";

/// 落盘索引文件名（版本写在魔数里，格式变更时改魔数即可安全重建）。
const INDEX_FILE_NAME: &str = "file-index-v1.bin";
/// 索引文件魔数：`P3IDX` + 版本号 `1`。
const INDEX_MAGIC: &[u8; 6] = b"P3IDX1";
/// 索引文件损坏时的上限保护，避免坏文件导致巨额分配。
const MAX_LOAD_ROOTS: u32 = 4_096;
const MAX_LOAD_ENTRIES: u32 = 4_000_000;

/// 索引条目上限：达到后停止收集并置 `truncated`（约 150MB 内存的兜底）。
pub const MAX_ENTRIES: usize = 600_000;
/// 遍历深度上限：与「跳过重解析点」一起杜绝联接点成环。
const MAX_DEPTH: u32 = 32;

/// 扫描时跳过的目录名（按最后一段名字匹配，不分大小写）。
///
/// 这些都是「体量大但几乎不需要搜」的目录：系统目录、包管理器缓存、构建产物。
pub const SKIP_DIRS: &[&str] = &[
    "$RECYCLE.BIN",
    "System Volume Information",
    "AppData",
    "Windows",
    "Program Files",
    "Program Files (x86)",
    "ProgramData",
    "node_modules",
    "target",
    ".git",
    ".cache",
    ".cargo",
    ".rustup",
];

/// 默认返回条数与硬上限。
const DEFAULT_LIMIT: u32 = 40;
const MAX_LIMIT: u32 = 60;

/// 扫描进度广播节流：时间与条数谁先到就发一次，避免事件风暴。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(300);
const PROGRESS_EVERY_ENTRIES: u64 = 5_000;

/// 索引所处的阶段（前端据此渲染空态 / 进度条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexPhase {
    /// 未启用。
    Idle,
    /// 正在读取落盘索引。
    Loading,
    /// 正在扫描目录（已有索引时仍可查询，结果标记为 `stale`）。
    Scanning,
    /// 就绪。
    Ready,
    /// 构建失败（`error` 带原因）。
    Error,
}

/// 索引状态快照：命令返回值与事件载荷共用同一个结构。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexState {
    /// 配置里是否启用了文件搜索。
    pub enabled: bool,
    pub phase: IndexPhase,
    /// 本轮扫描已遍历的条目数（含被跳过目录之外的子目录）。
    pub scanned: u64,
    /// 索引中的条目数。
    pub indexed: u64,
    /// 索引构建完成的时间戳（秒）；从未建成时为 `None`。
    pub built_at_secs: Option<u64>,
    /// 是否有一次刷新正在进行（此时结果可能略旧）。
    pub stale: bool,
    /// 是否因达到 `MAX_ENTRIES` 提前停止。
    pub truncated: bool,
    pub error: Option<String>,
    /// 本轮生效的索引根（绝对路径）。
    pub roots: Vec<String>,
    /// 配置里存在但当前已不存在的根。
    pub missing_roots: Vec<String>,
}

/// 一条命中结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub path: String,
    pub name: String,
    /// 所在目录（前端中间省略显示）。
    pub parent: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_secs: i64,
}

/// 内存中的索引条目。
///
/// `name_lower` / `parent_lower` 在构建时算好：查询是热路径，逐条 `to_lowercase`
/// 会让每次按键都产生十几万次分配。
#[derive(Debug, Clone)]
struct Entry {
    path: PathBuf,
    name_lower: String,
    parent_lower: String,
    size: u64,
    modified_secs: i64,
    is_dir: bool,
    /// 路径分量数，用于「浅层优先」的排序（构建时算一次）。
    depth: u16,
}

impl Entry {
    fn from_path(path: PathBuf, metadata: &std::fs::Metadata, is_dir: bool) -> Self {
        let modified_secs = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_secs() as i64)
            .unwrap_or(0);
        Self::from_loaded(path, metadata.len(), modified_secs, is_dir)
    }

    /// 由「路径 + 基础字段」构造（落盘索引读取时用，名称与父目录在此时派生）。
    fn from_loaded(path: PathBuf, size: u64, modified_secs: i64, is_dir: bool) -> Self {
        let name_lower = path
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let parent_lower = path
            .parent()
            .map(|parent| parent.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let depth = path.components().count().min(u16::MAX as usize) as u16;
        Self {
            path,
            name_lower,
            parent_lower,
            size,
            modified_secs,
            is_dir,
            depth,
        }
    }
}

/// 命中档位：数字越小越优先。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    /// 文件名完全相等。
    Exact = 0,
    /// 文件名以关键词开头。
    Prefix = 1,
    /// 文件名在词边界处以关键词开头（`report` 命中 `my-report`）。
    Boundary = 2,
    /// 文件名包含关键词。
    Contains = 3,
    /// 所在目录包含关键词（用户可以只记得目录名）。
    Path = 4,
    /// 模糊子序列（`rpt` 命中 `report`），只在前面几档凑不够结果时才跑。
    Fuzzy = 5,
}

/// 排序用的候选（`Ord` 即优先级：越小越靠前）。
///
/// `index` 是扫描序号，保证排序稳定确定；`rank` 派生自「档位 → 名字长度 → 路径深度 → 越新越前」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Ranked {
    tier: u8,
    name_len: u32,
    depth: u16,
    neg_mtime: i64,
    index: usize,
}

struct Inner {
    enabled: bool,
    phase: IndexPhase,
    entries: Vec<Entry>,
    scanned: u64,
    built_at_secs: Option<u64>,
    error: Option<String>,
    truncated: bool,
    roots: Vec<PathBuf>,
    missing_roots: Vec<PathBuf>,
}

/// 由 Tauri 托管的文件索引状态。
pub struct FileIndexState {
    inner: RwLock<Inner>,
    /// 每次构建自增；在途扫描发现自己的代号过期即放弃结果。
    generation: AtomicU64,
    cancel: AtomicBool,
}

impl FileIndexState {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner {
                enabled: false,
                phase: IndexPhase::Idle,
                entries: Vec::new(),
                scanned: 0,
                built_at_secs: None,
                error: None,
                truncated: false,
                roots: Vec::new(),
                missing_roots: Vec::new(),
            }),
            generation: AtomicU64::new(0),
            cancel: AtomicBool::new(false),
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

    /// 当前状态快照。
    pub fn snapshot(&self) -> IndexState {
        dto(&self.read())
    }

    /// 主动广播一次当前状态（配置开关等由外部触发的时机）。
    pub fn broadcast(&self, app: &AppHandle) {
        emit_state(app, &self.snapshot());
    }

    pub fn is_enabled(&self) -> bool {
        self.read().enabled
    }

    /// 配置启用状态变更。关闭时立即作废在途扫描（保留已建好的索引，重新开启秒开）。
    pub fn set_enabled(&self, enabled: bool) {
        let mut inner = self.write();
        inner.enabled = enabled;
        if enabled {
            if inner.phase == IndexPhase::Idle && inner.built_at_secs.is_some() {
                inner.phase = IndexPhase::Ready;
            }
            return;
        }

        self.generation.fetch_add(1, Ordering::SeqCst);
        self.cancel.store(true, Ordering::SeqCst);
        inner.phase = IndexPhase::Idle;
        inner.scanned = 0;
        inner.error = None;
    }

    /// 取消在途扫描并让阶段回落到可用状态（幂等）。
    pub fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.cancel.store(true, Ordering::SeqCst);

        let mut inner = self.write();
        if inner.phase == IndexPhase::Scanning || inner.phase == IndexPhase::Loading {
            inner.phase = if inner.built_at_secs.is_some() {
                IndexPhase::Ready
            } else {
                IndexPhase::Idle
            };
            inner.scanned = 0;
        }
    }

    /// 开始一轮扫描：返回本轮代号（旧代号的结果会被丢弃）。
    fn begin_scan(&self, roots: Vec<PathBuf>, missing_roots: Vec<PathBuf>) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.cancel.store(false, Ordering::SeqCst);

        let mut inner = self.write();
        inner.phase = IndexPhase::Scanning;
        inner.scanned = 0;
        inner.error = None;
        inner.roots = roots;
        inner.missing_roots = missing_roots;
        generation
    }

    /// 本轮扫描是否已作废（被取消，或已有更新的一轮开始）。
    fn is_stale(&self, generation: u64) -> bool {
        self.cancel.load(Ordering::SeqCst) || self.generation.load(Ordering::SeqCst) != generation
    }

    /// 记录扫描进度；作废时返回 `None`（调用方据此停止扫描）。
    fn report_progress(&self, generation: u64, scanned: u64, indexed: u64) -> Option<IndexState> {
        if self.is_stale(generation) {
            return None;
        }
        let mut inner = self.write();
        inner.scanned = scanned;
        Some(dto_with_index(&inner, indexed))
    }

    /// 安装构建结果；作废时返回 `None`。
    #[allow(clippy::too_many_arguments)]
    fn install(
        &self,
        generation: u64,
        entries: Vec<Entry>,
        roots: Vec<PathBuf>,
        missing_roots: Vec<PathBuf>,
        built_at_secs: u64,
        truncated: bool,
    ) -> Option<IndexState> {
        if self.is_stale(generation) {
            return None;
        }
        let mut inner = self.write();
        inner.entries = entries;
        inner.roots = roots;
        inner.missing_roots = missing_roots;
        inner.built_at_secs = Some(built_at_secs);
        inner.truncated = truncated;
        inner.scanned = inner.entries.len() as u64;
        inner.error = None;
        inner.phase = if inner.enabled {
            IndexPhase::Ready
        } else {
            IndexPhase::Idle
        };
        Some(dto(&inner))
    }

    /// 载入落盘索引（阶段先置为 `Loading`）。
    fn begin_load(&self) {
        let mut inner = self.write();
        if inner.enabled {
            inner.phase = IndexPhase::Loading;
        }
    }

    fn install_from_disk(
        &self,
        entries: Vec<Entry>,
        roots: Vec<PathBuf>,
        built_at_secs: Option<u64>,
    ) -> IndexState {
        let mut inner = self.write();
        inner.entries = entries;
        inner.roots = roots;
        inner.built_at_secs = built_at_secs;
        inner.scanned = inner.entries.len() as u64;
        inner.truncated = false;
        inner.error = None;
        inner.phase = if inner.enabled {
            IndexPhase::Ready
        } else {
            IndexPhase::Idle
        };
        dto(&inner)
    }

    /// 标记构建失败（已建好的索引保留可用，前端据此提示）。
    pub fn fail(&self, message: String) -> IndexState {
        let mut inner = self.write();
        inner.phase = IndexPhase::Error;
        inner.error = Some(message);
        dto(&inner)
    }

    /// 当前索引根（供「打开结果」的路径校验使用）。
    pub fn roots(&self) -> Vec<PathBuf> {
        self.read().roots.clone()
    }

    /// 查询命中：空关键词返回最近修改的文件。
    pub fn query(&self, raw_query: &str, limit: Option<u32>) -> Vec<SearchHit> {
        let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT) as usize;
        let query = raw_query.trim().to_lowercase();
        let inner = self.read();

        if !inner.enabled || inner.entries.is_empty() {
            return Vec::new();
        }
        if query.is_empty() {
            return recent_hits(&inner.entries, limit);
        }

        // 有界堆：只保留最优的 `limit * 2` 个候选，内存与条目总数无关。
        let capacity = limit * 2;
        let mut heap: BinaryHeap<Ranked> = BinaryHeap::with_capacity(capacity + 1);
        for (index, entry) in inner.entries.iter().enumerate() {
            if let Some(tier) = name_tier(&entry.name_lower, &query) {
                push_candidate(&mut heap, capacity, entry, index, tier);
            }
        }

        // 名字命中的结果不够，再补「目录名命中」与「模糊子序列」，避免空手而归。
        //
        // 每轮都用「更便宜的档位是否已命中」重新判断一次而不是维护已选集合：
        // 命中同一个条目的两条档位会让它在结果里出现两次，而重算 `name_tier` /
        // `contains` 的成本远低于维护集合（这两轮只在结果不足时才跑）。
        if heap.len() < limit {
            for (index, entry) in inner.entries.iter().enumerate() {
                if name_tier(&entry.name_lower, &query).is_some() {
                    continue;
                }
                if entry.parent_lower.contains(&query) {
                    push_candidate(&mut heap, capacity, entry, index, Tier::Path);
                }
            }
        }
        if heap.len() < limit && query.chars().count() >= 2 {
            for (index, entry) in inner.entries.iter().enumerate() {
                if name_tier(&entry.name_lower, &query).is_some()
                    || entry.parent_lower.contains(&query)
                {
                    continue;
                }
                if is_subsequence(&entry.name_lower, &query) {
                    push_candidate(&mut heap, capacity, entry, index, Tier::Fuzzy);
                }
            }
        }

        let mut ranked = heap.into_vec();
        ranked.sort_unstable();
        ranked.truncate(limit);
        ranked
            .into_iter()
            .map(|candidate| hit_of(&inner.entries[candidate.index]))
            .collect()
    }
}

impl Default for FileIndexState {
    fn default() -> Self {
        Self::new()
    }
}

/// 由内部状态派生 DTO；`indexed_override` 用于「条目还在栈上、尚未安装」的进度阶段。
fn dto_with_index(inner: &Inner, indexed: u64) -> IndexState {
    IndexState {
        indexed,
        ..dto(inner)
    }
}

fn dto(inner: &Inner) -> IndexState {
    IndexState {
        enabled: inner.enabled,
        phase: inner.phase,
        scanned: inner.scanned,
        indexed: inner.entries.len() as u64,
        built_at_secs: inner.built_at_secs,
        stale: inner.phase == IndexPhase::Scanning && !inner.entries.is_empty(),
        truncated: inner.truncated,
        error: inner.error.clone(),
        roots: inner
            .roots
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        missing_roots: inner
            .missing_roots
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    }
}

/// 判定文件名命中档位（不含路径与模糊档）。
fn name_tier(name_lower: &str, query: &str) -> Option<Tier> {
    if name_lower == query {
        return Some(Tier::Exact);
    }
    if name_lower.starts_with(query) {
        return Some(Tier::Prefix);
    }
    if !name_lower.contains(query) {
        return None;
    }
    if has_boundary_match(name_lower, query) {
        Some(Tier::Boundary)
    } else {
        Some(Tier::Contains)
    }
}

/// 关键词是否出现在文件名内部的词边界处。
///
/// 词边界只认 ASCII 分隔符（空格 / `-` / `_` / `.` 等）：中文没有词分隔符，
/// 因此中文关键词除「开头命中」外都会落在 `Contains` 档，这是刻意的取舍——
/// 不引入中文分词，也就不会出现「分词错误反而排到前面」的情况。
fn has_boundary_match(name_lower: &str, query: &str) -> bool {
    name_lower.match_indices(query).any(|(offset, _)| {
        name_lower[..offset]
            .chars()
            .next_back()
            .map(is_word_boundary)
            .unwrap_or(false)
    })
}

fn is_word_boundary(character: char) -> bool {
    matches!(
        character,
        ' ' | '-' | '_' | '.' | '(' | '[' | ',' | '+' | '&' | '\''
    )
}

/// `query` 的字符是否按顺序出现在 `name` 中（模糊匹配，允许间隔）。
fn is_subsequence(name_lower: &str, query: &str) -> bool {
    let mut characters = name_lower.chars();
    query
        .chars()
        .all(|needle| characters.any(|character| character == needle))
}

/// 尝试把候选塞进有界堆：满了就与当前最差的一个比较。
fn push_candidate(
    heap: &mut BinaryHeap<Ranked>,
    capacity: usize,
    entry: &Entry,
    index: usize,
    tier: Tier,
) {
    let candidate = Ranked {
        tier: tier as u8,
        name_len: entry.name_lower.len() as u32,
        depth: entry.depth,
        neg_mtime: -entry.modified_secs,
        index,
    };

    if heap.len() < capacity {
        heap.push(candidate);
        return;
    }
    if let Some(worst) = heap.peek() {
        if candidate < *worst {
            heap.pop();
            heap.push(candidate);
        }
    }
}

/// 空关键词时的「最近的文件」。
fn recent_hits(entries: &[Entry], limit: usize) -> Vec<SearchHit> {
    let mut recent: Vec<&Entry> = entries.iter().filter(|entry| !entry.is_dir).collect();
    recent.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.modified_secs));
    recent.truncate(limit);
    recent.into_iter().map(hit_of).collect()
}

fn hit_of(entry: &Entry) -> SearchHit {
    SearchHit {
        path: entry.path.to_string_lossy().into_owned(),
        name: entry
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry.path.to_string_lossy().into_owned()),
        parent: entry
            .path
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .unwrap_or_default(),
        is_dir: entry.is_dir,
        size: entry.size,
        modified_secs: entry.modified_secs,
    }
}

/// 解析索引根：配置为空时用「桌面 / 文档 / 下载 / 图片」四个常用目录。
///
/// 返回 `(可用根, 已不存在的根)`：后者回传给设置界面标灰，而不是静默忽略。
pub fn resolve_roots(app: &AppHandle, configured: &[String]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let candidates: Vec<PathBuf> = if configured.is_empty() {
        let resolver = app.path();
        [
            resolver.desktop_dir(),
            resolver.document_dir(),
            resolver.download_dir(),
            resolver.picture_dir(),
        ]
        .into_iter()
        .filter_map(Result::ok)
        .collect()
    } else {
        configured.iter().map(PathBuf::from).collect()
    };

    let mut roots: Vec<PathBuf> = Vec::new();
    let mut missing: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        if roots.iter().any(|known| known == &candidate) {
            continue;
        }
        if candidate.is_dir() {
            roots.push(candidate);
        } else {
            missing.push(candidate);
        }
    }
    (roots, missing)
}

/// 校验路径位于某个索引根之下。
///
/// 「打开结果」走的是 Rust 侧 opener（前端没有 opener 的路径权限），这里再兜一层：
/// 即使前端被注入，也无法借这个命令打开索引范围之外的任意路径。
pub fn path_within_roots(path: &Path, roots: &[PathBuf]) -> bool {
    let Ok(canonical) = std::fs::canonicalize(path) else {
        return false;
    };
    roots.iter().any(|root| {
        std::fs::canonicalize(root)
            .map(|canonical_root| canonical.starts_with(canonical_root))
            .unwrap_or(false)
    })
}

/// 读取落盘索引并安装（在独立线程中调用）。返回是否成功载入。
pub fn load_from_disk(app: &AppHandle, state: &FileIndexState) -> bool {
    state.begin_load();

    let Ok(path) = index_path(app) else {
        return false;
    };
    if !path.is_file() {
        return false;
    }

    match read_index_file(&path) {
        Ok((roots, entries)) => {
            let built_at_secs = std::fs::metadata(&path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|since| since.as_secs());
            let count = entries.len();
            let next = state.install_from_disk(entries, roots, built_at_secs);
            emit_state(app, &next);
            log::info!("已载入落盘文件索引：{count} 项");
            true
        }
        Err(err) => {
            log::warn!("读取文件索引失败（将重建）：{err}");
            false
        }
    }
}

/// 构建一轮索引：扫描 → 落盘 → 安装。**在独立线程中调用**，会阻塞到扫描结束。
pub fn rebuild_blocking(
    app: &AppHandle,
    state: &FileIndexState,
    roots: Vec<PathBuf>,
    missing_roots: Vec<PathBuf>,
) -> AppResult<()> {
    if !state.is_enabled() {
        return Ok(());
    }

    let generation = state.begin_scan(roots.clone(), missing_roots.clone());
    emit_state(app, &state.snapshot());

    let started = Instant::now();
    let mut last_emit = Instant::now();
    let outcome = walk(&roots, MAX_ENTRIES, &mut |scanned, indexed| {
        if state.is_stale(generation) {
            return false;
        }
        let due = last_emit.elapsed() >= PROGRESS_INTERVAL
            || indexed % PROGRESS_EVERY_ENTRIES == 0
            || scanned % PROGRESS_EVERY_ENTRIES == 0;
        if due {
            last_emit = Instant::now();
            if let Some(next) = state.report_progress(generation, scanned, indexed) {
                emit_state(app, &next);
            }
        }
        true
    });

    let Some(outcome) = outcome else {
        log::info!("文件索引构建已作废（取消或已有新一轮扫描）");
        return Ok(());
    };

    let elapsed_ms = started.elapsed().as_millis() as u64;
    if let Err(err) = save_to_disk(app, &roots, &outcome.entries) {
        // 落盘失败不影响本次可用性，只是下次启动需要重建。
        log::warn!("写入文件索引失败：{err}");
    }

    let truncated = outcome.truncated;
    let scanned = outcome.scanned;
    match state.install(
        generation,
        outcome.entries,
        roots,
        missing_roots,
        now_secs(),
        truncated,
    ) {
        Some(next) => {
            emit_state(app, &next);
            log::info!(
                "文件索引构建完成：索引 {} 项 / 遍历 {} 项，用时 {} ms{}",
                next.indexed,
                scanned,
                elapsed_ms,
                if truncated {
                    "（已达条目上限）"
                } else {
                    ""
                }
            );
        }
        None => log::info!("文件索引构建结果已过期，丢弃"),
    }
    Ok(())
}

struct WalkOutcome {
    entries: Vec<Entry>,
    scanned: u64,
    truncated: bool,
}

/// 遍历若干根目录收集条目。
///
/// `tick(已遍历, 已索引)` 返回 `false` 表示中止（取消 / 过期），此时整体返回 `None`。
fn walk(
    roots: &[PathBuf],
    max_entries: usize,
    tick: &mut dyn FnMut(u64, u64) -> bool,
) -> Option<WalkOutcome> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut scanned: u64 = 0;
    let mut truncated = false;
    let mut stack: Vec<(PathBuf, u32)> = roots.iter().rev().map(|root| (root.clone(), 0)).collect();

    while let Some((dir, depth)) = stack.pop() {
        let reader = match std::fs::read_dir(&dir) {
            Ok(reader) => reader,
            Err(err) => {
                // 无权限 / 目录被删：跳过而不是中断整轮扫描。
                log::debug!("读取目录失败，已跳过：{}（{err}）", dir.display());
                continue;
            }
        };

        for item in reader {
            let Ok(item) = item else { continue };
            let Ok(file_type) = item.file_type() else {
                continue;
            };
            // 符号链接直接跳过（重解析点另有属性判断，两者都覆盖才稳）。
            if file_type.is_symlink() {
                continue;
            }
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            if is_reparse_point(&metadata) {
                continue;
            }

            let path = item.path();
            let is_dir = file_type.is_dir();
            if is_dir {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default();
                if is_skipped_dir(&name) {
                    continue;
                }
                if depth + 1 < MAX_DEPTH {
                    stack.push((path.clone(), depth + 1));
                }
            }

            scanned += 1;
            if entries.len() >= max_entries {
                truncated = true;
                break;
            }
            entries.push(Entry::from_path(path, &metadata, is_dir));

            if !tick(scanned, entries.len() as u64) {
                return None;
            }
        }

        if truncated {
            break;
        }
    }

    Some(WalkOutcome {
        entries,
        scanned,
        truncated,
    })
}

/// 目录名是否在跳过名单里（不分大小写）。
fn is_skipped_dir(name: &str) -> bool {
    SKIP_DIRS
        .iter()
        .any(|skipped| name.eq_ignore_ascii_case(skipped))
}

/// 是否为需要跳过的重解析点（Windows 上的符号链接 / 目录联接）。
#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

fn index_path(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app.path().app_data_dir()?.join(INDEX_FILE_NAME))
}

fn emit_state(app: &AppHandle, state: &IndexState) {
    // 广播失败不该影响索引本身：前端还有 get_file_index_state 兜底。
    if let Err(err) = app.emit(SEARCH_INDEX_EVENT, state) {
        log::warn!("广播文件索引状态失败：{err}");
    }
}

/// 原子落盘：先写临时文件再重命名（照 `config.rs::save` 的模式）。
fn save_to_disk(app: &AppHandle, roots: &[PathBuf], entries: &[Entry]) -> AppResult<()> {
    let path = index_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("bin.tmp");

    {
        let mut writer = BufWriter::new(File::create(&tmp)?);
        writer.write_all(INDEX_MAGIC)?;
        writer.write_all(&(roots.len() as u32).to_le_bytes())?;
        for root in roots {
            write_text(&mut writer, &root.to_string_lossy())?;
        }
        writer.write_all(&(entries.len() as u32).to_le_bytes())?;
        for entry in entries {
            writer.write_all(&[u8::from(entry.is_dir)])?;
            writer.write_all(&entry.size.to_le_bytes())?;
            writer.write_all(&entry.modified_secs.to_le_bytes())?;
            write_text(&mut writer, &entry.path.to_string_lossy())?;
        }
        writer.flush()?;
    }

    std::fs::rename(&tmp, &path)?;
    log::debug!(
        "文件索引已落盘 path={} entries={}",
        path.display(),
        entries.len()
    );
    Ok(())
}

fn write_text(writer: &mut impl Write, text: &str) -> AppResult<()> {
    let bytes = text.as_bytes();
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    Ok(())
}

fn read_text(reader: &mut impl Read) -> AppResult<String> {
    let mut length = [0u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > 32_768 {
        return Err(AppError::Other(format!("索引中的路径长度异常：{length}")));
    }
    let mut bytes = vec![0u8; length];
    reader.read_exact(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_index_file(path: &Path) -> AppResult<(Vec<PathBuf>, Vec<Entry>)> {
    let mut reader = BufReader::new(File::open(path)?);

    let mut magic = [0u8; 6];
    reader.read_exact(&mut magic)?;
    if &magic != INDEX_MAGIC {
        return Err(AppError::Other("索引文件版本不匹配".into()));
    }

    let mut buffer = [0u8; 4];
    reader.read_exact(&mut buffer)?;
    let root_count = u32::from_le_bytes(buffer);
    if root_count > MAX_LOAD_ROOTS {
        return Err(AppError::Other(format!("索引根数量异常：{root_count}")));
    }
    let mut roots = Vec::with_capacity(root_count as usize);
    for _ in 0..root_count {
        roots.push(PathBuf::from(read_text(&mut reader)?));
    }

    reader.read_exact(&mut buffer)?;
    let entry_count = u32::from_le_bytes(buffer);
    if entry_count > MAX_LOAD_ENTRIES {
        return Err(AppError::Other(format!("索引条目数量异常：{entry_count}")));
    }

    let mut entries = Vec::with_capacity(entry_count as usize);
    for _ in 0..entry_count {
        let mut flags = [0u8; 1];
        reader.read_exact(&mut flags)?;
        let mut size = [0u8; 8];
        reader.read_exact(&mut size)?;
        let mut modified = [0u8; 8];
        reader.read_exact(&mut modified)?;
        let path = PathBuf::from(read_text(&mut reader)?);
        entries.push(Entry::from_loaded(
            path,
            u64::from_le_bytes(size),
            i64::from_le_bytes(modified),
            flags[0] & 1 == 1,
        ));
    }

    Ok((roots, entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, parent: &str, is_dir: bool, modified_secs: i64) -> Entry {
        let path = Path::new(parent).join(name);
        Entry::from_loaded(path, 10, modified_secs, is_dir)
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("prism3-file-index-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        dir
    }

    #[test]
    fn name_tiers_are_ordered() {
        assert_eq!(name_tier("report.pdf", "report.pdf"), Some(Tier::Exact));
        assert_eq!(name_tier("report.pdf", "report"), Some(Tier::Prefix));
        assert_eq!(name_tier("my-report.pdf", "report"), Some(Tier::Boundary));
        assert_eq!(name_tier("myreport.pdf", "report"), Some(Tier::Contains));
        assert_eq!(name_tier("other.pdf", "report"), None);
    }

    #[test]
    fn matching_works_for_cjk_without_word_separators() {
        // `name_tier` 收到的是**已小写化**的文件名与关键词（大小写由 `query` 统一处理）。
        // 中文没有词分隔符，故除「开头命中」外都落在 `Contains` 档。
        assert_eq!(name_tier("report", "report"), Some(Tier::Exact));
        assert_eq!(name_tier("report.pdf", "report"), Some(Tier::Prefix));
        assert_eq!(name_tier("报告2024.docx", "报告"), Some(Tier::Prefix));
        assert_eq!(name_tier("年度报告.docx", "报告"), Some(Tier::Contains));
        assert_eq!(name_tier("年度报告.docx", "度报"), Some(Tier::Contains));
        assert_eq!(name_tier("年度报告.docx", "季度"), None);
    }

    #[test]
    fn subsequence_matches_in_order_only() {
        assert!(is_subsequence("report.pdf", "rpt"));
        assert!(is_subsequence("report.pdf", "report"));
        assert!(!is_subsequence("report.pdf", "trp"));
    }

    #[test]
    fn skipped_directories_are_case_insensitive() {
        assert!(is_skipped_dir("node_modules"));
        assert!(is_skipped_dir("Node_Modules"));
        assert!(is_skipped_dir("AppData"));
        assert!(is_skipped_dir("$RECYCLE.BIN"));
        assert!(!is_skipped_dir("Documents"));
        assert!(!is_skipped_dir("target-release"));
    }

    #[test]
    fn query_ranks_by_tier_then_name_length_then_recency() {
        // `install` 会校验扫描代号，故这里走一次真实的 `begin_scan` 拿代号。
        let state = FileIndexState::new();
        state.set_enabled(true);
        let generation = state.begin_scan(vec![PathBuf::from("C:\\docs")], Vec::new());
        state
            .install(
                generation,
                vec![
                    entry("report-final.pdf", "C:\\docs", false, 100),
                    entry("report.pdf", "C:\\docs", false, 50),
                    entry("my-report.txt", "C:\\docs", false, 200),
                    entry("unrelated.md", "C:\\docs", false, 300),
                ],
                vec![PathBuf::from("C:\\docs")],
                Vec::new(),
                now_secs(),
                false,
            )
            .expect("安装索引失败");

        let hits = state.query("report", None);
        let names: Vec<&str> = hits.iter().map(|hit| hit.name.as_str()).collect();
        // 前缀档优先，且同档内名字更短者靠前；`my-report` 属词边界档排在最后。
        assert_eq!(
            names,
            vec!["report.pdf", "report-final.pdf", "my-report.txt"]
        );
        assert!(state.query("Report", None).len() == 3, "大小写不敏感");
        assert!(state.query("unrelated", None).len() == 1);
        assert!(state.query("zzz", None).is_empty());
    }

    #[test]
    fn query_falls_back_to_parent_directory_and_fuzzy() {
        let state = FileIndexState::new();
        state.set_enabled(true);
        let generation = state.begin_scan(vec![PathBuf::from("C:\\projects\\prism3")], Vec::new());
        state
            .install(
                generation,
                vec![
                    entry("main.rs", "C:\\projects\\prism3\\src", false, 10),
                    entry("readme.md", "C:\\docs", false, 20),
                ],
                vec![PathBuf::from("C:\\projects\\prism3")],
                Vec::new(),
                now_secs(),
                false,
            )
            .expect("安装索引失败");

        // 目录名命中（档 4）。
        let hits = state.query("prism3", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "main.rs");
        // 模糊子序列（档 5）。
        let hits = state.query("rdme", None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "readme.md");
    }

    #[test]
    fn fallback_tiers_never_duplicate_a_result() {
        // 同一文件可能同时命中「名字」与「模糊」两档，结果里只能出现一次。
        let state = FileIndexState::new();
        state.set_enabled(true);
        let generation = state.begin_scan(vec![PathBuf::from("C:\\docs")], Vec::new());
        state
            .install(
                generation,
                vec![entry("report.pdf", "C:\\docs", false, 5)],
                vec![PathBuf::from("C:\\docs")],
                Vec::new(),
                now_secs(),
                false,
            )
            .expect("安装索引失败");

        assert_eq!(state.query("report", None).len(), 1, "名字档");
        assert_eq!(state.query("rpt", None).len(), 1, "模糊档");
        assert_eq!(state.query("docs", None).len(), 1, "目录档");
    }

    #[test]
    fn empty_query_returns_recent_files_only() {
        let state = FileIndexState::new();
        state.set_enabled(true);
        let generation = state.begin_scan(vec![PathBuf::from("C:\\docs")], Vec::new());
        state
            .install(
                generation,
                vec![
                    entry("old.txt", "C:\\docs", false, 10),
                    entry("new.txt", "C:\\docs", false, 90),
                    entry("folder", "C:\\docs", true, 999),
                ],
                vec![PathBuf::from("C:\\docs")],
                Vec::new(),
                now_secs(),
                false,
            )
            .expect("安装索引失败");

        let hits = state.query("  ", None);
        assert_eq!(hits.len(), 2, "目录不参与「最近的文件」");
        assert_eq!(hits[0].name, "new.txt");
    }

    #[test]
    fn query_is_empty_when_disabled_or_empty() {
        let state = FileIndexState::new();
        assert!(state.query("a", None).is_empty(), "未启用时不出结果");

        state.set_enabled(true);
        assert!(state.query("a", None).is_empty(), "空索引不出结果");
    }

    #[test]
    fn limit_is_clamped() {
        let state = FileIndexState::new();
        state.set_enabled(true);
        let generation = state.begin_scan(vec![PathBuf::from("C:\\docs")], Vec::new());
        let entries: Vec<Entry> = (0..100)
            .map(|index| {
                entry(
                    &format!("file-{index:03}.txt"),
                    "C:\\docs",
                    false,
                    index as i64,
                )
            })
            .collect();
        state
            .install(
                generation,
                entries,
                vec![PathBuf::from("C:\\docs")],
                Vec::new(),
                now_secs(),
                false,
            )
            .expect("安装索引失败");

        assert_eq!(state.query("file", None).len(), DEFAULT_LIMIT as usize);
        assert_eq!(state.query("file", Some(5)).len(), 5);
        assert_eq!(state.query("file", Some(999)).len(), MAX_LIMIT as usize);
        assert_eq!(state.query("file", Some(0)).len(), 1);
    }

    #[test]
    fn stale_scans_are_discarded() {
        let state = FileIndexState::new();
        state.set_enabled(true);
        let generation = state.begin_scan(vec![PathBuf::from("C:\\docs")], Vec::new());
        // 新一轮开始后，旧代号的结果必须被丢弃。
        let _newer = state.begin_scan(vec![PathBuf::from("D:\\docs")], Vec::new());
        assert!(state
            .install(
                generation,
                vec![entry("a.txt", "C:\\docs", false, 1)],
                vec![PathBuf::from("C:\\docs")],
                Vec::new(),
                now_secs(),
                false,
            )
            .is_none());
        assert!(state.query("a", None).is_empty());
    }

    #[test]
    fn cancel_stops_progress_and_keeps_state_usable() {
        let state = FileIndexState::new();
        state.set_enabled(true);
        let generation = state.begin_scan(Vec::new(), Vec::new());
        state.cancel();
        assert!(state.report_progress(generation, 1, 1).is_none());
        let snapshot = state.snapshot();
        assert_eq!(snapshot.phase, IndexPhase::Idle);
    }

    #[test]
    fn disabling_invalidates_scans_and_query() {
        let state = FileIndexState::new();
        state.set_enabled(true);
        let generation = state.begin_scan(Vec::new(), Vec::new());
        state.set_enabled(false);
        assert!(!state.is_enabled());
        assert!(state.report_progress(generation, 1, 1).is_none());
        assert_eq!(state.snapshot().phase, IndexPhase::Idle);
    }

    #[test]
    fn walk_collects_files_and_skips_blocked_dirs() {
        let root = temp_dir("walk");
        std::fs::create_dir_all(root.join("sub")).expect("创建子目录失败");
        std::fs::create_dir_all(root.join("node_modules")).expect("创建跳过目录失败");
        std::fs::write(root.join("a.txt"), b"a").expect("写文件失败");
        std::fs::write(root.join("sub").join("b.txt"), b"b").expect("写文件失败");
        std::fs::write(root.join("node_modules").join("c.txt"), b"c").expect("写文件失败");

        let mut ticks = 0u64;
        let outcome = walk(std::slice::from_ref(&root), MAX_ENTRIES, &mut |_, _| {
            ticks += 1;
            true
        })
        .expect("遍历不应被中止");

        let names: Vec<String> = outcome
            .entries
            .iter()
            .map(|item| item.name_lower.clone())
            .collect();
        assert!(names.contains(&"a.txt".to_string()));
        assert!(names.contains(&"b.txt".to_string()));
        assert!(names.contains(&"sub".to_string()));
        assert!(
            !names.contains(&"c.txt".to_string()),
            "跳过名单内的目录不入索引"
        );
        assert_eq!(ticks, outcome.entries.len() as u64);
        assert!(!outcome.truncated);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn walk_aborts_when_tick_asks_to_stop() {
        let root = temp_dir("abort");
        std::fs::write(root.join("a.txt"), b"a").expect("写文件失败");

        let outcome = walk(std::slice::from_ref(&root), MAX_ENTRIES, &mut |_, _| false);
        assert!(outcome.is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn walk_stops_at_entry_cap() {
        let root = temp_dir("cap");
        for index in 0..10 {
            std::fs::write(root.join(format!("f{index}.txt")), b"x").expect("写文件失败");
        }

        let outcome =
            walk(std::slice::from_ref(&root), 4, &mut |_, _| true).expect("遍历不应被中止");
        assert_eq!(outcome.entries.len(), 4);
        assert!(outcome.truncated);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn index_file_round_trip() {
        let root = temp_dir("roundtrip");
        let file = root.join("index.bin");
        let entries = vec![
            entry("a.txt", "C:\\docs", false, 11),
            Entry::from_loaded(root.join("sub"), 0, 22, true),
        ];

        {
            let mut writer = BufWriter::new(File::create(&file).expect("创建索引文件失败"));
            writer.write_all(INDEX_MAGIC).unwrap();
            writer.write_all(&1u32.to_le_bytes()).unwrap();
            write_text(&mut writer, "C:\\docs").unwrap();
            writer
                .write_all(&(entries.len() as u32).to_le_bytes())
                .unwrap();
            for item in &entries {
                writer.write_all(&[u8::from(item.is_dir)]).unwrap();
                writer.write_all(&item.size.to_le_bytes()).unwrap();
                writer.write_all(&item.modified_secs.to_le_bytes()).unwrap();
                write_text(&mut writer, &item.path.to_string_lossy()).unwrap();
            }
            writer.flush().unwrap();
        }

        let (roots, loaded) = read_index_file(&file).expect("读取索引失败");
        assert_eq!(roots, vec![PathBuf::from("C:\\docs")]);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name_lower, "a.txt");
        assert_eq!(loaded[0].size, 10);
        assert_eq!(loaded[0].modified_secs, 11);
        assert!(!loaded[0].is_dir);
        assert!(loaded[1].is_dir);
        assert_eq!(loaded[1].modified_secs, 22);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_index_files_are_rejected_without_panic() {
        let root = temp_dir("corrupt");

        let wrong_magic = root.join("wrong-magic.bin");
        std::fs::write(&wrong_magic, b"NOPE!!\x01\x00\x00\x00").unwrap();
        assert!(read_index_file(&wrong_magic).is_err());

        let truncated = root.join("truncated.bin");
        std::fs::write(&truncated, b"P3IDX1\x02\x00\x00\x00").unwrap();
        assert!(read_index_file(&truncated).is_err());

        let empty = root.join("empty.bin");
        std::fs::write(&empty, b"").unwrap();
        assert!(read_index_file(&empty).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn path_within_roots_requires_membership() {
        let root = temp_dir("within");
        let inside = root.join("nested");
        std::fs::create_dir_all(&inside).unwrap();
        let file = inside.join("a.txt");
        std::fs::write(&file, b"a").unwrap();

        let roots = vec![root.clone()];
        assert!(path_within_roots(&file, &roots));
        assert!(path_within_roots(&inside, &roots));
        assert!(!path_within_roots(
            Path::new("C:\\Windows\\notepad.exe"),
            &roots
        ));

        let _ = std::fs::remove_dir_all(&root);
    }
}

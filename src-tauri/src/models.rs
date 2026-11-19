//! 速记（笔记）领域的模型。
//!
//! 全部结构 `#[serde(rename_all = "camelCase", default)]`，与 `clipboard.rs` / `config.rs`
//! 的既有约定一致；时间字段一律 **INTEGER 秒级 unix 时间戳**（`db::now_secs()`），
//! 与前端 `formatTimeAgo(secs)` 同口径。

use serde::{Deserialize, Serialize};

/// 一条笔记（正文 + 元数据）。`deleted_at` 有值即在回收站。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub created_at: u64,
    pub updated_at: u64,
    /// 所在文件夹（`None` = 树根）。
    pub folder_id: Option<i64>,
    /// 剪藏来源 URL（普通笔记为空串）；同 URL 落库即去重键。
    pub source_url: String,
    /// 回收站标记（`None` = 正常）。
    pub deleted_at: Option<u64>,
    /// 自定义树图标（emoji；`None` = 默认文件图标）。
    pub icon: Option<String>,
}

/// 笔记文件夹（单归属、可嵌套；`builtin` 的「剪藏」不可改名/删除）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NoteFolder {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub sort_order: i64,
    pub builtin: bool,
    pub created_at: u64,
}

/// 笔记标签（`builtin` 的「剪藏」不可改名/删除）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub created_at: u64,
    pub builtin: bool,
}

/// 双链出链项（`to_note_id = None` 表示「未链接提及」，引用了不存在的标题）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NoteLinkOut {
    pub to_title: String,
    pub to_note_id: Option<i64>,
}

/// 双链入链项（谁引用了我）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NoteLinkIn {
    pub from_note_id: i64,
    pub from_title: String,
}

/// 反链面板数据（`get_note_links`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NoteLinks {
    pub outgoing: Vec<NoteLinkOut>,
    pub incoming: Vec<NoteLinkIn>,
}

/// 笔记-标签关联对（前端构建 `tag_id → noteIds` 筛选映射用）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NoteTagRow {
    pub note_id: i64,
    pub tag_id: i64,
}

/// 工作记录的项目（单层容器，不嵌套；删除只解除日志归属，不删日志）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkProject {
    pub id: i64,
    pub name: String,
    pub archived_at: Option<u64>,
    pub created_at: u64,
}

/// 一条工作记录（台账主表）。
///
/// `status = "done"` 是已完成的记录，`"planned"` 是提前列出的未来工作
/// （此时 `finished_at` 表示**计划日期**）。
/// `hours = 0` 表示「没填工时」；`project_name` 由查询时 JOIN 带出，
/// 项目删除后为空串。`source` 预留 `manual` / `task` / `git`，本版只写 `manual`。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkLog {
    pub id: i64,
    pub title: String,
    pub detail: String,
    pub project_id: Option<i64>,
    pub project_name: String,
    pub kind: String,
    pub status: String,
    pub hours: f64,
    pub finished_at: u64,
    pub source: String,
    pub created_at: u64,
    pub updated_at: u64,
}

/// 回收站保留策略下的清理报告（`purge_expired_notes`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PurgeReport {
    pub purged: usize,
}

/// 孤儿图片清理报告（先 `dry_run` 出报告，用户确认后再真删）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NoteImageGcReport {
    pub dry_run: bool,
    /// `notes/images` 下图片总数。
    pub total_files: usize,
    /// 被活笔记（含回收站）引用的图片数。
    pub referenced: usize,
    /// 孤儿图片文件名列表（hash.ext）。
    pub orphan_files: Vec<String>,
    /// 真删模式下的实际删除数（`dry_run` 为 0）。
    pub removed: usize,
    /// 删除失败的文件数（被占用等）。
    pub failed: usize,
}

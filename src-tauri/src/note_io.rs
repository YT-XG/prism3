//! 速记导出 / 导入。
//!
//! - 导出：按文件夹树建目录，每条笔记一个 `.md` + front-matter
//!   （`title` / `created_at` / `source_url` / `folder` / `icon` / `tags`），
//!   正文里的图片 URL 改相对路径 `assets/<hash>.<ext>` 并把用到的图复制进 `assets/`。
//! - 导入（限定档）：只导**我们自己导出的产物**（front-matter 格式已知、闭环）。
//! - 手写 `key: value` 行解析，不引 YAML 依赖。
//! - 逐条事务，失败不回滚整体；导入可取消（`IMPORT_CANCELLED` 原子标志，逐条检查）。
//!
//! 时间列是 INTEGER 秒级时间戳（与库一致），front-matter 里也写成整数秒。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::db::DbState;
use crate::error::{AppError, AppResult};
use crate::note_image::{self, NOTE_URL_PREFIX};
use crate::repo::{note, note_folder, note_link, tag};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub exported: usize,
    /// 复制进 `assets/` 的图片数。
    pub images: usize,
    pub failed: Vec<String>,
    pub dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: usize,
    /// `source_url` 命中已有笔记而跳过（去重）。
    pub skipped: usize,
    /// 失败条目（文件名 + 原因）。
    pub failed: Vec<String>,
    pub cancelled: bool,
}

/// 导入选项（前端可传 `null`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportOptions {
    /// `source_url` 命中已有笔记时是否覆盖（默认跳过）。
    pub overwrite: bool,
}

static IMPORT_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 取消正在进行的导入（幂等）。
pub fn cancel_import() {
    IMPORT_CANCELLED.store(true, Ordering::SeqCst);
}

/// 导出快照：四张表一次读齐，之后导出全程不再碰数据库。
pub type ExportSnapshot = (
    Vec<crate::models::Note>,
    Vec<crate::models::NoteFolder>,
    Vec<crate::models::Tag>,
    Vec<crate::models::NoteTagRow>,
);

pub fn read_export_snapshot(conn: &rusqlite::Connection) -> AppResult<ExportSnapshot> {
    Ok((
        note::list_full(conn, false)?,
        note_folder::list(conn)?,
        tag::list(conn)?,
        tag::list_note_tags(conn)?,
    ))
}

// ---------- 导出 ----------

pub fn export(app: &AppHandle, snapshot: ExportSnapshot, root: &Path) -> AppResult<ExportReport> {
    let (notes, folders, tags, note_tags) = snapshot;
    std::fs::create_dir_all(root)?;
    let mut tag_names: HashMap<i64, String> = HashMap::new();
    for t in &tags {
        tag_names.insert(t.id, t.name.clone());
    }
    let mut tags_of: HashMap<i64, Vec<String>> = HashMap::new();
    for row in &note_tags {
        if let Some(name) = tag_names.get(&row.tag_id) {
            tags_of.entry(row.note_id).or_default().push(name.clone());
        }
    }

    let mut dir_cache: HashMap<i64, PathBuf> = HashMap::new();
    let mut used_files: HashSet<String> = HashSet::new();
    let mut copied_images: HashSet<String> = HashSet::new();
    let assets_dir = root.join("assets");
    let mut report = ExportReport {
        exported: 0,
        images: 0,
        failed: Vec::new(),
        dir: root.display().to_string(),
    };
    let images_root = note_image::image_dir(app).unwrap_or_else(|_| root.join("assets"));

    for n in &notes {
        let folder_dir = match n.folder_id {
            Some(fid) => match ensure_folder_dir(&folders, fid, root, &mut dir_cache) {
                Ok(path) => path,
                Err(err) => {
                    report.failed.push(format!("{}: {err}", n.title));
                    continue;
                }
            },
            None => root.to_path_buf(),
        };
        let stem = unique_filename(&n.title, &folder_dir, &mut used_files);
        let body = n.content.replace(NOTE_URL_PREFIX, "assets/");
        for name in extract_asset_names(&body) {
            if copied_images.insert(name.clone()) {
                let src = images_root.join(&name);
                if src.is_file() {
                    std::fs::create_dir_all(&assets_dir).ok();
                    if let Err(err) = std::fs::copy(&src, assets_dir.join(&name)) {
                        report
                            .failed
                            .push(format!("{} 图片 {name}: {err}", n.title));
                    } else {
                        report.images += 1;
                    }
                }
            }
        }
        let mut md = String::new();
        md.push_str("---\n");
        md.push_str(&format!("title: {}\n", n.title));
        md.push_str(&format!("created_at: {}\n", n.created_at));
        md.push_str(&format!("source_url: {}\n", n.source_url));
        md.push_str(&format!(
            "folder: {}\n",
            folder_rel_path(&folders, n.folder_id)
        ));
        if let Some(icon) = n.icon.as_deref().filter(|s| !s.is_empty()) {
            md.push_str(&format!("icon: {icon}\n"));
        }
        md.push_str(&format!(
            "tags: {}\n",
            tags_of.get(&n.id).cloned().unwrap_or_default().join(", ")
        ));
        md.push_str("---\n\n");
        let body = body.trim_start().to_string();
        md.push_str(&body);
        if !body.is_empty() && !body.ends_with('\n') {
            md.push('\n');
        }
        let path = folder_dir.join(format!("{stem}.md"));
        if let Err(err) = std::fs::write(&path, md) {
            report.failed.push(format!("{}: {err}", n.title));
            continue;
        }
        report.exported += 1;
    }
    if !report.failed.is_empty() {
        log::warn!("速记导出部分失败: {} 条", report.failed.len());
    }
    log::info!(
        "速记导出完成: {} 条笔记，{} 张图片 → {}",
        report.exported,
        report.images,
        report.dir
    );
    Ok(report)
}

fn ensure_folder_dir(
    folders: &[crate::models::NoteFolder],
    id: i64,
    root: &Path,
    cache: &mut HashMap<i64, PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(path) = cache.get(&id) {
        return Ok(path.clone());
    }
    let folder = folders
        .iter()
        .find(|folder| folder.id == id)
        .ok_or_else(|| format!("文件夹 {id} 不存在"))?;
    let parent_dir = match folder.parent_id {
        Some(parent_id) => ensure_folder_dir(folders, parent_id, root, cache)?,
        None => root.to_path_buf(),
    };
    let dir = parent_dir.join(sanitize_filename(&folder.name));
    std::fs::create_dir_all(&dir).map_err(|err| format!("创建目录失败: {err}"))?;
    cache.insert(id, dir.clone());
    Ok(dir)
}

fn folder_rel_path(folders: &[crate::models::NoteFolder], folder_id: Option<i64>) -> String {
    let mut names: Vec<String> = Vec::new();
    let mut cursor = folder_id;
    let mut hops = 0;
    while let Some(current) = cursor {
        hops += 1;
        if hops > 100 {
            break;
        }
        match folders.iter().find(|folder| folder.id == current) {
            Some(folder) => {
                names.push(folder.name.clone());
                cursor = folder.parent_id;
            }
            None => break,
        }
    }
    names.reverse();
    names.join("/")
}

/// Windows 文件名清洗：非法字符替换、结尾点/空格去掉、保留名加前缀、超长截断。
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if (c as u32) < 0x20 => ' ',
            c => c,
        })
        .collect();
    let mut s = cleaned.trim().to_string();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s.is_empty() {
        return "无标题笔记".to_string();
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem.as_str()) {
        s = format!("_{s}");
    }
    if s.chars().count() > 200 {
        s = s.chars().take(200).collect();
        while s.ends_with('.') || s.ends_with(' ') {
            s.pop();
        }
    }
    s
}

fn unique_filename(title: &str, dir: &Path, used: &mut HashSet<String>) -> String {
    let base = sanitize_filename(title);
    let mut candidate = base.clone();
    let mut n = 2;
    loop {
        let key = format!("{}\u{0}{}", dir.display(), candidate);
        let taken = used.contains(&key)
            || dir.join(format!("{candidate}.md")).exists()
            || dir.join(&candidate).exists();
        if !taken {
            used.insert(key);
            return candidate;
        }
        candidate = format!("{base} ({n})");
        n += 1;
    }
}

fn extract_asset_names(body: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = body;
    while let Some(pos) = rest.find("assets/") {
        let tail = &rest[pos + "assets/".len()..];
        let name: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
            .collect();
        let stem = name.split('.').next().unwrap_or("");
        if stem.len() == 16 && stem.bytes().all(|b| b.is_ascii_hexdigit()) && name.contains('.') {
            names.push(name.clone());
        }
        rest = tail;
    }
    names
}

// ---------- 导入 ----------

/// 解析 front-matter：首行 `---`，到下一个 `---` 行为止的 `key: value` 行。
/// 没等到闭合 `---` 时按无 front-matter 处理，正文原样保留（宁漏勿改）。
pub fn parse_front_matter(md: &str) -> (HashMap<String, String>, String) {
    let text = md.strip_prefix('\u{feff}').unwrap_or(md);
    let mut keys = HashMap::new();
    let mut rest = text;
    let mut first = true;
    while let Some(nl) = rest.find('\n') {
        let mut line = &rest[..nl];
        rest = &rest[nl + 1..];
        if line.ends_with('\r') {
            line = &line[..line.len() - 1];
        }
        if first {
            first = false;
            if line.trim() != "---" {
                return (HashMap::new(), md.to_string());
            }
            continue;
        }
        if line.trim() == "---" {
            return (keys, rest.to_string());
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            if !key.is_empty() && !key.contains(' ') {
                keys.insert(key, value);
            }
        }
    }
    (HashMap::new(), md.to_string())
}

/// 导入（调用方负责不在锁里做文件 IO：本函数逐条短暂持锁）。
pub fn import(
    app: &AppHandle,
    state: &DbState,
    root: &Path,
    opts: ImportOptions,
) -> AppResult<ImportReport> {
    if !root.is_dir() {
        return Err(AppError::Other(format!("目录不存在: {}", root.display())));
    }
    IMPORT_CANCELLED.store(false, Ordering::SeqCst);
    let mut report = ImportReport {
        imported: 0,
        skipped: 0,
        failed: Vec::new(),
        cancelled: false,
    };
    let mut md_files: Vec<PathBuf> = Vec::new();
    collect_md_files(root, &mut md_files, 0);
    if md_files.is_empty() {
        return Err(AppError::Other("所选目录下没有 .md 文件".into()));
    }
    let images_root = note_image::image_dir(app)?;
    std::fs::create_dir_all(&images_root)?;
    let mut folder_cache: HashMap<String, Option<i64>> = HashMap::new();

    for path in md_files {
        if IMPORT_CANCELLED.load(Ordering::SeqCst) {
            report.cancelled = true;
            break;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => {
                report.failed.push(format!("{rel}: {err}"));
                continue;
            }
        };
        let (meta, body) = parse_front_matter(&content);
        if meta.is_empty() {
            report.failed.push(format!(
                "{rel}: 缺少 front-matter（仅支持导入本应用导出的产物）"
            ));
            continue;
        }
        let title = meta
            .get("title")
            .cloned()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("无标题笔记")
                    .to_string()
            });
        // 图片：assets/<name> 复制进 notes/images（沿用内容哈希名）
        for name in extract_asset_names(&body) {
            let candidates = match path.parent() {
                Some(parent) => vec![
                    parent.join("assets").join(&name),
                    root.join("assets").join(&name),
                ],
                None => vec![root.join("assets").join(&name)],
            };
            for candidate in candidates {
                if candidate.is_file() {
                    let dst = images_root.join(&name);
                    if !dst.exists() {
                        if let Err(err) = std::fs::copy(&candidate, &dst) {
                            report.failed.push(format!("{rel} 图片 {name}: {err}"));
                        }
                    }
                    break;
                }
            }
        }
        let body_final = body.replace("assets/", NOTE_URL_PREFIX);
        let outcome = {
            let conn = state.lock();
            import_one(
                &conn,
                &title,
                &body_final,
                &meta,
                opts.overwrite,
                &mut folder_cache,
            )
        };
        match outcome {
            Ok(ImportOne::Imported) => report.imported += 1,
            Ok(ImportOne::Skipped) => {
                report.skipped += 1;
                continue;
            }
            Err(err) => report.failed.push(format!("{rel}: {err}")),
        }
        if report.imported % 50 == 0 {
            let _ = app.emit("notes-import-progress", report.imported);
        }
    }
    log::info!(
        "速记导入结束: 成功 {} / 跳过 {} / 失败 {}（取消: {}）",
        report.imported,
        report.skipped,
        report.failed.len(),
        report.cancelled
    );
    Ok(report)
}

enum ImportOne {
    Imported,
    Skipped,
}

fn import_one(
    conn: &rusqlite::Connection,
    title: &str,
    body: &str,
    meta: &HashMap<String, String>,
    overwrite: bool,
    folder_cache: &mut HashMap<String, Option<i64>>,
) -> Result<ImportOne, String> {
    let source_url = meta.get("source_url").cloned().unwrap_or_default();
    let mut overwrite_id: Option<i64> = None;
    if !source_url.is_empty() {
        let dup: Option<i64> = conn
            .query_row(
                "SELECT id FROM notes WHERE source_url = ?1 LIMIT 1",
                [source_url.as_str()],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = dup {
            if overwrite {
                overwrite_id = Some(id);
            } else {
                return Ok(ImportOne::Skipped);
            }
        }
    }
    let folder_rel = meta.get("folder").cloned().unwrap_or_default();
    let folder_id = resolve_folder(conn, &folder_rel, folder_cache)?;
    let created_at = meta
        .get("created_at")
        .and_then(|value| value.parse::<u64>().ok());
    let icon = meta.get("icon").cloned();
    let tags_value = meta.get("tags").cloned().unwrap_or_default();

    (|conn: &rusqlite::Connection| -> Result<(), String> {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let id = if let Some(oid) = overwrite_id {
            tx.execute(
                "UPDATE notes SET title = ?1, content = ?2, folder_id = ?3, icon = ?4 WHERE id = ?5",
                rusqlite::params![title, body, folder_id, icon, oid],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM note_tags WHERE note_id = ?1",
                rusqlite::params![oid],
            )
            .map_err(|e| e.to_string())?;
            oid
        } else {
            let ts = created_at.unwrap_or_else(crate::db::now_secs);
            tx.execute(
                "INSERT INTO notes (title, content, folder_id, source_url, icon, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                rusqlite::params![title, body, folder_id, source_url, icon, ts],
            )
            .map_err(|e| e.to_string())?;
            tx.last_insert_rowid()
        };
        note_link::reindex(&tx, id, body).map_err(|e| e.to_string())?;
        for name in tags_value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            tx.execute(
                "INSERT OR IGNORE INTO tags (name, created_at) VALUES (?1, ?2)",
                rusqlite::params![name, crate::db::now_secs()],
            )
            .map_err(|e| e.to_string())?;
            let tag_id: i64 = tx
                .query_row(
                    "SELECT id FROM tags WHERE name = ?1",
                    rusqlite::params![name],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT OR IGNORE INTO note_tags (note_id, tag_id) VALUES (?1, ?2)",
                rusqlite::params![id, tag_id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })(conn)?;
    Ok(ImportOne::Imported)
}

fn collect_md_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 20 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_md_files(&path, out, depth + 1);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

fn resolve_folder(
    conn: &rusqlite::Connection,
    rel: &str,
    cache: &mut HashMap<String, Option<i64>>,
) -> Result<Option<i64>, String> {
    let rel = rel.trim().trim_matches('/').to_string();
    if rel.is_empty() {
        return Ok(None);
    }
    if let Some(id) = cache.get(&rel) {
        return Ok(*id);
    }
    let mut parent: Option<i64> = None;
    let mut built = String::new();
    for segment in rel.split('/') {
        let name = segment.trim();
        if name.is_empty() {
            continue;
        }
        if !built.is_empty() {
            built.push('/');
        }
        built.push_str(name);
        if let Some(id) = cache.get(&built) {
            parent = *id;
            continue;
        }
        let found: Option<i64> = conn
            .query_row(
                "SELECT id FROM note_folders WHERE name = ?1 AND parent_id IS ?2 LIMIT 1",
                rusqlite::params![name, parent],
                |row| row.get(0),
            )
            .ok();
        let id = match found {
            Some(id) => id,
            None => {
                note_folder::create(conn, name, parent)
                    .map_err(|e| e.to_string())?
                    .id
            }
        };
        cache.insert(built.clone(), Some(id));
        parent = Some(id);
    }
    cache.insert(rel, parent);
    Ok(parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_windows_filenames() {
        assert_eq!(
            sanitize_filename("a<b>:c/d\\e|f?g*h\"i"),
            "a b  c d e f g h i"
        );
        assert_eq!(sanitize_filename("  结尾点... "), "结尾点");
        assert_eq!(sanitize_filename("..."), "无标题笔记");
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("con.md"), "_con.md");
        assert_eq!(sanitize_filename("正常 标题.md"), "正常 标题.md");
        assert_eq!(sanitize_filename(&"长".repeat(300)).chars().count(), 200);
    }

    #[test]
    fn front_matter_parse_roundtrip() {
        let md = "---\ntitle: 甲\ncreated_at: 1700000000\nsource_url: https://x\ntags: 工作, 生活\nfolder: 剪藏/深度\n---\n\n正文第一行\n";
        let (meta, body) = parse_front_matter(md);
        assert_eq!(meta.get("title").map(String::as_str), Some("甲"));
        assert_eq!(
            meta.get("created_at").map(String::as_str),
            Some("1700000000")
        );
        assert_eq!(meta.get("tags").map(String::as_str), Some("工作, 生活"));
        assert_eq!(meta.get("folder").map(String::as_str), Some("剪藏/深度"));
        assert!(body.trim_start().starts_with("正文第一行"));
        // 无 front-matter → 空 meta、全文为 body
        let (meta, body) = parse_front_matter("# 纯 markdown\n");
        assert!(meta.is_empty());
        assert!(body.contains("# 纯 markdown"));
    }

    #[test]
    fn asset_name_extraction_only_accepts_hashes() {
        let names = extract_asset_names("![](assets/0123456789abcdef.png) 与 assets/notahash.png");
        assert_eq!(names, vec!["0123456789abcdef.png".to_string()]);
    }
}

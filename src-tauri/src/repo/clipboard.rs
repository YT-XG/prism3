//! 剪贴板历史的持久化（SQLite 为权威）：原 JSON 版本的内存 `entries` 全部改由这里承载。
//! 图片文件本体仍在 `clipboard-images` 目录（由 `clipboard.rs` 读写），本模块只存文件名与元数据。

use rusqlite::{params, Connection, OptionalExtension, Result};

use crate::clipboard::{ClipEntry, ClipKind, ClipKindFilter, ImageMeta};

/// 查询列顺序，必须与 `row_to_entry` 一一对应。
const CLIP_COLS: &str = "id, kind, text, digest, size, pinned, created_at, \
     image_file, image_thumb, image_width, image_height, image_bytes";

fn kind_from_str(raw: &str) -> ClipKind {
    if raw == "image" {
        ClipKind::Image
    } else {
        ClipKind::Text
    }
}

fn row_to_entry(row: &rusqlite::Row) -> Result<ClipEntry> {
    let kind = kind_from_str(&row.get::<_, String>(1)?);
    let image_file: Option<String> = row.get(7)?;
    let image = if kind == ClipKind::Image {
        image_file.map(|file| ImageMeta {
            width: row.get::<_, Option<u32>>(9).unwrap_or(None).unwrap_or(0),
            height: row.get::<_, Option<u32>>(10).unwrap_or(None).unwrap_or(0),
            file,
            thumb: row
                .get::<_, Option<String>>(8)
                .unwrap_or(None)
                .unwrap_or_default(),
            bytes: row.get::<_, Option<u64>>(11).unwrap_or(None).unwrap_or(0),
        })
    } else {
        None
    };
    Ok(ClipEntry {
        id: row.get::<_, i64>(0)? as u64,
        kind,
        text: row.get(2)?,
        digest: row.get(3)?,
        size: row.get::<_, i64>(4)? as u64,
        pinned: row.get::<_, i64>(5)? != 0,
        created_at: row.get::<_, i64>(6)? as u64,
        image,
    })
}

fn find_id_by_digest(conn: &Connection, digest: &str) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM clipboard_history WHERE digest = ?1",
        params![digest],
        |r| r.get(0),
    )
    .optional()
}

/// 插入 / 复用一条文本记录。同 digest 命中则只把 `created_at` 提到最新（复用原 id、
/// **保留 pinned**）；否则新插一行。返回 `(id, 是否命中已有)`。
pub fn upsert_text(
    conn: &Connection,
    digest: &str,
    text: &str,
    size: u64,
    created_at: u64,
) -> Result<(i64, bool)> {
    if let Some(id) = find_id_by_digest(conn, digest)? {
        conn.execute(
            "UPDATE clipboard_history SET created_at = ?1 WHERE id = ?2",
            params![created_at, id],
        )?;
        return Ok((id, true));
    }
    conn.execute(
        "INSERT INTO clipboard_history (kind, text, digest, size, pinned, created_at)
         VALUES ('text', ?1, ?2, ?3, 0, ?4)",
        params![text, digest, size, created_at],
    )?;
    Ok((conn.last_insert_rowid(), false))
}

/// 插入 / 复用一条图片记录（图片元数据留空，`set_image` 回填）。返回 `(id, 是否命中已有)`。
pub fn upsert_image_stub(conn: &Connection, digest: &str, created_at: u64) -> Result<(i64, bool)> {
    if let Some(id) = find_id_by_digest(conn, digest)? {
        conn.execute(
            "UPDATE clipboard_history SET created_at = ?1 WHERE id = ?2",
            params![created_at, id],
        )?;
        return Ok((id, true));
    }
    conn.execute(
        "INSERT INTO clipboard_history (kind, digest, size, pinned, created_at)
         VALUES ('image', ?1, 0, 0, ?2)",
        params![digest, created_at],
    )?;
    Ok((conn.last_insert_rowid(), false))
}

/// 回填图片元数据（文件落盘后调用）。
pub fn set_image(conn: &Connection, id: i64, image: &ImageMeta) -> Result<()> {
    conn.execute(
        "UPDATE clipboard_history
         SET image_file = ?1, image_thumb = ?2, image_width = ?3, image_height = ?4, image_bytes = ?5, size = ?5
         WHERE id = ?6",
        params![
            image.file,
            image.thumb,
            image.width,
            image.height,
            image.bytes,
            id
        ],
    )?;
    Ok(())
}

/// 单条（文本 / 图片）读取。
pub fn get(conn: &Connection, id: i64) -> Result<Option<ClipEntry>> {
    conn.query_row(
        &format!("SELECT {CLIP_COLS} FROM clipboard_history WHERE id = ?1"),
        params![id],
        row_to_entry,
    )
    .optional()
}

/// 全量读取（启动时校验图片文件是否存在用）。
pub fn all(conn: &Connection) -> Result<Vec<ClipEntry>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CLIP_COLS} FROM clipboard_history ORDER BY pinned DESC, created_at DESC, id DESC"
    ))?;
    let rows = stmt.query_map([], row_to_entry)?;
    rows.collect()
}

/// 查询历史（**收藏置顶**，组内按最近入库；空关键词返回最近条目）。
pub fn query(
    conn: &Connection,
    needle: &str,
    kind: ClipKindFilter,
    limit: usize,
) -> Result<Vec<ClipEntry>> {
    let kind_cond = match kind {
        ClipKindFilter::All => "",
        ClipKindFilter::Text => " AND kind = 'text'",
        ClipKindFilter::Image => " AND kind = 'image'",
    };
    // 关键词里的 LIKE 元字符转义，避免 `%` / `_` 被当通配符。
    let escaped = needle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let has_needle = !needle.is_empty();
    let text_cond = if has_needle {
        " AND text LIKE ?1 ESCAPE '\\'"
    } else {
        ""
    };
    let sql = format!(
        "SELECT {CLIP_COLS} FROM clipboard_history
         WHERE 1 = 1{kind_cond}{text_cond}
         ORDER BY pinned DESC, created_at DESC, id DESC LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let pattern = format!("%{escaped}%");
    let rows = if has_needle {
        stmt.query_map(params![pattern], row_to_entry)?
    } else {
        stmt.query_map([], row_to_entry)?
    };
    rows.collect()
}

/// 统计：`(总条数, 收藏条数, 占用字节)`。
pub fn stats(conn: &Connection) -> Result<(usize, usize, u64)> {
    let (count, pinned, bytes): (i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(pinned), 0), COALESCE(SUM(size), 0) FROM clipboard_history",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok((count as usize, pinned as usize, bytes as u64))
}

/// 修改一条文本记录的正文（digest / size 一并重算）。同 digest 撞上别的记录时，
/// 先删掉那条旧记录，避免 UNIQUE 冲突。
pub fn update_text(conn: &Connection, id: i64, text: &str, digest: &str, size: u64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM clipboard_history WHERE digest = ?1 AND id <> ?2",
        params![digest, id],
    )?;
    tx.execute(
        "UPDATE clipboard_history SET text = ?1, digest = ?2, size = ?3 WHERE id = ?4",
        params![text, digest, size, id],
    )?;
    tx.commit()
}

/// 设置 / 取消收藏。
pub fn set_pinned(conn: &Connection, id: i64, pinned: bool) -> Result<()> {
    conn.execute(
        "UPDATE clipboard_history SET pinned = ?1 WHERE id = ?2",
        params![pinned as i64, id],
    )?;
    Ok(())
}

/// 删除一条记录，返回 `(删除条数, 其图片文件名)`（供调用方删文件）。
pub fn delete(conn: &Connection, id: i64) -> Result<(usize, Vec<String>)> {
    let files = image_files_by_id(conn, id)?;
    let n = conn.execute("DELETE FROM clipboard_history WHERE id = ?1", params![id])?;
    Ok((n, files))
}

/// 清空**未收藏**的历史，返回 `(删除条数, 被删记录的图片文件名)`。
pub fn clear_unpinned(conn: &Connection) -> Result<(usize, Vec<String>)> {
    let files = collect_image_files(
        conn,
        "SELECT image_file, image_thumb FROM clipboard_history WHERE pinned = 0 AND kind = 'image'",
    )?;
    let n = conn.execute("DELETE FROM clipboard_history WHERE pinned = 0", [])?;
    Ok((n, files))
}

/// 按保留天数裁剪（收藏的永不参与；`retention_days == 0` 不裁），
/// 返回 `(删除条数, 被删条目的图片文件名)`。
pub fn prune(conn: &Connection, retention_days: u64, now: u64) -> Result<(usize, Vec<String>)> {
    if retention_days == 0 {
        return Ok((0, Vec::new()));
    }
    let cutoff = now.saturating_sub(retention_days.saturating_mul(86_400));
    // 只收集「图片」条目的文件；纯文本条目无文件，条数由 DELETE 的 affected 得到。
    let mut files = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT image_file, image_thumb FROM clipboard_history
             WHERE pinned = 0 AND created_at < ?1 AND kind = 'image'",
        )?;
        let rows = stmt.query_map(params![cutoff], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        })?;
        for row in rows {
            let (file, thumb) = row?;
            files.extend(file.into_iter().chain(thumb));
        }
    }
    let n = conn.execute(
        "DELETE FROM clipboard_history WHERE pinned = 0 AND created_at < ?1",
        params![cutoff],
    )?;
    Ok((n, files))
}

/// 按给定查询收集图片文件名（原图 + 缩略图）。
fn collect_image_files(conn: &Connection, sql: &str) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
        ))
    })?;
    for row in rows {
        let (file, thumb) = row?;
        if let Some(file) = file {
            files.push(file);
        }
        if let Some(thumb) = thumb {
            files.push(thumb);
        }
    }
    Ok(files)
}

fn image_files_by_id(conn: &Connection, id: i64) -> Result<Vec<String>> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT image_file, image_thumb FROM clipboard_history WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let mut files = Vec::new();
    if let Some((file, thumb)) = row {
        if let Some(file) = file {
            files.push(file);
        }
        if let Some(thumb) = thumb {
            files.push(thumb);
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_in_memory;

    fn text(conn: &Connection, digest: &str, body: &str, created_at: u64) -> i64 {
        upsert_text(conn, digest, body, body.len() as u64, created_at)
            .unwrap()
            .0
    }

    #[test]
    fn text_upsert_dedupes_by_digest_and_keeps_pinned() {
        let conn = init_in_memory().unwrap();
        let id = text(&conn, "t1", "hello", 100);
        assert_eq!(stats(&conn).unwrap().0, 1);
        // 同 digest 再入库：不加条、复用 id、pinned 保留
        set_pinned(&conn, id, true).unwrap();
        let (again, existed) = upsert_text(&conn, "t1", "hello", 5, 200).unwrap();
        assert!(existed);
        assert_eq!(again, id);
        assert_eq!(stats(&conn).unwrap().0, 1);
        let e = get(&conn, id).unwrap().unwrap();
        assert!(e.pinned);
        assert_eq!(e.created_at, 200, "重复复制会把它提到最新");
    }

    #[test]
    fn query_pins_first_and_filters_kind_and_substring() {
        let conn = init_in_memory().unwrap();
        let image_id = upsert_image_stub(&conn, "i1", 10).unwrap().0;
        set_image(
            &conn,
            image_id,
            &ImageMeta {
                width: 100,
                height: 50,
                file: "1.png".into(),
                thumb: "1.thumb.png".into(),
                bytes: 10,
            },
        )
        .unwrap();
        let a = text(&conn, "t2", "Hello World", 20);
        let b = text(&conn, "t3", "world 之外", 30);
        let pinned = text(&conn, "t4", "pinned", 5);
        set_pinned(&conn, pinned, true).unwrap();

        let all: Vec<u64> = query(&conn, "", ClipKindFilter::All, 100)
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(
            all,
            vec![pinned as u64, b as u64, a as u64, image_id as u64]
        );

        let matched: Vec<u64> = query(&conn, "WORLD", ClipKindFilter::All, 100)
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(
            matched,
            vec![b as u64, a as u64],
            "大小写不敏感，图片不参与"
        );

        let images: Vec<u64> = query(&conn, "", ClipKindFilter::Image, 100)
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(images, vec![image_id as u64]);

        // 关键词里的 % 不当通配符
        assert!(query(&conn, "%", ClipKindFilter::All, 100)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn limit_clamps_and_caps() {
        let conn = init_in_memory().unwrap();
        for i in 1..=10 {
            text(&conn, &format!("t{i}"), &format!("text {i}"), i);
        }
        assert_eq!(query(&conn, "", ClipKindFilter::All, 3).unwrap().len(), 3);
        assert_eq!(
            query(&conn, "", ClipKindFilter::All, 100).unwrap().len(),
            10
        );
    }

    #[test]
    fn prune_drops_expired_keeps_pinned_and_reports_files() {
        let conn = init_in_memory().unwrap();
        let now = 1_000_000_000u64;
        let day = 86_400u64;
        text(&conn, "t1", "今天", now - 10);
        text(&conn, "t2", "昨天", now - day - 10);
        let old = text(&conn, "t3", "上个月", now - 40 * day);
        // 一条老图片（带文件）与一条老但收藏的文本
        let img = upsert_image_stub(&conn, "i9", now - 40 * day).unwrap().0;
        set_image(
            &conn,
            img,
            &ImageMeta {
                width: 4,
                height: 4,
                file: "9.png".into(),
                thumb: "9.thumb.png".into(),
                bytes: 128,
            },
        )
        .unwrap();
        let keep = text(&conn, "t4", "收藏的旧文本", now - 40 * day);
        set_pinned(&conn, keep, true).unwrap();

        let (n, removed) = prune(&conn, 30, now).unwrap();
        assert_eq!(n, 2, "裁掉老图片与老文本各一条");
        assert_eq!(
            removed,
            vec!["9.png".to_string(), "9.thumb.png".to_string()]
        );
        let ids: Vec<u64> = all(&conn).unwrap().iter().map(|e| e.id).collect();
        assert!(ids.contains(&(keep as u64)));
        assert!(!ids.contains(&(old as u64)));
        // 0 = 永久保留
        assert_eq!(prune(&conn, 0, now).unwrap().0, 0);
    }

    #[test]
    fn clear_keeps_pinned_and_delete_reports_files() {
        let conn = init_in_memory().unwrap();
        let a = text(&conn, "t1", "a", 1);
        let keep = text(&conn, "t2", "keep", 2);
        set_pinned(&conn, keep, true).unwrap();
        let (n, files) = clear_unpinned(&conn).unwrap();
        assert_eq!(n, 1);
        assert!(files.is_empty());
        let left: Vec<u64> = all(&conn).unwrap().iter().map(|e| e.id).collect();
        assert_eq!(left, vec![keep as u64], "收藏的还在，其余被清");
        assert!(get(&conn, a).unwrap().is_none());

        let img = upsert_image_stub(&conn, "i1", 3).unwrap().0;
        set_image(
            &conn,
            img,
            &ImageMeta {
                width: 1,
                height: 1,
                file: "1.png".into(),
                thumb: "1.thumb.png".into(),
                bytes: 1,
            },
        )
        .unwrap();
        let (n, names) = delete(&conn, img).unwrap();
        assert_eq!(n, 1);
        assert_eq!(names, vec!["1.png".to_string(), "1.thumb.png".to_string()]);
    }

    #[test]
    fn update_text_recomputes_digest_without_unique_conflict() {
        let conn = init_in_memory().unwrap();
        let a = text(&conn, "t1", "hello", 1);
        let b = text(&conn, "t2", "other", 2);
        // 把 b 改成与 a 同内容：旧 a 被删，b 成为唯一
        update_text(&conn, b, "hello", "t1", 5).unwrap();
        assert_eq!(stats(&conn).unwrap().0, 1);
        assert!(get(&conn, a).unwrap().is_none());
        assert_eq!(
            get(&conn, b).unwrap().unwrap().text.as_deref(),
            Some("hello")
        );
    }
}

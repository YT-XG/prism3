//! 工作记录（完成台账）的数据访问：`work_projects` + `work_logs`。
//!
//! 本层只做 SQL 与行映射，不做业务校验（标题 / 工时 / kind 白名单在
//! `commands.rs` 里统一把关）；时间一律秒级 unix 时间戳（`super::now()`）。

use rusqlite::{params, Connection, OptionalExtension, Result};

use crate::models::{WorkLog, WorkProject};

/// 项目查询列顺序，必须与 `row_to_project` 一一对应。
const PROJECT_COLS: &str = "id, name, archived_at, created_at";

/// 日志查询列顺序，必须与 `row_to_log` 一一对应（项目名由 JOIN 带出）。
const LOG_COLS: &str = "l.id, l.title, l.detail, l.project_id, COALESCE(p.name, ''), \
     l.kind, l.hours, l.finished_at, l.source, l.created_at, l.updated_at";

const LOG_FROM: &str = "FROM work_logs l LEFT JOIN work_projects p ON p.id = l.project_id";

fn row_to_project(row: &rusqlite::Row) -> Result<WorkProject> {
    Ok(WorkProject {
        id: row.get(0)?,
        name: row.get(1)?,
        archived_at: row.get::<_, Option<i64>>(2)?.map(|v| v as u64),
        created_at: row.get::<_, i64>(3)? as u64,
    })
}

fn row_to_log(row: &rusqlite::Row) -> Result<WorkLog> {
    Ok(WorkLog {
        id: row.get(0)?,
        title: row.get(1)?,
        detail: row.get(2)?,
        project_id: row.get(3)?,
        project_name: row.get(4)?,
        kind: row.get(5)?,
        hours: row.get(6)?,
        finished_at: row.get::<_, i64>(7)? as u64,
        source: row.get(8)?,
        created_at: row.get::<_, i64>(9)? as u64,
        updated_at: row.get::<_, i64>(10)? as u64,
    })
}

// ------------------------------ 项目 ------------------------------

/// 未归档的项目，按创建顺序（前端按此顺序渲染下拉与侧栏）。
pub fn list_projects(conn: &Connection) -> Result<Vec<WorkProject>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PROJECT_COLS} FROM work_projects
         WHERE archived_at IS NULL ORDER BY created_at ASC, id ASC"
    ))?;
    let rows = stmt.query_map([], row_to_project)?;
    rows.collect()
}

pub fn get_project(conn: &Connection, id: i64) -> Result<WorkProject> {
    conn.query_row(
        &format!("SELECT {PROJECT_COLS} FROM work_projects WHERE id = ?1"),
        params![id],
        row_to_project,
    )
}

/// 项目名是否已被占用（大小写不敏感：SQLite 的 `=` 对 TEXT 默认区分大小写，
/// 这里用 `COLLATE NOCASE` 与用户直觉对齐）。
pub fn project_name_taken(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM work_projects WHERE name = ?1 COLLATE NOCASE",
        params![name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

pub fn project_exists(conn: &Connection, id: i64) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM work_projects WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

pub fn create_project(conn: &Connection, name: &str) -> Result<WorkProject> {
    let ts = super::now();
    conn.execute(
        "INSERT INTO work_projects (name, created_at) VALUES (?1, ?2)",
        params![name, ts as i64],
    )?;
    get_project(conn, conn.last_insert_rowid())
}

pub fn rename_project(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let affected = conn.execute(
        "UPDATE work_projects SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    if affected == 0 {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "NOT_FOUND: 项目 {id} 不存在"
        )));
    }
    Ok(())
}

/// 删除项目：日志保留，`project_id` 由外键 `ON DELETE SET NULL` 置空。
pub fn delete_project(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM work_projects WHERE id = ?1", params![id])?;
    Ok(())
}

// ------------------------------ 完成记录 ------------------------------

pub fn get(conn: &Connection, id: i64) -> Result<WorkLog> {
    conn.query_row(
        &format!("SELECT {LOG_COLS} {LOG_FROM} WHERE l.id = ?1"),
        params![id],
        row_to_log,
    )
}

/// `[from, to]` 闭区间内（按 `finished_at`）的完成记录，最新在前。
pub fn list_range(conn: &Connection, from: u64, to: u64) -> Result<Vec<WorkLog>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {LOG_COLS} {LOG_FROM}
         WHERE l.finished_at >= ?1 AND l.finished_at <= ?2
         ORDER BY l.finished_at DESC, l.id DESC"
    ))?;
    let rows = stmt.query_map(params![from as i64, to as i64], row_to_log)?;
    rows.collect()
}

#[allow(clippy::too_many_arguments)]
pub fn create(
    conn: &Connection,
    title: &str,
    detail: &str,
    project_id: Option<i64>,
    kind: &str,
    hours: f64,
    finished_at: u64,
    source: &str,
) -> Result<WorkLog> {
    if let Some(pid) = project_id {
        if !project_exists(conn, pid)? {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "NOT_FOUND: 项目 {pid} 不存在"
            )));
        }
    }
    let ts = super::now();
    conn.execute(
        "INSERT INTO work_logs
           (title, detail, project_id, kind, hours, finished_at, source, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            title,
            detail,
            project_id,
            kind,
            hours,
            finished_at as i64,
            source,
            ts as i64
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    conn: &Connection,
    id: i64,
    title: &str,
    detail: &str,
    project_id: Option<i64>,
    kind: &str,
    hours: f64,
    finished_at: u64,
) -> Result<WorkLog> {
    if let Some(pid) = project_id {
        if !project_exists(conn, pid)? {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "NOT_FOUND: 项目 {pid} 不存在"
            )));
        }
    }
    let affected = conn.execute(
        "UPDATE work_logs
         SET title = ?1, detail = ?2, project_id = ?3, kind = ?4,
             hours = ?5, finished_at = ?6, updated_at = ?7
         WHERE id = ?8",
        params![
            title,
            detail,
            project_id,
            kind,
            hours,
            finished_at as i64,
            super::now() as i64,
            id
        ],
    )?;
    if affected == 0 {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "NOT_FOUND: 工作记录 {id} 不存在"
        )));
    }
    get(conn, id)
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM work_logs WHERE id = ?1", params![id])?;
    Ok(())
}

/// 单条记录是否存在（命令层用于给出更友好的错误；不额外查库时可直接依赖 update 的影响行数）。
#[allow(dead_code)]
pub fn exists(conn: &Connection, id: i64) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row("SELECT id FROM work_logs WHERE id = ?1", params![id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(found.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn conn() -> Connection {
        db::init_in_memory().unwrap()
    }

    fn sample(conn: &Connection, title: &str, finished_at: u64) -> WorkLog {
        create(conn, title, "", None, "dev", 1.5, finished_at, "manual").unwrap()
    }

    #[test]
    fn create_list_update_delete_log() {
        let conn = conn();
        let log = sample(&conn, "修复登录超时", 1_700_000_000);

        let list = list_range(&conn, 1_700_000_000, 1_700_000_000).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "修复登录超时");
        assert_eq!(list[0].hours, 1.5);
        assert_eq!(list[0].source, "manual");

        let updated = update(
            &conn,
            log.id,
            "修复登录超时（补充）",
            "补充说明",
            None,
            "bug",
            2.0,
            1_700_000_100,
        )
        .unwrap();
        assert_eq!(updated.title, "修复登录超时（补充）");
        assert_eq!(updated.kind, "bug");
        assert_eq!(updated.hours, 2.0);
        assert_eq!(updated.finished_at, 1_700_000_100);

        delete(&conn, log.id).unwrap();
        assert!(list_range(&conn, 0, u64::MAX).unwrap().is_empty());
        assert!(!exists(&conn, log.id).unwrap());
    }

    #[test]
    fn range_filter_is_inclusive_and_sorted_desc() {
        let conn = conn();
        sample(&conn, "甲", 100);
        sample(&conn, "乙", 200);
        sample(&conn, "丙", 300);

        let middle = list_range(&conn, 100, 200).unwrap();
        assert_eq!(
            middle.iter().map(|l| l.title.as_str()).collect::<Vec<_>>(),
            vec!["乙", "甲"]
        );
        assert!(list_range(&conn, 201, 299).unwrap().is_empty());
    }

    #[test]
    fn projects_are_unique_case_insensitively_and_delete_clears_attachment() {
        let conn = conn();
        let project = create_project(&conn, "Prism3").unwrap();
        assert!(project_name_taken(&conn, "prism3").unwrap());

        let log = create(
            &conn,
            "接入工作记录",
            "",
            Some(project.id),
            "dev",
            1.0,
            1_700_000_000,
            "manual",
        )
        .unwrap();
        assert_eq!(log.project_name, "Prism3");

        delete_project(&conn, project.id).unwrap();
        let after = get(&conn, log.id).unwrap();
        assert_eq!(after.project_id, None);
        assert_eq!(after.project_name, "");
        assert!(list_projects(&conn).unwrap().is_empty());
    }

    #[test]
    fn unknown_project_is_rejected() {
        let conn = conn();
        let err = create(&conn, "x", "", Some(999), "dev", 0.0, 1, "manual").unwrap_err();
        assert!(err.to_string().contains("NOT_FOUND"));
    }
}

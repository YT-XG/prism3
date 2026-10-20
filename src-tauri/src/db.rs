//! 统一存储层：一个 SQLite 库承载速记（笔记正文 + 元数据）与剪贴板历史。
//!
//! 库文件位于 `<app_config_dir>\prism3.db`（`%APPDATA%\com.prism3.desktop`，
//! 与既有的 `config.json` / 剪贴板图片目录同目录），WAL 模式下会一并出现
//! `prism3.db-wal` / `prism3.db-shm`，属正常。
//!
//! ## 锁策略（后续所有读写都要遵守，写在这里当约定）
//! 1. **单连接 + `Mutex<Connection>`**：`rusqlite::Connection` 是 `Send` 非 `Sync`，
//!    用 `Mutex` 包裹后可直接交给 Tauri 托管；`DbState` 就是这一层。
//! 2. **绝不跨 `await` 持锁**：async 命令先读成 owned 快照，`drop(conn)` 后再做文件 IO。
//! 3. 临界区只做一两句 SQL；仅 `note::update_with_link_fixup` 与文件夹删除/排序用多语句
//!    事务（`unchecked_transaction()`，毫秒级）。
//! 4. 剪贴板采集线程（400ms 轮询、仅序列号变化才写一行）与笔记命令共用这一把锁；
//!    同进程单连接不会 `SQLITE_BUSY`，`busy_timeout` 只是给将来的短连接模式预留。
//! 5. 全量拉正文 / 导出 / GC 这类重活走 `#[tauri::command(async)]`，避免卡主线程。
//! 6. Fallback（首版不做，留档）：(a) 每命令开短连接（WAL 下多连接安全）；(b) 所有写
//!    集中到一个 DB 线程 + mpsc。

use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

/// 逻辑 schema 版本。本库是新建的，首版即 1；无历史补列逻辑。
const SCHEMA_VERSION: u32 = 1;

/// 由 Tauri 托管的数据库状态。
///
/// `notes_revision` 是笔记侧的变更计数：任何 note/folder/tag 写操作后 +1，随
/// `notes:changed` 事件广播（载荷只带 revision，不带内容——同 `ClipState` 的教训）。
pub struct DbState {
    pub conn: Mutex<Connection>,
    pub notes_revision: AtomicU64,
}

impl DbState {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
            notes_revision: AtomicU64::new(0),
        }
    }

    /// 取锁（中毒时沿用内部值，与 `config.rs` / `clipboard.rs` 同口径）。
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 当前时间（秒级 unix 时间戳）。与 `clipboard.rs::now_secs()` 同一口径，
/// 前端 `formatTimeAgo(secs)` 直接可用。
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 打开（或新建）库并应用 PRAGMA + 迁移。
pub fn init(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    apply_pragmas(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// 测试用内存库。
#[cfg(test)]
pub fn init_in_memory() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    apply_pragmas(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// PRAGMA 必须每个连接都设（尤其 `foreign_keys`，否则 `ON DELETE CASCADE/SET NULL`
/// 会静默失效）。
fn apply_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    Ok(())
}

#[cfg(test)]
fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

/// 建表 + 索引 + 内置种子。幂等（全部 IF NOT EXISTS / 幂等 INSERT）。
///
/// 表结构逐字取自 x-hub `db.rs` 的速记段，仅把时间列由 TEXT 改为 INTEGER 秒级时间戳。
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS note_folders (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          parent_id INTEGER REFERENCES note_folders(id) ON DELETE SET NULL,
          sort_order INTEGER NOT NULL DEFAULT 0,
          builtin INTEGER NOT NULL DEFAULT 0,
          created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS notes (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          title TEXT NOT NULL DEFAULT '',
          content TEXT NOT NULL DEFAULT '',
          folder_id INTEGER REFERENCES note_folders(id) ON DELETE SET NULL,
          source_url TEXT NOT NULL DEFAULT '',
          icon TEXT,
          deleted_at INTEGER,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );

        -- 双链轻量版：引用键 = 标题（不落块 id）。to_note_id 解析到已存在标题时补上；
        -- 引用了不存在的标题保留为「未链接提及」（to_note_id NULL），不算错。
        CREATE TABLE IF NOT EXISTS note_links (
          from_note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
          to_note_id INTEGER REFERENCES notes(id) ON DELETE SET NULL,
          to_title TEXT NOT NULL,
          PRIMARY KEY (from_note_id, to_title)
        );

        CREATE TABLE IF NOT EXISTS tags (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL UNIQUE,
          builtin INTEGER NOT NULL DEFAULT 0,
          created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS note_tags (
          note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
          tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
          PRIMARY KEY (note_id, tag_id)
        );

        -- 剪贴板历史：与既有 JSON 版本同字段，图片只存文件名（原名 / 缩略图名），
        -- 文件本体仍在 clipboard-images 目录下。
        CREATE TABLE IF NOT EXISTS clipboard_history (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          kind TEXT NOT NULL DEFAULT 'text' CHECK (kind IN ('text','image')),
          text TEXT,
          digest TEXT NOT NULL,
          size INTEGER NOT NULL DEFAULT 0,
          pinned INTEGER NOT NULL DEFAULT 0,
          created_at INTEGER NOT NULL,
          image_file TEXT,
          image_thumb TEXT,
          image_width INTEGER,
          image_height INTEGER,
          image_bytes INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_notes_updated     ON notes(updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_notes_folder      ON notes(folder_id);
        CREATE INDEX IF NOT EXISTS idx_notes_deleted     ON notes(deleted_at);
        CREATE INDEX IF NOT EXISTS idx_notes_source_url  ON notes(source_url);
        CREATE INDEX IF NOT EXISTS idx_notes_title       ON notes(title);
        CREATE INDEX IF NOT EXISTS idx_note_tags_tag     ON note_tags(tag_id);
        CREATE INDEX IF NOT EXISTS idx_note_links_to     ON note_links(to_note_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_clip_digest ON clipboard_history(digest);
        CREATE INDEX IF NOT EXISTS idx_clip_pinned_created ON clipboard_history(pinned, created_at DESC);
        ",
    )?;

    // 清理历史遗留的内置「剪藏」文件夹 / 标签（幂等、**不再新建**）。
    //
    // prism3 没有「网页剪藏」功能，这个内置项是从 x-hub 照搬 schema 时一起带过来的：
    // 既解释不清用途，又因为 `builtin = 1` 受保护而改不动、删不掉。这里直接清掉——
    // 删文件夹会把其中的笔记与子文件夹**上移到树根**（外键 `ON DELETE SET NULL`），
    // 删标签只解关联（`note_tags` 级联），都不会丢笔记。新建的库本就没有这些行。
    //
    // `builtin` 列与 `repo::{note_folder,tag}` 里的内置守卫**保留但不再命中**：老库里
    // 该列还在，留着避免做一次有风险的 `ALTER TABLE ... DROP COLUMN`（本表是外键的父表）。
    let purged_folders = conn.execute("DELETE FROM note_folders WHERE builtin = 1", [])?;
    if purged_folders > 0 {
        log::info!(
            "已清理历史遗留的内置「剪藏」文件夹 {purged_folders} 个（其中的笔记已上移到树根）"
        );
    }
    let purged_tags = conn.execute("DELETE FROM tags WHERE builtin = 1", [])?;
    if purged_tags > 0 {
        log::info!("已清理历史遗留的内置「剪藏」标签 {purged_tags} 个");
    }

    // 记录 schema 版本（仅作留痕，本库无历史补列逻辑）。
    conn.pragma_update(None, "user_version", SCHEMA_VERSION as i64)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn fresh_db_creates_all_tables_without_builtin_items() {
        let conn = init_in_memory().unwrap();
        for table in [
            "note_folders",
            "notes",
            "note_links",
            "tags",
            "note_tags",
            "clipboard_history",
        ] {
            assert!(table_exists(&conn, table), "缺表 {table}");
        }
        // 全新库不再新建任何内置（builtin）文件夹 / 标签

        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM note_folders WHERE builtin = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "不应再新建内置文件夹");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags WHERE builtin = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 0, "不应再新建内置标签");
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = init_in_memory().unwrap();
        // 连跑两次：不得重复建表/重复种子/报错
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let folders: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM note_folders WHERE builtin = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(folders, 0);
        let tags: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags WHERE builtin = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(tags, 0);
    }

    #[test]
    fn legacy_builtin_items_are_cleaned_and_their_notes_move_to_root() {
        // 模拟老库：内置「剪藏」文件夹（含一条笔记 + 一个子文件夹）与内置标签。
        let conn = init_in_memory().unwrap();
        let now = now_secs() as i64;
        conn.execute(
            "INSERT INTO note_folders (name, parent_id, sort_order, builtin, created_at)
             VALUES ('剪藏', NULL, 0, 1, ?1)",
            params![now],
        )
        .unwrap();
        let builtin = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO note_folders (name, parent_id, sort_order, builtin, created_at)
             VALUES ('子目录', ?1, 0, 0, ?2)",
            params![builtin, now],
        )
        .unwrap();
        let child = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO notes (title, content, folder_id, created_at, updated_at)
             VALUES ('夹内笔记', '', ?1, ?2, ?2)",
            params![builtin, now],
        )
        .unwrap();
        let note_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO tags (name, builtin, created_at) VALUES ('剪藏', 1, ?1)",
            params![now],
        )
        .unwrap();
        let tag_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO note_tags (note_id, tag_id) VALUES (?1, ?2)",
            params![note_id, tag_id],
        )
        .unwrap();

        migrate(&conn).unwrap();

        // 内置项被清掉
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM note_folders WHERE builtin = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags WHERE builtin = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 0);
        // 笔记与子文件夹都上移到树根（不丢数据）
        let folder_id: Option<i64> = conn
            .query_row(
                "SELECT folder_id FROM notes WHERE id = ?1",
                params![note_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(folder_id, None);
        let parent_id: Option<i64> = conn
            .query_row(
                "SELECT parent_id FROM note_folders WHERE id = ?1",
                params![child],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(parent_id, None);
        // 标签关联被级联清掉
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM note_tags WHERE tag_id = ?1",
                params![tag_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn hard_delete_cascades_note_tags_and_unlinks_notes() {
        let conn = init_in_memory().unwrap();
        // 笔记 A、B；B 引用 A；B 带一个标签
        let now = now_secs() as i64;
        conn.execute(
            "INSERT INTO notes (title, content, created_at, updated_at) VALUES ('甲', '', ?1, ?1)",
            params![now],
        )
        .unwrap();
        let a = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO notes (title, content, created_at, updated_at) VALUES ('乙', '见 [[甲]]', ?1, ?1)",
            params![now],
        )
        .unwrap();
        let b = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO tags (name, created_at) VALUES ('工作', ?1)",
            params![now],
        )
        .unwrap();
        let tag = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO note_tags (note_id, tag_id) VALUES (?1, ?2)",
            params![b, tag],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO note_links (from_note_id, to_note_id, to_title) VALUES (?1, ?2, '甲')",
            params![b, a],
        )
        .unwrap();

        // 硬删 A：B 的入链因 ON DELETE SET NULL 降级为「未链接提及」，note_tags 不受影响
        conn.execute("DELETE FROM notes WHERE id = ?1", params![a])
            .unwrap();
        let link_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM note_links WHERE from_note_id = ?1",
                params![b],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(link_count, 1, "入链应保留为未链接提及");
        let to_id: Option<i64> = conn
            .query_row(
                "SELECT to_note_id FROM note_links WHERE from_note_id = ?1",
                params![b],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(to_id, None);

        // 硬删 B：出链随 CASCADE 消失，note_tags 亦级联清除
        conn.execute("DELETE FROM notes WHERE id = ?1", params![b])
            .unwrap();
        let links: i64 = conn
            .query_row("SELECT COUNT(*) FROM note_links", [], |r| r.get(0))
            .unwrap();
        assert_eq!(links, 0, "出链应随 CASCADE 清除");
        let nt: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM note_tags WHERE note_id = ?1",
                params![b],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(nt, 0, "note_tags 应随 CASCADE 清除");
    }
}

//! 数据访问层：每个领域一个子模块，函数签名统一 `(&Connection, ...)`，不做长事务。
//! 时间一律用 `now_secs()`（秒级 unix 时间戳，见 `crate::db`）。

pub mod clipboard;
pub mod note;
pub mod note_folder;
pub mod note_link;
pub mod tag;

/// 秒级时间戳别名（与 `crate::db::now_secs` 同口径）。
pub fn now() -> u64 {
    crate::db::now_secs()
}

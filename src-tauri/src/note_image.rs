//! 笔记图片：落盘命名、URL 口径、协议侧校验、孤儿扫描的共用助手。
//!
//! 图片本体是 `<app_config_dir>\notes\images\<16位hex>.<ext>`（内容哈希命名，同名即同内容），
//! 正文里以自定义协议 URL `http://prism3-note.localhost/<name>` 引用；
//! 协议处理器与孤儿 GC 都走这里同一套「合法文件名」口径与同一段哈希实现。

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

/// 自定义图片协议的 URL 前缀（Windows 上 Tauri 把自定义 scheme 映射成 `<scheme>.localhost`）。
pub const NOTE_URL_PREFIX: &str = "http://prism3-note.localhost/";
/// 允许的图片扩展名（小写）。
pub const NOTE_IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];
/// 单张图片上限（10 MiB）。
pub const NOTE_IMAGE_MAX_BYTES: usize = 10 * 1024 * 1024;

/// 图片目录。
pub fn image_dir(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(app.path().app_config_dir()?.join("notes").join("images"))
}

/// 文件名 → 正文应插入的 URL。
pub fn note_image_url(name: &str) -> String {
    format!("{NOTE_URL_PREFIX}{name}")
}

/// 合法性口径：16 位十六进制哈希 + 白名单扩展。协议处理器与 GC 共用，杜绝路径穿越。
pub fn valid_note_image_name(name: &str) -> bool {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    stem.len() == 16
        && stem.bytes().all(|b| b.is_ascii_hexdigit())
        && NOTE_IMAGE_EXTS.contains(&ext.to_lowercase().as_str())
}

/// FNV-1a（64 位）：只需要「同内容同名」，不要求密码学强度（与剪贴板摘要同一实现）。
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn normalize_ext(ext: &str) -> AppResult<String> {
    let e = ext.trim().trim_start_matches('.').to_lowercase();
    if NOTE_IMAGE_EXTS.contains(&e.as_str()) {
        Ok(e)
    } else {
        Err(AppError::Other(format!("不支持的图片格式：{ext}")))
    }
}

/// 保存图片字节并返回正文 URL。内容哈希命名 ⇒ 同一张图重复上传不会写两份。
pub fn save(app: &AppHandle, bytes: &[u8], ext: &str) -> AppResult<String> {
    if bytes.is_empty() {
        return Err(AppError::Other("图片内容为空".into()));
    }
    if bytes.len() > NOTE_IMAGE_MAX_BYTES {
        return Err(AppError::Other(format!(
            "图片超过 {} MB，请压缩后再试",
            NOTE_IMAGE_MAX_BYTES / 1024 / 1024
        )));
    }
    let ext = normalize_ext(ext)?;
    let name = format!("{:016x}.{ext}", fnv1a(bytes));
    let dir = image_dir(app)?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(&name);
    if !path.is_file() {
        std::fs::write(&path, bytes)?;
    }
    Ok(note_image_url(&name))
}

/// 从正文里抽取被引用的图片哈希主干（GC 差集用）。
pub fn extract_image_stems(content: &str) -> Vec<String> {
    let mut stems = Vec::new();
    let mut rest = content;
    while let Some(pos) = rest.find(NOTE_URL_PREFIX) {
        let tail = &rest[pos + NOTE_URL_PREFIX.len()..];
        let name: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
            .collect();
        if let Some(stem) = name.split('.').next() {
            if stem.len() == 16 && stem.bytes().all(|b| b.is_ascii_hexdigit()) {
                stems.push(stem.to_string());
            }
        }
        rest = tail;
    }
    stems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_name_requires_hash_and_whitelisted_ext() {
        assert!(valid_note_image_name("0123456789abcdef.png"));
        assert!(valid_note_image_name("0123456789ABCDEF.JPG"));
        assert!(!valid_note_image_name("notahash.png"));
        assert!(!valid_note_image_name("0123456789abcdef.exe"));
        assert!(!valid_note_image_name("0123456789abcde.png"));
        assert!(!valid_note_image_name("../secret.png"));
        assert!(!valid_note_image_name("0123456789abcdef"));
    }

    #[test]
    fn extract_stems_reads_only_hashed_urls() {
        let url = note_image_url("0123456789abcdef.png");
        let content = format!("前 ![x]({url}) 后 ![y](assets/notahash.png)");
        assert_eq!(
            extract_image_stems(&content),
            vec!["0123456789abcdef".to_string()]
        );
    }

    #[test]
    fn normalize_ext_rejects_unknown() {
        assert_eq!(normalize_ext(".PNG").unwrap(), "png");
        assert_eq!(normalize_ext("jpeg").unwrap(), "jpeg");
        assert!(normalize_ext("svg").is_err());
    }
}

/// 应用统一错误类型。
///
/// 命令返回值中的错误需要能被序列化传给前端，因此这里手写 `Serialize`，
/// 统一序列化为错误消息字符串。
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("序列化错误: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Tauri 错误: {0}")]
    Tauri(#[from] tauri::Error),
    #[error("图片错误: {0}")]
    Image(#[from] image::ImageError),
    #[error("剪贴板错误: {0}")]
    Clipboard(#[from] arboard::Error),
    /// 兜底变体：用于放置不便归类的临时错误，避免为每个场景新增变体。
    #[error("{0}")]
    Other(String),
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

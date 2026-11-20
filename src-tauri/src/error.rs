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
    #[error("数据库错误: {0}")]
    Db(#[from] rusqlite::Error),
    /// 协议不符：连上了设备，但它的 GATT 里没有本平台的私有服务 / 特征。
    ///
    /// 与普通 IO 失败必须分开：这类失败**重试没有意义**（同一台设备不会突然长出特征），
    /// 会话线程据此停掉自动连接的看门狗，避免把手机 / 别的品牌耳机当成「耳机没取出」
    /// 每轮都连一遍。
    #[error("协议不符：{0}")]
    ProtocolMismatch(String),
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

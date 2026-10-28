//! 统一错误类型。
//!
//! 设计要点：所有跨越 Tauri IPC 边界的错误都必须实现 `Serialize`，
//! 且前端只看到一句人话，不暴露内部路径与栈信息。

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("非法或越界的路径：{0}")]
    PathEscape(String),

    #[error("文件不存在：{0}")]
    NotFound(String),

    #[error("该文件不是文本，无法内置预览")]
    NotText,

    #[error("扫描已在进行中")]
    ScanBusy,

    /// 监听起不来不是致命错误：索引仍可手动全量扫描，所以单独一个变体，
    /// 让调用方能降级而不是整体启动失败。
    #[error("文件监听不可用：{0}")]
    Watch(String),

    #[error("未实现：{0}")]
    Unimplemented(&'static str),

    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),

    #[error("数据库错误：{0}")]
    Db(#[from] rusqlite::Error),

    #[error("启动外部程序失败：{0}")]
    Launch(String),

    #[error("运行时清单有误：{0}")]
    Manifest(String),

    #[error("运行时组件未就位：{0}")]
    ComponentMissing(String),

    #[error("执行超时（{0} ms）")]
    Timeout(u64),

    #[error("内部状态锁被污染")]
    Poisoned,
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

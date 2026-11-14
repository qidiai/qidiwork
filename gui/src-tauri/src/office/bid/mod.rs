//! 标书导出 P0a(T1)：数据模型与 docx 底座。
//!
//! T1a 交付：`BidError`/`BidResult`（本文件）+ `model`（数据模型）+ `docx`（docx 包读写）。
//! T2 追加 `template`/`fill`/`verify`，T3 追加 `paths`（本文件只允许追加 `pub mod` 行）。

#[derive(Debug)]
pub struct BidError(pub String);

impl BidError {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for BidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BidError {}

impl From<std::io::Error> for BidError {
    fn from(e: std::io::Error) -> Self {
        Self(format!("IO 失败: {e}"))
    }
}

pub type BidResult<T> = Result<T, BidError>;

pub mod model;
pub mod docx;
pub mod template_gen;
pub mod template;
pub mod verify;
pub mod fill;
pub mod paths;

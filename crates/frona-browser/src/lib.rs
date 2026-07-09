mod aria;
mod connection;
mod error;
mod keymap;
mod markdown;
mod ops;
mod types;
mod url;

pub use connection::BrowserConnection;
pub use error::Error;
pub use types::{
    ElementTarget, ExtractFormat, Link, MarkdownPage, PageInfo, ScreenshotResult, Snapshot,
    TabInfo,
};

pub type Result<T> = std::result::Result<T, Error>;

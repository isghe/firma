pub mod common;
pub mod offline;
pub mod online;

#[cfg(target_os = "android")]
mod android;

pub use common::cmd::*;
pub use common::error::*;
pub use common::file::*;
pub use common::json::*;
pub use common::*;
pub use online::Wallet;

pub type Result<R> = std::result::Result<R, Error>;
pub type PSBT = bitcoin::util::psbt::PartiallySignedTransaction;

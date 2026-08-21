//! Soulseek backend adapter.
//!
//! Thin adapter over the [`rustsoseek`] native client. The client owns the
//! wire protocol, login, search, download, and distributed search; this crate
//! only maps those into agpeer's shared [`agpeer_common`] backend traits.

pub mod error;
pub mod native_backend;

pub use native_backend::NativeSoulseekBackend;

pub use rustsoseek::{NativeClient, NativeConfig};

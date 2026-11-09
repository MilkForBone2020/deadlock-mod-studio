//! Core file logic for Deadlock Mod Studio.
//!
//! Nothing in here talks to the UI. Every function works on plain paths and
//! data so it can be unit tested without Tauri or a real Deadlock install.

pub mod audio;
pub mod blender;
pub mod catalog;
pub mod csdk;
pub mod dmx;
pub mod gameinfo;
pub mod glb;
pub mod kv3;
pub mod kv3text;
pub mod library;
pub mod material;
pub mod meshswap;
pub mod model;
pub mod steam;
pub mod sync;
pub mod tools;
pub mod vpk;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Msg(String),
    #[error("File error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Could not read the library file: {0}")]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub fn msg(s: impl Into<String>) -> Self {
        Error::Msg(s.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Wrap an io::Error with the path it happened on, so messages are useful.
pub(crate) trait IoCtx<T> {
    fn at(self, path: &std::path::Path) -> Result<T>;
}

impl<T> IoCtx<T> for std::io::Result<T> {
    fn at(self, path: &std::path::Path) -> Result<T> {
        self.map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })
    }
}

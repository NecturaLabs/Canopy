//! # canopy-core
//!
//! The Git backend of Canopy. It knows nothing about the user interface: it turns a repository
//! into plain data ([`RepoSnapshot`], [`CommitDetails`], [`FileDiff`]) and performs operations on
//! it. Reads and local mutations use libgit2 through the `git2` crate; network and
//! history-rewriting operations run the `git` executable (see [`cli`]) so that credential
//! helpers, SSH configuration and hooks behave exactly as they do in a terminal.
//!
//! Every function here may block on disk or network I/O. Callers with a user interface must run
//! them off the UI thread.

pub mod cli;
pub mod diff;
mod error;
pub mod graph;
mod lines;
pub mod model;
pub mod ops;
pub mod repo;
pub mod status;

pub use diff::{DiffLine, DiffSource, FileDiff, Hunk, HunkKey, LineKind};
pub use error::{Error, Result};
pub use git2::Oid;
pub use graph::{GraphRow, LineEnd, Segment};
pub use model::*;
pub use repo::Repo;

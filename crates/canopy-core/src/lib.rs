//! # canopy-core
//!
//! The Git backend of Canopy. It knows nothing about the user interface: it turns a repository
//! into plain data ([`RepoSnapshot`], [`CommitDetails`], [`FileDiff`]) and performs operations on
//! it. Reads use libgit2 through the `git2` crate; every mutation and all network traffic run the
//! `git` executable (see [`cli`]) so that credential helpers, SSH configuration, hooks and the
//! index behave exactly as they do in a terminal. The one plain file write is saving a
//! hand-edited conflict resolution ([`conflict`]), which git then stages.
//!
//! Every function here may block on disk or network I/O. Callers with a user interface must run
//! them off the UI thread.

pub mod cli;
pub mod conflict;
pub mod diff;
mod error;
pub mod flow;
pub mod forge;
pub mod graph;
pub mod ignore;
pub mod lfs;
mod lines;
mod merged;
pub mod model;
pub mod ops;
pub mod partial;
pub mod paths;
pub mod repo;
pub mod status;
pub mod submodule;
pub mod update;

pub use conflict::{
    Choice, ConflictFile, ConflictKind, ConflictRegion, Segment, Side, has_conflict_markers,
};
pub use diff::{DiffLine, DiffSource, FileDiff, Hunk, HunkKey, LineKind, Whitespace};
pub use error::{Error, Result};
pub use flow::{FlowConfig, FlowFinish, FlowKind};
pub use forge::{Forge, ForgeKind};
pub use git2::Oid;
// `Segment` at the crate root is the conflict-file segment; the graph's is `GraphSegment`.
pub use graph::{GraphRow, LineEnd, Segment as GraphSegment};
pub use ignore::{
    IgnoreDestination, IgnoreMatch, IgnoreOutcome, IgnorePreview, IgnoreRule, IgnoreTarget,
};
pub use lfs::{LfsFile, LfsPointer};
pub use merged::{MergedBranch, MergedReason};
pub use model::*;
pub use ops::{HunkAction, MergeMode, PullStrategy, ResetMode};
pub use partial::MissingObjects;
pub use repo::Repo;
pub use submodule::{SubmoduleChange, SubmoduleInfo, SubmoduleState};

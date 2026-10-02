//! Pull requests (GitHub) and merge requests (GitLab) for Canopy, over the providers' REST APIs.
//!
//! This crate is Canopy's only network client besides the `git` executable. Its scope is
//! deliberately narrow:
//!
//! - It talks only to the REST API of the host the repository's own remote points at:
//!   `api.github.com` for github.com, `https://<host>/api/v4` for gitlab.com and self-hosted
//!   GitLab. The host comes from a [`canopy_core::forge::Forge`] recognised from a remote URL;
//!   nothing else is contacted (no telemetry, avatars or third-party services). The one
//!   exception is Canopy's own update manifest ([`updates`]), fetched anonymously from the public
//!   `NecturaLabs/app-updates` repository.
//! - It authenticates only with a token the user supplied: an environment variable
//!   ([`token_from_env_for`], which offers `GH_TOKEN` only to github.com and `GITLAB_TOKEN` only
//!   to gitlab.com, `GITLAB_HOST` or a host the user listed), git's own credential helpers
//!   ([`token_from_git_credentials`]) or a token entered in Canopy. The token travels only in an
//!   `Authorization: Bearer` header, never in a URL, a log line or any `Debug` output.
//! - Every call is blocking (no async runtime), with a 10 s connect and 30 s total timeout, and
//!   is meant to run on a background worker thread. TLS is rustls with the operating system's
//!   certificate verifier, so a self-hosted GitLab behind a corporate CA works; the standard
//!   proxy environment variables are honoured.
//!
//! Errors carry the server's own message ([`Error`]).

mod client;
mod error;
mod github;
mod gitlab;
mod http;
mod model;
mod token;
pub mod updates;

pub use client::Client;
pub use error::{Error, Result};
pub use http::{is_loopback_base, set_build};
pub use model::{
    Check, CheckState, Comment, MergeMethod, NewRequest, RequestDetails, RequestState,
    RequestSummary, Review, StateFilter, checkout_ref,
};
#[allow(deprecated)]
pub use token::token_from_env;
pub use token::{token_from_env_for, token_from_git_credentials};

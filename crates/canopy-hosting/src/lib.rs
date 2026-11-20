//! Pull requests (GitHub) and merge requests (GitLab) for Canopy, over the providers' REST APIs
//! (plus GraphQL for a list's checks and reviews, [`Client::rollup`], and for searching GitHub's
//! requests, [`Client::search_page`]).
//!
//! This crate is Canopy's only network client besides the `git` executable. Its scope is
//! deliberately narrow:
//!
//! - It talks only to the API of the host the repository's own remote points at:
//!   `api.github.com` for github.com, `https://<host>[:port]/api/v4` for gitlab.com and
//!   self-hosted GitLab, and the GraphQL endpoint on that same host (`api.github.com/graphql`,
//!   `https://<host>[:port]/api/graphql`). The host comes from a [`canopy_core::forge::Forge`]
//!   recognised from a remote URL; author pictures also use validated image hosts
//!   ([`avatars`]), without credentials.
//!   Canopy's own update manifest ([`updates`]) is fetched anonymously from the public
//!   `NecturaLabs/app-updates` repository.
//! - It authenticates only with the sign-in of the `gh` or `glab` CLI ([`login_from_cli`]): the
//!   token it holds for the repository's host is kept in memory, travels only in an
//!   `Authorization: Bearer` header to that host, and never reaches a URL, a log line, a file or
//!   any `Debug` output. No token is typed, saved, or read from the environment or git's
//!   credential helpers.
//! - Every call is blocking (no async runtime), with a 10 s connect and 30 s total timeout, and
//!   is meant to run on a background worker thread. TLS is rustls with the operating system's
//!   certificate verifier, so a self-hosted GitLab behind a corporate CA works; the standard
//!   proxy environment variables are honoured.
//!
//! Errors carry the server's own message ([`Error`]).

pub mod avatars;
mod cli_token;
mod client;
mod error;
mod github;
mod gitlab;
mod http;
mod model;
mod preview;
#[cfg(test)]
mod test_server;
mod token;
mod transport;
pub mod updates;

pub use cli_token::{CliAccount, CliError, CliLogin, install_page, login_command, login_from_cli};
pub use client::{Client, SEARCH_WORDS, is_login};
pub use error::{Error, Result};
pub use http::{is_loopback_base, plain_text, set_build, visible_name};
pub use model::{
    Check, CheckState, Comment, ListPage, MergeMethod, NewRequest, RequestDetails, RequestQuery,
    RequestState, RequestSummary, Review, ReviewDecision, Rollup, SearchOrder, StateFilter,
    checkout_ref,
};
pub use preview::{PREVIEW_CHARS, description_preview};
pub use token::TOKEN_PREFIXES;
pub use transport::default_agent;

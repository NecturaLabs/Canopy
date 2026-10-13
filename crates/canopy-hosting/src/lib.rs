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
//! - It authenticates only with a token the user supplied: the sign-in of the `gh` or `glab` CLI
//!   ([`login_from_cli`], held in memory only), an environment variable
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

pub use cli_token::{CliError, CliLogin, login_from_cli};
pub use client::{Client, SEARCH_WORDS, is_login};
pub use error::{Error, Result};
pub use http::{is_loopback_base, plain_text, set_build};
pub use model::{
    Check, CheckState, Comment, ListPage, MergeMethod, NewRequest, RequestDetails, RequestQuery,
    RequestState, RequestSummary, Review, ReviewDecision, Rollup, SearchOrder, StateFilter,
    checkout_ref,
};
pub use preview::{PREVIEW_CHARS, description_preview};
pub use token::{TOKEN_PREFIXES, token_from_env_for, token_from_git_credentials};
pub use transport::default_agent;

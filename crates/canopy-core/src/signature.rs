//! Whether a commit carries a signature and what `git` makes of it (`%G?` of `git log`).
//!
//! Verifying runs the program `gpg.program` / `gpg.ssh.program` names, which a repository's own
//! configuration can set, so callers verify only in a trusted folder.

use git2::Oid;

use crate::cli;
use crate::{Repo, Result};

/// What verifying a commit's signature found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureStatus {
    /// No signature.
    Unsigned,
    /// A good signature from a key git trusts.
    Verified(Signer),
    /// A good signature, but git does not trust its signer (unknown validity).
    Untrusted(Signer),
    /// A good signature whose key or signature has expired, or whose key was revoked.
    Expired(Signer),
    /// A signature that does not match the commit.
    Invalid(Signer),
    /// Signed, but git could not check it: the key or the verifying program is missing.
    Unavailable,
}

impl SignatureStatus {
    /// Only [`Self::Verified`] is a signature to rely on.
    #[must_use]
    pub fn is_trusted(&self) -> bool {
        matches!(self, Self::Verified(_))
    }
}

/// Who signed, as git reports it. Either part may be empty.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signer {
    /// The signer's name and address (`%GS`).
    pub name: String,
    /// The key (`%GK`).
    pub key: String,
}

impl Repo {
    /// Verifies the signature of `oid` with git. Slow and may start `gpg` or `ssh-keygen`:
    /// call it from the worker, and only for a repository in a trusted folder.
    pub fn commit_signature(&self, oid: Oid) -> Result<SignatureStatus> {
        let id = oid.to_string();
        let out = cli::run(
            self.workdir(),
            &[
                "log",
                "-1",
                "--no-show-signature",
                "--format=%G?%x1f%GS%x1f%GK",
                &id,
                "--",
            ],
        )?;
        let status = parse(out.trim_end_matches('\n'));
        // Git prints `N` for a signed commit it has no way to check (no allowed-signers file),
        // so a signature in the commit itself means "cannot be checked", not "unsigned".
        if status == SignatureStatus::Unsigned && self.git().extract_signature(&oid, None).is_ok() {
            return Ok(SignatureStatus::Unavailable);
        }
        Ok(status)
    }
}

/// The fields of the format string: status letter, signer, key.
const FIELDS: usize = 3;

fn parse(line: &str) -> SignatureStatus {
    let mut parts = line.splitn(FIELDS, '\u{1f}');
    let code = parts.next().unwrap_or("N");
    let signer = Signer {
        name: parts.next().unwrap_or_default().to_owned(),
        key: parts.next().unwrap_or_default().to_owned(),
    };
    match code {
        "G" => SignatureStatus::Verified(signer),
        "U" => SignatureStatus::Untrusted(signer),
        "X" | "Y" | "R" => SignatureStatus::Expired(signer),
        "B" => SignatureStatus::Invalid(signer),
        "E" => SignatureStatus::Unavailable,
        _ => SignatureStatus::Unsigned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_git_status_letter_has_a_meaning() {
        let signer = |name: &str| Signer {
            name: name.into(),
            key: "K".into(),
        };
        assert_eq!(parse("N\u{1f}\u{1f}"), SignatureStatus::Unsigned);
        assert_eq!(
            parse("G\u{1f}Ada\u{1f}K"),
            SignatureStatus::Verified(signer("Ada"))
        );
        assert_eq!(
            parse("U\u{1f}Ada\u{1f}K"),
            SignatureStatus::Untrusted(signer("Ada"))
        );
        for letter in ["X", "Y", "R"] {
            assert_eq!(
                parse(&format!("{letter}\u{1f}Ada\u{1f}K")),
                SignatureStatus::Expired(signer("Ada"))
            );
        }
        assert_eq!(
            parse("B\u{1f}Ada\u{1f}K"),
            SignatureStatus::Invalid(signer("Ada"))
        );
        assert_eq!(parse("E\u{1f}\u{1f}K"), SignatureStatus::Unavailable);
        assert!(!parse("U\u{1f}Ada\u{1f}K").is_trusted());
        assert!(parse("G\u{1f}Ada\u{1f}K").is_trusted());
    }
}

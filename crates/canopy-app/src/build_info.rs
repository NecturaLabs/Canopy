//! The identity of this build, embedded by `build.rs`: the build number (`git rev-list --count` of
//! the commit on `main`) and the commit's short hash. Builds are compared by the number alone;
//! `Cargo.toml`'s version is not bumped per release and says nothing about which build runs.

/// The build number as text: digits, or `dev` when the build had no git history to count.
pub const NUMBER: &str = env!("CANOPY_BUILD_NUMBER");

/// The commit's short hash; empty when the build had no git checkout.
pub const COMMIT: &str = env!("CANOPY_COMMIT");

/// The build number, when this build has one (a `dev` build has none and is never offered
/// updates).
pub fn number() -> Option<u64> {
    canopy_core::update::parse_build(NUMBER)
}

/// How a build is named in the interface: `build 412`.
pub fn name(number: impl std::fmt::Display) -> String {
    format!("build {number}")
}

/// The release notes of this build, embedded by `build.rs` from `CANOPY_RELEASE_NOTES_FILE`
/// (the release workflow sets it); empty in a development build.
pub const RELEASE_NOTES: &str = include_str!(concat!(env!("OUT_DIR"), "/release-notes.md"));

/// This build, named in full: `build 412 (a1b2c3d)`, or `build 412` without a commit.
pub fn label() -> String {
    if COMMIT.is_empty() {
        name(NUMBER)
    } else {
        format!("{} ({COMMIT})", name(NUMBER))
    }
}

/// The same for a title or the About dialog: `Build 412 (a1b2c3d)`.
pub fn title_label() -> String {
    let label = label();
    let mut chars = label.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_build() {
        assert_eq!(name(412), "build 412");
        assert_eq!(name("dev"), "build dev");
    }

    #[test]
    fn this_build_is_a_number_or_dev_with_a_short_hash() {
        assert!(number().is_some() || NUMBER == "dev", "{NUMBER}");
        assert!(COMMIT.len() <= 7 && COMMIT.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(label().starts_with("build "));
        assert!(title_label().starts_with("Build "));
    }
}

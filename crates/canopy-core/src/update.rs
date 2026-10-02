//! Build numbers, for Canopy's update check (the manifest itself is fetched by `canopy-hosting`'s
//! `updates` module).
//!
//! A release is identified by its build number, `git rev-list --count` of its commit on `main`,
//! and builds are ordered by that integer alone. Text that is not a build number, such as the
//! `0.2.0-beta.1` of a release made before builds were numbered, has no number and counts as
//! older than every build.

/// Reads a build number: ASCII digits only, surrounding whitespace ignored. Anything else, a
/// semantic version, a sign, a prefix, an empty string or a number too large for `u64`, is `None`.
pub fn parse_build(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Whether `candidate` is a newer build than `running`. A candidate without a number is never
/// newer.
pub fn is_newer(candidate: Option<u64>, running: u64) -> bool {
    candidate.is_some_and(|c| c > running)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_build_numbers() {
        assert_eq!(parse_build("412"), Some(412));
        assert_eq!(parse_build(" 7\n"), Some(7));
        assert_eq!(parse_build("0"), Some(0));
        assert_eq!(parse_build("0412"), Some(412));
    }

    #[test]
    fn everything_else_has_no_number() {
        for s in [
            "",
            " ",
            "dev",
            "v412",
            "build-412",
            "-3",
            "+3",
            "4 12",
            "4.12",
            "0.2.0",
            "0.2.0-beta.1",
            "99999999999999999999999",
        ] {
            assert_eq!(parse_build(s), None, "{s:?}");
        }
    }

    #[test]
    fn builds_are_ordered_as_integers() {
        // As text "99" sorts after "100"; as numbers it does not.
        assert!(is_newer(parse_build("100"), 99));
        assert!(!is_newer(parse_build("99"), 100));
        assert!(!is_newer(parse_build("100"), 100));
        assert!(is_newer(parse_build("2"), 1));
    }

    #[test]
    fn a_semantic_version_is_never_newer() {
        assert!(!is_newer(parse_build("0.2.0-beta.1"), 0));
        assert!(!is_newer(parse_build("999.0.0"), 1));
        assert!(!is_newer(None, 0));
    }
}

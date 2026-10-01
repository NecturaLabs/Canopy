//! Release versions: parsing and ordering semantic versions, for Canopy's update check (the
//! manifest itself is fetched by `canopy-hosting`'s `updates` module).

use std::cmp::Ordering;
use std::fmt;

/// A semantic version (`MAJOR.MINOR.PATCH[-PRERELEASE]`). Build metadata (`+…`) is accepted
/// and discarded, since it does not affect precedence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Major version.
    pub major: u64,
    /// Minor version.
    pub minor: u64,
    /// Patch version.
    pub patch: u64,
    /// Pre-release identifiers without the leading `-`, e.g. `beta.1`.
    pub pre: Option<String>,
}

/// A numeric identifier: digits only, no leading zero unless it is `0` (SemVer 2.0.0 §2, §9).
fn numeric(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0'))
    {
        return None;
    }
    s.parse().ok()
}

fn valid_pre(pre: &str) -> bool {
    pre.split('.').all(|id| {
        !id.is_empty()
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && (!id.bytes().all(|b| b.is_ascii_digit()) || numeric(id).is_some())
    })
}

/// Compares two pre-release strings by SemVer precedence (§11.4).
fn cmp_pre(a: &str, b: &str) -> Ordering {
    let mut left = a.split('.');
    let mut right = b.split('.');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let is_num = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
                let ord = match (is_num(x), is_num(y)) {
                    // Valid numeric identifiers have no leading zeros, so the longer is larger.
                    (true, true) => x.len().cmp(&y.len()).then_with(|| x.cmp(y)),
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => x.cmp(y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

impl Version {
    /// Parses `1.2.3`, `v1.2.3` or `1.2.3-beta.1`. Rejects anything that is not a full
    /// three-part SemVer version, such as `1.2` or `release-7`.
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let s = match s.split_once('+') {
            Some((core, build)) => {
                // Build identifiers may have leading zeros, unlike pre-release ones (§10).
                let valid_build = build.split('.').all(|id| {
                    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                });
                if !valid_build {
                    return None;
                }
                core
            }
            None => s,
        };
        let (core, pre) = match s.split_once('-') {
            Some((core, pre)) => {
                if !valid_pre(pre) {
                    return None;
                }
                (core, Some(pre.to_owned()))
            }
            None => (s, None),
        };
        let mut parts = core.split('.');
        let major = numeric(parts.next()?)?;
        let minor = numeric(parts.next()?)?;
        let patch = numeric(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
            pre,
        })
    }

    /// Whether this is a pre-release such as `1.0.0-rc.1`.
    pub fn is_prerelease(&self) -> bool {
        self.pre.is_some()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => cmp_pre(a, b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("{s} should parse"))
    }

    /// `(input, (major, minor, patch, pre))`.
    type ParseCase<'a> = (&'a str, (u64, u64, u64, Option<&'a str>));

    #[test]
    fn parses_versions() {
        let cases: &[ParseCase<'_>] = &[
            ("1.2.3", (1, 2, 3, None)),
            ("v1.2.3", (1, 2, 3, None)),
            ("V0.0.0", (0, 0, 0, None)),
            (" v10.20.30 ", (10, 20, 30, None)),
            ("1.2.3-beta.1", (1, 2, 3, Some("beta.1"))),
            ("v1.0.0-rc.1", (1, 0, 0, Some("rc.1"))),
            ("1.0.0-alpha-1.0", (1, 0, 0, Some("alpha-1.0"))),
            ("1.0.0-0.3.7", (1, 0, 0, Some("0.3.7"))),
            ("1.0.0+build.5", (1, 0, 0, None)),
            ("1.0.0-beta+exp.sha.5114f85", (1, 0, 0, Some("beta"))),
        ];
        for (s, (major, minor, patch, pre)) in cases {
            assert_eq!(
                Version::parse(s),
                Some(Version {
                    major: *major,
                    minor: *minor,
                    patch: *patch,
                    pre: pre.map(str::to_owned)
                }),
                "{s}"
            );
        }
    }

    #[test]
    fn rejects_non_versions() {
        for s in [
            "",
            "v",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "a.b.c",
            "release-7",
            "notaversion",
            "vv1.2.3",
            "01.2.3",
            "1.02.3",
            "1.2.3-",
            "1.2.3-beta..1",
            "1.2.3-01",
            "1.2.3-be ta",
            "1.2.3+",
            "1.2.3+a..b",
            "-1.2.3",
            "1.2.3 4",
            "99999999999999999999.0.0",
        ] {
            assert_eq!(Version::parse(s), None, "{s:?}");
        }
    }

    #[test]
    fn orders_by_semver_precedence() {
        // The example chain from SemVer 2.0.0 §11, plus core-version ordering.
        let chain = [
            "0.9.9",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "1.0.1",
            "1.1.0",
            "1.10.0",
            "2.0.0",
        ];
        for pair in chain.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
            assert!(v(pair[1]) > v(pair[0]), "{} > {}", pair[1], pair[0]);
        }
        assert_eq!(v("v1.2.3").cmp(&v("1.2.3")), Ordering::Equal);
        assert_eq!(v("1.2.3+a").cmp(&v("1.2.3+b")), Ordering::Equal);
        assert!(v("1.0.0-2") < v("1.0.0-10"));
        assert!(v("1.0.0-999") < v("1.0.0-a"));
    }

    #[test]
    fn displays_without_prefix() {
        assert_eq!(v("v1.2.3").to_string(), "1.2.3");
        assert_eq!(v("1.2.3-beta.1+build").to_string(), "1.2.3-beta.1");
        assert!(v("1.2.3-beta.1").is_prerelease());
        assert!(!v("1.2.3").is_prerelease());
    }
}

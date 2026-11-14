//! The shapes of GitHub and GitLab access tokens, so a log line, an error or a crash report never
//! carries one.

/// Known GitHub and GitLab token prefixes shared by API error and crash-report scrubbing.
pub const TOKEN_PREFIXES: [&str; 17] = [
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "gldt-",
    "glptt-",
    "glrt-",
    "gloas-",
    "glcbt-",
    "glft-",
    "glimt-",
    "glagent-",
    "glsoat-",
    "glffct-",
];

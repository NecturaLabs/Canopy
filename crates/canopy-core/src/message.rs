//! Checks on a commit message before it is committed.
//!
//! These are hints, not rules: nothing here stops a commit or changes the message. The limit is
//! the 72 characters git tooling conventionally wraps at, which most history views, mail
//! clients and terminals show in full.

/// The most characters a subject or a body line should have.
pub const LIMIT: usize = 72;

/// Something a commit message does that tools commonly cut off or wrap badly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// The summary has more than [`LIMIT`] characters.
    SummaryTooLong {
        /// Its length in characters, trailing whitespace not counted.
        chars: usize,
    },
    /// The description has lines of more than [`LIMIT`] characters that a reader cannot
    /// wrap: not indented, not quoted and with more than one word.
    DescriptionTooWide {
        /// How many such lines there are.
        lines: usize,
        /// The first of them, counted from 1 in the description.
        first: usize,
        /// The first one's length in characters.
        chars: usize,
    },
}

impl Problem {
    /// The problem as one plain sentence for the person writing the message.
    #[must_use]
    pub fn describe(&self) -> String {
        match *self {
            Self::SummaryTooLong { chars } => {
                format!("Subject is {chars} characters; {LIMIT} is the most tools show.")
            }
            Self::DescriptionTooWide {
                lines: 1,
                first,
                chars,
            } => format!("Body line {first} is {chars} characters; wrap it at {LIMIT}."),
            Self::DescriptionTooWide { lines, first, .. } => {
                format!("{lines} body lines pass {LIMIT} characters; the first is line {first}.")
            }
        }
    }
}

/// The problems in a message made of `summary` and `body`, in reading order; none when it is
/// fine. Characters are Unicode scalar values, not bytes.
///
/// A description line is left alone when it starts with whitespace (a list continuation, code
/// or a table), starts with `>` (a quote) or is a single word (a URL or a path), since none of
/// those can be wrapped without changing them. A summary's trailing whitespace is ignored, as
/// git strips it.
#[must_use]
pub fn check(summary: &str, body: &str) -> Vec<Problem> {
    let mut found = Vec::new();
    let chars = summary.trim_end().chars().count();
    if chars > LIMIT {
        found.push(Problem::SummaryTooLong { chars });
    }
    let mut wide = body
        .lines()
        .enumerate()
        .filter(|(_, line)| !is_exempt(line))
        .map(|(i, line)| (i + 1, line.trim_end().chars().count()))
        .filter(|&(_, chars)| chars > LIMIT);
    if let Some((first, chars)) = wide.next() {
        found.push(Problem::DescriptionTooWide {
            lines: 1 + wide.count(),
            first,
            chars,
        });
    }
    found
}

/// Whether a description line is one that is not wrapped: indented, quoted or a single word.
fn is_exempt(line: &str) -> bool {
    line.starts_with(char::is_whitespace)
        || line.starts_with('>')
        || !line.trim_end().contains(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line of exactly `chars` characters made of words, so it can wrap, and ending in a
    /// letter so nothing trims off its end.
    fn words(chars: usize) -> String {
        let mut line: Vec<char> = "abcdefghi "
            .repeat(chars / 10 + 1)
            .chars()
            .take(chars)
            .collect();
        if let Some(last) = line.last_mut() {
            *last = 'z';
        }
        line.into_iter().collect()
    }

    #[test]
    fn a_short_message_has_no_problems() {
        assert_eq!(
            check("Add metrics", "Explains why.\n\nSecond paragraph."),
            []
        );
        assert_eq!(check("", ""), []);
    }

    #[test]
    fn the_summary_limit_is_72_characters() {
        assert_eq!(check(&words(72), ""), []);
        assert_eq!(
            check(&words(73), ""),
            [Problem::SummaryTooLong { chars: 73 }]
        );
    }

    #[test]
    fn the_summary_counts_characters_not_bytes() {
        // 72 two-byte characters are 144 bytes but fit.
        assert_eq!(check(&"é".repeat(72), ""), []);
        assert_eq!(
            check(&"é".repeat(73), ""),
            [Problem::SummaryTooLong { chars: 73 }]
        );
        // Four-byte characters too.
        assert_eq!(check(&"😀".repeat(72), ""), []);
    }

    #[test]
    fn trailing_whitespace_in_the_summary_is_not_counted() {
        let summary = format!("{}   \t", words(72));
        assert_eq!(check(&summary, ""), []);
    }

    #[test]
    fn a_summary_over_the_limit_says_how_long_it_is() {
        let text = check(&words(80), "")[0].describe();
        assert_eq!(text, "Subject is 80 characters; 72 is the most tools show.");
    }

    #[test]
    fn the_description_limit_is_72_characters() {
        assert_eq!(check("s", &words(72)), []);
        assert_eq!(
            check("s", &words(73)),
            [Problem::DescriptionTooWide {
                lines: 1,
                first: 1,
                chars: 73
            }]
        );
    }

    #[test]
    fn the_description_counts_characters_not_bytes() {
        let line = format!("{} é", "é".repeat(70));
        assert_eq!(line.chars().count(), 72);
        assert_eq!(check("s", &line), []);
        assert_eq!(check("s", &format!("{line}é")).len(), 1);
    }

    #[test]
    fn long_lines_that_cannot_be_wrapped_are_left_alone() {
        let url = format!("https://example.com/{}", "a".repeat(90));
        let indented = format!("    {}", words(90));
        let tabbed = format!("\t{}", words(90));
        let quoted = format!("> {}", words(90));
        let body = format!("{url}\n{indented}\n{tabbed}\n{quoted}");
        assert_eq!(check("s", &body), []);
    }

    #[test]
    fn a_long_line_with_trailing_spaces_still_needs_words_to_count() {
        // One word followed by spaces is still one word.
        let body = format!("{}   ", "a".repeat(90));
        assert_eq!(check("s", &body), []);
    }

    #[test]
    fn crlf_line_endings_do_not_add_a_character() {
        let body = format!("{}\r\nnext", words(72));
        assert_eq!(check("s", &body), []);
    }

    #[test]
    fn the_description_names_the_first_wide_line_and_counts_them() {
        let body = format!("short\n\n{}\nfine\n{}\n{}", words(80), words(90), words(75));
        assert_eq!(
            check("s", &body),
            [Problem::DescriptionTooWide {
                lines: 3,
                first: 3,
                chars: 80
            }]
        );
    }

    #[test]
    fn both_parts_are_reported_in_reading_order() {
        let found = check(&words(90), &words(100));
        assert_eq!(found.len(), 2);
        assert!(matches!(found[0], Problem::SummaryTooLong { chars: 90 }));
        assert!(matches!(found[1], Problem::DescriptionTooWide { .. }));
    }

    #[test]
    fn descriptions_are_plain_and_specific() {
        let one = Problem::DescriptionTooWide {
            lines: 1,
            first: 4,
            chars: 85,
        };
        assert_eq!(
            one.describe(),
            "Body line 4 is 85 characters; wrap it at 72."
        );
        let many = Problem::DescriptionTooWide {
            lines: 3,
            first: 2,
            chars: 85,
        };
        assert_eq!(
            many.describe(),
            "3 body lines pass 72 characters; the first is line 2."
        );
    }
}

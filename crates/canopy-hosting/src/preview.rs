//! A request's description as one line of plain text, for a list row and its text filter.
//!
//! Both providers' list responses already carry the description (`body` on GitHub, `description`
//! on GitLab), so a row needs no request of its own: the Markdown is flattened here, on the
//! hosting thread, and only the first part is kept.

use crate::http::plain_text;

/// The most characters of a description kept for a list row.
pub const PREVIEW_CHARS: usize = 600;
/// How much of a description is read at all: a pull request template's comments and headings can
/// fill the start of a long one.
const SOURCE_CHARS: usize = 8_000;
/// The fewest characters of a horizontal rule or a table's separator row (`---`).
const RULE_CHARS: usize = 3;
/// The width of a doubled emphasis mark (`**`, `__`, `~~`).
const DOUBLE_MARK: usize = 2;
/// What stands between two paragraphs, so the end of one sentence does not run into the start of
/// the next.
const PARAGRAPH_BREAK: &str = " \u{B7} ";
/// The Braille blank: a character that draws nothing and is neither whitespace nor ignorable.
const BRAILLE_BLANK: char = '\u{2800}';
/// The most characters a link's target (or a reference's name) is read over. A target longer than
/// this is left as text, which bounds what a run of openers can cost.
const TARGET_CHARS: usize = 512;

/// `markdown` flattened to plain text on one line, at most [`PREVIEW_CHARS`] characters; empty
/// when the description says nothing.
///
/// Comments, HTML tags, code fences, heading, quote and list marks and emphasis are dropped,
/// links and images reduced to their text, paragraphs told apart by [`PARAGRAPH_BREAK`], and
/// control and invisible characters removed ([`plain_text`]).
pub fn description_preview(markdown: &str) -> String {
    let source: String = markdown
        .chars()
        .take(SOURCE_CHARS)
        .map(|c| if c == BRAILLE_BLANK { ' ' } else { c })
        .collect();
    let mut flat = String::new();
    let mut in_fence = false;
    let mut new_paragraph = false;
    for line in without_comments(&source).lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if line.is_empty() && !in_fence {
            new_paragraph = true;
            continue;
        }
        let text = if in_fence {
            line.to_owned()
        } else if let Some(text) = block_text(line) {
            inline(text)
        } else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        if !flat.is_empty() {
            flat.push_str(if new_paragraph { PARAGRAPH_BREAK } else { " " });
        }
        new_paragraph = false;
        flat.push_str(&text);
    }
    plain_text(&flat, usize::MAX)
        .chars()
        .take(PREVIEW_CHARS)
        .collect()
}

/// `text` without its `<!-- … -->` comments (an unterminated one takes the rest).
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("<!--") {
        out.push_str(before);
        let Some((_, tail)) = after.split_once("-->") else {
            return out;
        };
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// What a line of Markdown says once its block marks are off: `None` for a rule or a table's
/// separator row.
fn block_text(line: &str) -> Option<&str> {
    if line.is_empty() {
        return None;
    }
    let only = |allowed: &str| line.chars().all(|c| allowed.contains(c) || c == ' ');
    if line.len() >= RULE_CHARS && only("-*_=|:") && line.chars().any(|c| "-*_=".contains(c)) {
        return None;
    }
    let mut text = line;
    while let Some(rest) = text.strip_prefix('>') {
        text = rest.trim_start();
    }
    let unhashed = text.trim_start_matches('#');
    if unhashed.len() < text.len()
        && let Some(rest) = unhashed.strip_prefix(' ')
    {
        text = rest.trim_start();
    }
    for mark in ["- ", "* ", "+ "] {
        if let Some(rest) = text.strip_prefix(mark) {
            text = rest.trim_start();
            break;
        }
    }
    let undigited = text.trim_start_matches(|c: char| c.is_ascii_digit());
    if undigited.len() < text.len()
        && let Some(rest) = undigited
            .strip_prefix(". ")
            .or_else(|| undigited.strip_prefix(") "))
    {
        text = rest.trim_start();
    }
    for boxed in ["[ ] ", "[x] ", "[X] "] {
        if let Some(rest) = text.strip_prefix(boxed) {
            text = rest;
            break;
        }
    }
    Some(text)
}

/// A line's inline Markdown flattened: links and images keep their text, tags and emphasis
/// marks go, an autolink keeps its address.
fn inline(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let word = |i: Option<usize>| {
        i.and_then(|i| c.get(i))
            .is_some_and(|ch| ch.is_alphanumeric())
    };
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '!' if c.get(i + 1) == Some(&'[') => {
                if let Some((label, end)) = bracketed(&c, i + 1) {
                    out.push_str(&inline(&label));
                    i = end;
                } else {
                    out.push('!');
                    i += 1;
                }
            }
            '[' => {
                if let Some((label, end)) = bracketed(&c, i) {
                    out.push_str(&inline(&label));
                    i = end;
                } else {
                    out.push('[');
                    i += 1;
                }
            }
            '<' => {
                // The tag or address ends at the first `>`, and a `<` before it is not one: the
                // search stops there, so a run of `<` is scanned once, not once each.
                let close = c[i + 1..]
                    .iter()
                    .position(|&ch| ch == '>' || ch == '<')
                    .filter(|&at| c[i + 1 + at] == '>')
                    .map(|at| at + 1);
                let inner: String = close
                    .map(|close| c[i + 1..i + close].iter().collect())
                    .unwrap_or_default();
                let address = ["http://", "https://", "mailto:"]
                    .iter()
                    .any(|scheme| inner.starts_with(scheme))
                    && !inner.contains(' ');
                let tag = inner
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '/' || ch == '!');
                match close {
                    Some(close) if address || tag => {
                        // An autolink keeps its address; a tag is a word break.
                        out.push_str(if address { &inner } else { " " });
                        i += close + 1;
                    }
                    _ => {
                        out.push('<');
                        i += 1;
                    }
                }
            }
            '`' => i += 1,
            '|' => {
                out.push(' ');
                i += 1;
            }
            ch @ ('*' | '_' | '~') => {
                if c.get(i + 1) == Some(&ch) {
                    // `**`, `__` and `~~`.
                    i += DOUBLE_MARK;
                    continue;
                }
                // A lone `*` or `_` is emphasis only at the edge of a word: not in `snake_case`,
                // not in a glob such as `*.rs`. A lone `~` is text.
                if ch == '~' || word(i.checked_sub(1)) == word(Some(i + 1)) {
                    out.push(ch);
                }
                i += 1;
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// A link, reference or badge link starting at `open`: its label and the index after the
/// complete link. `None` for a bare bracket.
fn bracketed(c: &[char], open: usize) -> Option<(String, usize)> {
    let start = open + 1;
    let (start, badge) = match &c[start..] {
        ['[', ..] => (start + 1, true),
        ['!', '[', ..] => (start + 2, true), // Skip `![`.
        _ => (start, false),
    };
    // Allow one nested opener for a badge. Stop at any further `[` so runs of openers stay linear.
    let close = start
        + c[start..]
            .iter()
            .position(|&ch| ch == ']' || ch == '[')
            .filter(|&at| c[start + at] == ']')?;
    let closer = match c.get(close + 1)? {
        '(' => ')',
        '[' if !badge => ']',
        _ => return None,
    };
    let mut end = target_end(c, close + 1, closer)?;
    if badge {
        // The inner target must be followed immediately by the enclosing link's target.
        if !c[end..].starts_with(&[']', '(']) {
            return None;
        }
        end = target_end(c, end + 1, ')')?;
    }
    Some((c[start..close].iter().collect(), end))
}

/// The index after a link or reference target, within the bounded scan from its opener.
fn target_end(c: &[char], open: usize, closer: char) -> Option<usize> {
    c[open..]
        .iter()
        .take(TARGET_CHARS)
        .position(|&ch| ch == closer)
        .map(|offset| open + offset + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_is_flattened_to_its_words() {
        let text = "## Summary\n\nAdds **bold** and _soft_ text, a [link](https://example.com/a), \
            an ![screenshot](shot.png) and `code`.\n\n> quoted\n- [x] done\n- [ ] open\n1. first\n\
            ---\n| a | b |\n|---|---|\n| 1 | 2 |";
        assert_eq!(
            description_preview(text),
            "Summary · Adds bold and soft text, a link, an screenshot and code. · quoted done \
             open first a b 1 2"
        );
    }

    #[test]
    fn a_badge_link_keeps_only_its_label() {
        for input in ["[[alt](img)](url)", "[![alt](img)](url)"] {
            assert_eq!(description_preview(input), "alt", "{input}");
        }
    }

    #[test]
    fn a_badge_link_leaves_following_text() {
        assert_eq!(
            description_preview("[![**Build**](img)](url) passed"),
            "Build passed"
        );
    }

    #[test]
    fn consecutive_badge_links_keep_both_labels() {
        assert_eq!(
            description_preview("[![Build](img)](url)[[Coverage](img)](url)"),
            "BuildCoverage"
        );
    }

    #[test]
    fn comments_tags_and_fences_do_not_reach_the_preview() {
        let text = "<!-- Thanks for the pull request! Describe it here. -->\n\
            Fixes <b>#12</b><br/> and <https://example.com/x>.\n```rust\nlet a = 1;\n```\n\
            <!-- unterminated\nnever shown";
        assert_eq!(
            description_preview(text),
            "Fixes #12 and https://example.com/x. let a = 1;"
        );
    }

    #[test]
    fn identifiers_and_globs_keep_their_marks() {
        assert_eq!(
            description_preview("rename my_var_name to *.rs globs, a * b and 2 < 3"),
            "rename my_var_name to *.rs globs, a * b and 2 < 3"
        );
    }

    #[test]
    fn a_description_that_says_nothing_has_no_preview() {
        for empty in [
            "",
            "   \n\n",
            "<!-- only a template -->",
            "---",
            "![](x.png)",
            "\u{2800}\u{2800}\n\n\u{2800}",
        ] {
            assert_eq!(description_preview(empty), "", "{empty:?}");
        }
    }

    #[test]
    fn a_long_description_is_cut_and_made_plain() {
        let long = "word ".repeat(1_000);
        assert_eq!(description_preview(&long).chars().count(), PREVIEW_CHARS);
        let hidden = "a\u{202E}b\u{200B}c \u{7}d";
        assert_eq!(description_preview(hidden), "abc d");
    }

    #[test]
    fn a_pasted_token_is_not_kept() {
        let text = "use ghp_abcdefghijklmnopqrstuvwxyz0123456789ABCD to log in";
        let preview = description_preview(text);
        assert!(!preview.contains("ghp_abcdef"), "{preview}");
    }

    #[test]
    fn paragraphs_are_told_apart_and_blanks_leave_no_separator() {
        assert_eq!(
            description_preview("First idea.\n\nSecond idea.\nStill the second."),
            "First idea. · Second idea. Still the second."
        );
        assert_eq!(
            description_preview("\n\nA\n\n\n<!-- note -->\n\nB\n\n"),
            "A · B"
        );
        assert_eq!(
            description_preview("```\na\n\nb\n```\nafter"),
            "a b after",
            "a blank line in code is not a paragraph break"
        );
        assert_eq!(description_preview("A\n\n![](x.png)\n\nB"), "A · B");
        let long = format!("{}\n\n{}", "word ".repeat(400), "more ".repeat(400));
        assert_eq!(description_preview(&long).chars().count(), PREVIEW_CHARS);
    }

    #[test]
    fn a_braille_blank_is_a_space() {
        assert_eq!(description_preview("a\u{2800}b"), "a b");
    }

    #[test]
    fn hangul_fillers_are_dropped_from_preview_text() {
        for filler in ['\u{115F}', '\u{1160}', '\u{3164}', '\u{FFA0}'] {
            assert_eq!(
                description_preview(&format!("a{filler}b")),
                "ab",
                "U+{:04X}",
                u32::from(filler)
            );
        }
    }

    #[test]
    fn unclosed_marks_cost_one_pass_and_closed_ones_still_work() {
        // Far past what a description is cut to, so a pass per mark could not finish.
        let n = 200_000;
        let started = std::time::Instant::now();
        for hostile in [
            format!("{}>", "<a".repeat(n / 2)),
            "<".repeat(n),
            "[".repeat(n),
            "[a](".repeat(n / 4),
            "[a][".repeat(n / 4),
        ] {
            let flat = inline(&hostile);
            assert_ne!(flat, "");
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );

        assert_eq!(
            description_preview("<<<a [[b](c) [d](e) <f>g</f> [h"),
            "<<<a [b d g [h"
        );
        assert_eq!(
            description_preview("<https://example.com/a?b=1> and [x][y]"),
            "https://example.com/a?b=1 and x"
        );
    }
}

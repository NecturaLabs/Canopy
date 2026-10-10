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

/// `markdown` flattened to plain text on one line, at most [`PREVIEW_CHARS`] characters; empty
/// when the description says nothing.
///
/// Comments, HTML tags, code fences, heading, quote and list marks and emphasis are dropped,
/// links and images reduced to their text, and control and invisible characters removed
/// ([`plain_text`]).
pub fn description_preview(markdown: &str) -> String {
    let source: String = markdown.chars().take(SOURCE_CHARS).collect();
    let mut flat = String::new();
    let mut in_fence = false;
    for line in without_comments(&source).lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        let text = if in_fence {
            line.to_owned()
        } else if let Some(text) = block_text(line) {
            inline(text)
        } else {
            continue;
        };
        flat.push_str(&text);
        flat.push(' ');
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
                let close = c[i..].iter().position(|&ch| ch == '>');
                let inner: String = close
                    .map(|close| c[i + 1..i + close].iter().collect())
                    .unwrap_or_default();
                let address = ["http://", "https://", "mailto:"]
                    .iter()
                    .any(|scheme| inner.starts_with(scheme))
                    && !inner.contains(' ');
                let tag = !inner.contains('<')
                    && inner
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

/// A link or reference `[label](target)` / `[label][reference]` starting at `open`: its label
/// and the index after it. `None` for a bare bracket.
fn bracketed(c: &[char], open: usize) -> Option<(String, usize)> {
    let close = open + c[open..].iter().position(|&ch| ch == ']')?;
    let closer = match c.get(close + 1)? {
        '(' => ')',
        '[' => ']',
        _ => return None,
    };
    let end = close + 1 + c[close + 1..].iter().position(|&ch| ch == closer)?;
    Some((c[open + 1..close].iter().collect(), end + 1))
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
            "Summary Adds bold and soft text, a link, an screenshot and code. quoted done open \
             first a b 1 2"
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
}

//! Require the Git isolation wrapper before an integration test runs any code.

use std::fs;
use std::iter::Peekable;
use std::path::Path;

#[test]
fn integration_tests_start_with_git_isolation() -> std::io::Result<()> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut problems = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "rs")
            && path
                .file_name()
                .is_some_and(|name| name != "isolation_guard.rs")
        {
            check_tests(&fs::read_to_string(&path)?, &path, &mut problems);
        }
    }
    problems.sort();
    let problems = problems.join("\n");
    assert!(problems.is_empty(), "\n{problems}");
    Ok(())
}

#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "an unrecognised test declaration fails the guard"
)]
fn check_tests(source: &str, path: &Path, problems: &mut Vec<String>) {
    let mut tokens = Tokens(source).peekable();
    while let Some(token) = tokens.next() {
        if token != "#" || tokens.next_if_eq(&"[").is_none() {
            continue;
        }
        if tokens.next_if_eq(&"test").is_none() || tokens.next_if_eq(&"]").is_none() {
            skip_to(&mut tokens, "]");
            continue;
        }

        skip_attributes(&mut tokens);
        let name = loop {
            match tokens.next() {
                Some("fn") => break tokens.next().expect("test function name"),
                Some("pub" | "const" | "async" | "unsafe" | "extern") => (),
                Some("(") => skip_to(&mut tokens, ")"),
                other => panic!(
                    "{}: expected a test function, found {other:?}",
                    path.display()
                ),
            }
        };
        skip_to(&mut tokens, "{");
        if !starts_with_isolation(&mut tokens, name) {
            problems.push(format!(
                "{}: {name} must start with isolate_git!({name})",
                path.display(),
            ));
        }
    }
}

fn starts_with_isolation(tokens: &mut Peekable<Tokens<'_>>, name: &str) -> bool {
    loop {
        let attributed = tokens.peek() == Some(&"#");
        skip_attributes(tokens);
        match tokens.next_if(|token| matches!(*token, "const" | "use" | "struct" | "impl")) {
            Some("const") => {
                // A constant has a name and a type. A const fn ends with its body instead.
                if tokens
                    .next()
                    .is_none_or(|token| !token.starts_with(is_word))
                    || tokens.next_if_eq(&":").is_none()
                {
                    return false;
                }
                skip_to(tokens, ";");
            }
            Some("use") => skip_to(tokens, ";"),
            Some(_) => {
                // Tuple and unit structs end in a semicolon. Named fields and impls have a body.
                if skip_to_any(tokens, &["{", ";"]) == Some("{") {
                    skip_to(tokens, "}");
                }
            }
            // An attribute on the call itself could compile it out.
            None if attributed => return false,
            None => break,
        }
    }
    if !["isolate_git", "!", "(", name]
        .into_iter()
        .all(|part| tokens.next_if_eq(&part).is_some())
        || !matches!(tokens.peek(), Some(&("," | ")")))
    {
        return false;
    }
    skip_to(tokens, ")");
    tokens.next_if_eq(&";").is_some()
}

fn skip_attributes(tokens: &mut Peekable<Tokens<'_>>) {
    while tokens.next_if_eq(&"#").is_some() {
        assert_eq!(tokens.next(), Some("["));
        skip_to(tokens, "]");
    }
}

fn skip_to(tokens: &mut Peekable<Tokens<'_>>, end: &str) {
    assert_eq!(skip_to_any(tokens, &[end]), Some(end));
}

fn skip_to_any<'a>(tokens: &mut Peekable<Tokens<'a>>, ends: &[&str]) -> Option<&'a str> {
    while let Some(token) = tokens.next() {
        if ends.contains(&token) {
            return Some(token);
        }
        match token {
            "(" => skip_to(tokens, ")"),
            "[" => skip_to(tokens, "]"),
            "{" => skip_to(tokens, "}"),
            _ => (),
        }
    }
    None
}

/// Return identifiers, punctuation and whole literals. Comments and whitespace have no tokens.
struct Tokens<'a>(&'a str);

impl<'a> Iterator for Tokens<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.0 = self.0.trim_start();
            if let Some(comment) = self.0.strip_prefix("//") {
                self.0 = comment.split_once('\n').map_or("", |(_, rest)| rest);
            } else if let Some(comment) = self.0.strip_prefix("/*") {
                self.0 = after_block_comment(comment);
            } else {
                break;
            }
        }

        let first = self.0.chars().next()?;
        let end = literal_end(self.0).unwrap_or_else(|| {
            if is_word(first) {
                self.0.find(|c: char| !is_word(c)).unwrap_or(self.0.len())
            } else {
                first.len_utf8()
            }
        });
        let (token, rest) = self.0.split_at(end);
        self.0 = rest;
        Some(token)
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[expect(clippy::expect_used, reason = "an unterminated comment fails the scan")]
fn after_block_comment(mut text: &str) -> &str {
    let mut depth = 1;
    while depth > 0 {
        let end = text.find(['/', '*']).expect("block comment ends");
        text = text.split_at(end).1;
        if let Some(rest) = text.strip_prefix("/*") {
            depth += 1;
            text = rest;
        } else if let Some(rest) = text.strip_prefix("*/") {
            depth -= 1;
            text = rest;
        } else {
            text = text.split_at(1).1;
        }
    }
    text
}

fn literal_end(source: &str) -> Option<usize> {
    let literal = source
        .strip_prefix(['b', 'c'])
        .filter(|rest| rest.starts_with(['r', '"', '\'']))
        .unwrap_or(source);
    if let Some(raw) = literal.strip_prefix('r') {
        let quote = raw.trim_start_matches('#');
        if let Some(body) = quote.strip_prefix('"') {
            let hashes = raw.len() - quote.len();
            let close = format!("\"{}", "#".repeat(hashes));
            return Some(source.len() - body.len() + body.find(&close)? + close.len());
        }
    }
    if let Some(body) = literal.strip_prefix('"') {
        let mut escaped = false;
        for (index, c) in body.char_indices() {
            if c == '"' && !escaped {
                return Some(source.len() - body.len() + index + 1);
            }
            escaped = c == '\\' && !escaped;
        }
    }
    if let Some(body) = literal.strip_prefix('\'') {
        // A lifetime has no closing quote. A character has one after its value or escape.
        let first = body.chars().next()?;
        let end = if first == '\\' {
            body.get(2..)?.find('\'')? + 2
        } else {
            first.len_utf8()
        };
        if body.get(end..)?.starts_with('\'') {
            return Some(source.len() - body.len() + end + 1);
        }
    }
    None
}

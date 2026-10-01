//! Syntax highlighting for diffs with `syntect` (grammars from syntect and bat, through
//! `two-face`), computed on a small pool of highlighter threads shared by every tab, so neither
//! the UI nor a repository thread waits for it. The diff shows as plain text until the result
//! arrives; a request that a newer diff of the same tab superseded is abandoned.
//!
//! A diff is two partial files: the old side (context and removed lines) and the new side
//! (context and added lines). Each side is parsed in order with its own parser state, so
//! multi-line constructs such as block comments and strings carry across lines within a hunk.
//! Every hunk starts from a fresh parser state: a hunk can begin inside a construct the diff
//! does not show (the middle of a block comment), and that must not colour the hunks after it.
//! Grammar scopes map to a handful of [`Token`] kinds; the colours come from the active theme
//! (`Palette::syntax`), never from a syntect theme.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use canopy_core::{FileDiff, LineKind};

/// What a span of code is, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Comment,
    Number,
    Function,
    Type,
    Constant,
}

/// One coloured span: byte range in the line's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub token: Token,
}

/// Spans for every line of a diff: `lines[hunk][line]`. Plain text has no span.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Highlights {
    pub lines: Vec<Vec<Vec<Span>>>,
    /// Grammar name, shown in the diff header.
    pub language: String,
}

/// Diffs larger than this are not highlighted (they would take too long to parse).
const MAX_LINES: usize = 40_000;
/// Lines longer than this (minified code, data) are left plain.
const MAX_LINE_BYTES: usize = 2_000;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// Scope prefixes in priority order, checked from the innermost scope outwards.
fn rules() -> &'static [(Scope, Token)] {
    static RULES: OnceLock<Vec<(Scope, Token)>> = OnceLock::new();
    RULES.get_or_init(|| {
        let table: &[(&str, Token)] = &[
            ("comment", Token::Comment),
            ("punctuation.definition.comment", Token::Comment),
            ("string", Token::String),
            ("constant.character", Token::String),
            ("markup.raw", Token::String),
            ("markup.inline.raw", Token::String),
            ("constant.numeric", Token::Number),
            ("constant.language", Token::Constant),
            ("constant.other", Token::Constant),
            ("support.constant", Token::Constant),
            ("variable.language", Token::Constant),
            ("markup.underline.link", Token::Constant),
            ("entity.name.function", Token::Function),
            ("support.function", Token::Function),
            ("variable.function", Token::Function),
            ("meta.function-call.identifier", Token::Function),
            ("entity.other.attribute-name", Token::Function),
            ("markup.heading", Token::Function),
            ("entity.name.type", Token::Type),
            ("entity.name.class", Token::Type),
            ("entity.name.struct", Token::Type),
            ("entity.name.enum", Token::Type),
            ("entity.name.trait", Token::Type),
            ("entity.name.interface", Token::Type),
            ("entity.other.inherited-class", Token::Type),
            ("support.type", Token::Type),
            ("support.class", Token::Type),
            ("storage.type", Token::Keyword),
            ("storage.modifier", Token::Keyword),
            ("keyword", Token::Keyword),
            ("entity.name.tag", Token::Keyword),
            ("entity.name.section", Token::Function),
        ];
        table
            .iter()
            .filter_map(|(s, t)| Scope::new(s).ok().map(|scope| (scope, *t)))
            .collect()
    })
}

fn classify(stack: &ScopeStack) -> Option<Token> {
    let rules = rules();
    for scope in stack.as_slice().iter().rev() {
        // Operators and punctuation inside other constructs stay plain unless the construct is
        // a string or comment (handled by their own prefixes further out).
        if let Some((_, t)) = rules.iter().find(|(prefix, _)| prefix.is_prefix_of(*scope)) {
            return Some(*t);
        }
    }
    None
}

/// The grammar for `path`, else one recognised from `first_line`.
fn syntax_for<'a>(set: &'a SyntaxSet, path: &str, first_line: &str) -> Option<&'a SyntaxReference> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e);
    ext.and_then(|e| set.find_syntax_by_extension(e))
        .or_else(|| set.find_syntax_by_extension(name))
        .or_else(|| set.find_syntax_by_first_line(first_line))
        .filter(|s| s.name != "Plain Text")
}

/// One side of the diff being parsed.
struct Side {
    state: ParseState,
    stack: ScopeStack,
}

impl Side {
    fn new(syntax: &SyntaxReference) -> Self {
        Self {
            state: ParseState::new(syntax),
            stack: ScopeStack::new(),
        }
    }

    fn line(&mut self, set: &SyntaxSet, text: &str) -> Vec<Span> {
        if text.len() > MAX_LINE_BYTES {
            return Vec::new();
        }
        let mut owned = String::with_capacity(text.len() + 1);
        owned.push_str(text);
        owned.push('\n');
        let Ok(ops) = self.state.parse_line(&owned, set) else {
            return Vec::new();
        };
        let mut spans: Vec<Span> = Vec::new();
        let mut push = |start: usize, end: usize, token: Option<Token>| {
            let end = end.min(text.len());
            let (Some(token), true) = (token, start < end) else {
                return;
            };
            let (Ok(start), Ok(end)) = (u32::try_from(start), u32::try_from(end)) else {
                return;
            };
            match spans.last_mut() {
                Some(last) if last.token == token && last.end == start => last.end = end,
                _ => spans.push(Span { start, end, token }),
            }
        };
        let mut pos = 0;
        for (at, op) in ops {
            if at > pos {
                push(pos, at, classify(&self.stack));
                pos = at;
            }
            if self.stack.apply(&op).is_err() {
                return Vec::new();
            }
        }
        if pos < text.len() {
            push(pos, text.len(), classify(&self.stack));
        }
        spans
    }
}

/// Highlights a diff, or `None` when its language is unknown or it is too large.
#[cfg(test)]
pub fn highlight(diff: &FileDiff) -> Option<Highlights> {
    highlight_until(diff, &|| false)
}

/// Lines parsed between checks for cancellation.
const CHECK_EVERY: usize = 256;

/// Highlights a diff, giving up (`None`) as soon as `cancelled` returns true.
fn highlight_until(diff: &FileDiff, cancelled: &dyn Fn() -> bool) -> Option<Highlights> {
    let total: usize = diff.hunks.iter().map(|h| h.lines.len()).sum();
    if total == 0 || total > MAX_LINES || diff.binary {
        return None;
    }
    let set = syntaxes();
    let first = diff
        .hunks
        .first()
        .and_then(|h| h.lines.first())
        .map(|l| l.text.as_str())
        .unwrap_or("");
    let syntax = syntax_for(set, &diff.path, first)?;
    let mut lines = Vec::with_capacity(diff.hunks.len());
    for hunk in &diff.hunks {
        // A fresh state per hunk: the text between hunks is unknown.
        let mut old = Side::new(syntax);
        let mut new = Side::new(syntax);
        let mut spans = Vec::with_capacity(hunk.lines.len());
        for (i, line) in hunk.lines.iter().enumerate() {
            if i % CHECK_EVERY == 0 && cancelled() {
                return None;
            }
            spans.push(match line.kind {
                LineKind::Removed => old.line(set, &line.text),
                LineKind::Added => new.line(set, &line.text),
                LineKind::Context => {
                    let s = new.line(set, &line.text);
                    // Keep the old side's parser in step with the shared line.
                    let _ = old.line(set, &line.text);
                    s
                }
            });
        }
        lines.push(spans);
    }
    Some(Highlights {
        lines,
        language: syntax.name.clone(),
    })
}

/// Highlights reused across requests for the same file revision (a refresh after staging, or
/// returning to a file), keyed by the diff's content.
#[derive(Debug, Default)]
pub struct Cache {
    entries: HashMap<u64, Arc<Highlights>>,
    order: Vec<u64>,
}

const CACHE_SIZE: usize = 48;

fn fingerprint(diff: &FileDiff) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    diff.path.hash(&mut h);
    for hunk in &diff.hunks {
        for line in &hunk.lines {
            (line.kind as u8).hash(&mut h);
            line.text.hash(&mut h);
        }
        0xFFu8.hash(&mut h);
    }
    h.finish()
}

impl Cache {
    fn lookup(&self, key: u64) -> Option<Arc<Highlights>> {
        self.entries.get(&key).cloned()
    }

    fn insert(&mut self, key: u64, h: Arc<Highlights>) {
        if self.entries.contains_key(&key) {
            return;
        }
        if self.order.len() >= CACHE_SIZE {
            let old = self.order.remove(0);
            self.entries.remove(&old);
        }
        self.order.push(key);
        self.entries.insert(key, h);
    }
}

/// A diff to highlight, and what to do with the result.
pub struct Request {
    pub diff: Arc<FileDiff>,
    /// True once the request no longer matters (a newer diff was asked for). Checked while
    /// parsing, so a superseded huge diff stops early.
    pub stale: Box<dyn Fn() -> bool + Send>,
    /// Receives the highlights. Not called when the language is unknown, the diff is too large
    /// or the request went stale.
    pub deliver: Box<dyn FnOnce(Arc<Highlights>) + Send>,
}

/// Highlighter threads shared by every tab: two, so one huge file does not hold up the others.
const THREADS: usize = 2;

fn pool() -> Option<&'static crossbeam_channel::Sender<Request>> {
    static POOL: OnceLock<Option<crossbeam_channel::Sender<Request>>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (jobs, rx) = crossbeam_channel::unbounded::<Request>();
        let cache = Arc::new(Mutex::new(Cache::default()));
        let mut started = 0;
        for i in 0..THREADS {
            let rx = rx.clone();
            let cache = Arc::clone(&cache);
            let spawned = std::thread::Builder::new()
                .name(format!("highlight-{i}"))
                .spawn(move || {
                    while let Ok(req) = rx.recv() {
                        run(req, &cache);
                    }
                });
            match spawned {
                Ok(_) => started += 1,
                Err(e) => tracing::warn!(error = %e, "cannot start a highlighter thread"),
            }
        }
        (started > 0).then_some(jobs)
    })
    .as_ref()
}

fn run(req: Request, cache: &Mutex<Cache>) {
    if (req.stale)() {
        return;
    }
    let key = fingerprint(&req.diff);
    let cached = cache.lock().ok().and_then(|c| c.lookup(key));
    let h = match cached {
        Some(h) => h,
        None => {
            let started = std::time::Instant::now();
            let Some(h) = highlight_until(&req.diff, &*req.stale) else {
                return;
            };
            tracing::debug!(
                lines = req.diff.hunks.iter().map(|h| h.lines.len()).sum::<usize>(),
                ms = started.elapsed().as_millis() as u64,
                "highlighted"
            );
            let h = Arc::new(h);
            if let Ok(mut c) = cache.lock() {
                c.insert(key, Arc::clone(&h));
            }
            h
        }
    };
    if !(req.stale)() {
        (req.deliver)(h);
    }
}

/// Queues a diff for highlighting on the shared highlighter threads, or highlights it on the
/// calling thread when they could not be started.
pub fn request(req: Request) {
    let req = match pool() {
        Some(jobs) => match jobs.send(req) {
            Ok(()) => return,
            Err(e) => e.0,
        },
        None => req,
    };
    run(req, &Mutex::new(Cache::default()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use canopy_core::diff::{DiffLine, Hunk};

    fn diff(path: &str, lines: &[(LineKind, &str)]) -> FileDiff {
        FileDiff {
            path: path.into(),
            hunks: vec![Hunk {
                header: "@@".into(),
                old_start: 1,
                old_lines: 1,
                new_start: 1,
                new_lines: 1,
                lines: lines
                    .iter()
                    .map(|(kind, text)| DiffLine {
                        kind: *kind,
                        old_no: None,
                        new_no: None,
                        text: (*text).into(),
                    })
                    .collect(),
            }],
            ..FileDiff::default()
        }
    }

    fn tokens(h: &Highlights, line: usize, text: &str) -> Vec<(String, Token)> {
        h.lines[0][line]
            .iter()
            .map(|s| (text[s.start as usize..s.end as usize].to_owned(), s.token))
            .collect()
    }

    #[test]
    fn highlights_common_languages() {
        let cases = [
            ("src/main.rs", "fn main() { let x = 42; } // done", "Rust"),
            ("app.ts", "const n: number = 1; // note", "TypeScript"),
            ("tool.py", "def run(): return \"ok\"  # note", "Python"),
            ("README.md", "# Title", "Markdown"),
        ];
        for (path, text, language) in cases {
            let h = highlight(&diff(path, &[(LineKind::Added, text)])).expect(path);
            assert_eq!(h.language, language);
            assert!(!h.lines[0][0].is_empty(), "{path} has spans");
        }
        let rust = "fn main() { let x = 42; } // done";
        let h = highlight(&diff("a.rs", &[(LineKind::Added, rust)])).expect("rust");
        let t = tokens(&h, 0, rust);
        assert!(t.contains(&("fn".into(), Token::Keyword)), "{t:?}");
        assert!(t.contains(&("42".into(), Token::Number)), "{t:?}");
        assert!(
            t.iter()
                .any(|(s, k)| *k == Token::Comment && s.contains("done")),
            "{t:?}"
        );
    }

    #[test]
    fn sides_keep_their_own_state() {
        // A block comment opened on the removed side must not swallow added lines.
        let d = diff(
            "x.rs",
            &[
                (LineKind::Removed, "/* old"),
                (LineKind::Added, "let a = 1;"),
                (LineKind::Removed, "end */"),
            ],
        );
        let h = highlight(&d).expect("rust");
        let added = tokens(&h, 1, "let a = 1;");
        assert!(added.contains(&("let".into(), Token::Keyword)), "{added:?}");
        let closing = tokens(&h, 2, "end */");
        assert!(
            closing.iter().all(|(_, k)| *k == Token::Comment),
            "{closing:?}"
        );
    }

    #[test]
    fn unknown_and_huge_files_are_plain() {
        assert!(highlight(&diff("notes.unknownext", &[(LineKind::Added, "hello")])).is_none());
        let long = "x".repeat(MAX_LINE_BYTES + 1);
        let h = highlight(&diff("a.rs", &[(LineKind::Added, &long)])).expect("rust");
        assert!(h.lines[0][0].is_empty());
    }

    #[test]
    fn cache_reuses_results() {
        let d = diff("a.rs", &[(LineKind::Added, "fn x() {}")]);
        let mut cache = Cache::default();
        let key = fingerprint(&d);
        assert!(cache.lookup(key).is_none());
        let h = Arc::new(highlight(&d).expect("rust"));
        cache.insert(key, Arc::clone(&h));
        assert!(Arc::ptr_eq(&cache.lookup(key).expect("cached"), &h));
        let other = diff("a.rs", &[(LineKind::Added, "fn y() {}")]);
        assert_ne!(fingerprint(&other), key);
    }

    #[test]
    fn every_hunk_starts_with_a_fresh_parser() {
        // The first hunk opens a block comment whose end lies outside the diff; the second hunk
        // is ordinary code and must not be coloured as a comment.
        let mut d = diff("x.rs", &[(LineKind::Added, "/* starts here")]);
        let mut second = d.hunks[0].clone();
        second.lines[0].text = "let a = 1;".into();
        d.hunks.push(second);
        let h = highlight(&d).expect("rust");
        let t: Vec<(String, Token)> = h.lines[1][0]
            .iter()
            .map(|s| {
                (
                    "let a = 1;"[s.start as usize..s.end as usize].to_owned(),
                    s.token,
                )
            })
            .collect();
        assert!(t.contains(&("let".into(), Token::Keyword)), "{t:?}");
        assert!(t.iter().all(|(_, k)| *k != Token::Comment), "{t:?}");
        // Within one hunk the comment still carries over to the next line.
        let d = diff(
            "x.rs",
            &[
                (LineKind::Added, "/* open"),
                (LineKind::Added, "let b = 2;"),
            ],
        );
        let h = highlight(&d).expect("rust");
        assert!(!h.lines[0][1].is_empty());
        assert!(h.lines[0][1].iter().all(|s| s.token == Token::Comment));
    }

    #[test]
    fn requests_deliver_off_thread_and_skip_stale_ones() {
        let d = Arc::new(diff("a.rs", &[(LineKind::Added, "fn x() {}")]));
        let (tx, rx) = crossbeam_channel::unbounded();
        let deliver = |tx: crossbeam_channel::Sender<(std::thread::ThreadId, String)>| {
            Box::new(move |h: Arc<Highlights>| {
                let _ = tx.send((std::thread::current().id(), h.language.clone()));
            })
        };
        request(Request {
            diff: Arc::clone(&d),
            stale: Box::new(|| true),
            deliver: deliver(tx.clone()),
        });
        request(Request {
            diff: d,
            stale: Box::new(|| false),
            deliver: deliver(tx),
        });
        let (thread, language) = rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("delivered");
        assert_eq!(language, "Rust");
        assert_ne!(thread, std::thread::current().id());
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "the stale request delivers nothing"
        );
    }

    #[test]
    fn cancelled_highlighting_stops() {
        let d = diff("a.rs", &[(LineKind::Added, "fn x() {}")]);
        assert!(highlight_until(&d, &|| true).is_none());
    }
}

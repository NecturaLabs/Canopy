//! Author pictures cached locally for twelve hours, including failed lookups.
//! All cache I/O, provider requests and image decoding run on the hosting thread.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::{
    Mutex, Once,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, SystemTime};

use canopy_core::Oid;
use egui::{ColorImage, TextureHandle};
use serde::{Deserialize, Serialize};

/// Pictures a tab keeps as textures; the least recently shown one makes room for a new author
/// (its record stays on disk, so it comes back without a request).
const MAX_PICTURES: usize = 256;
pub const CACHE_SECS: i64 = 12 * 60 * 60;
const HOUR_SECS: i64 = 60 * 60;
/// Provider lookups pictures may take an hour. Unauthenticated GitHub allows 60 API calls an hour
/// to the whole computer and pull requests need them too, so pictures take half; with a token
/// the limit is 5000.
const LOOKUPS_PER_HOUR_ANONYMOUS: u32 = 30;
const LOOKUPS_PER_HOUR_SIGNED_IN: u32 = 500;
/// A cache file nobody refreshed for this long belongs to an author not seen for a month.
const PRUNE_AFTER: Duration = Duration::from_hours(30 * 24);
const PIXEL_BYTES: usize = 64 * 64 * 4;
const MAX_RECORD_BYTES: u64 = 128 * 1024;
// Tabs may have separate picture threads. Serialize cache misses so they never fetch an author
// twice while the first request is still in flight. Only picture threads wait on it.
static CACHE_LOCK: Mutex<()> = Mutex::new(());
static BUDGET: Mutex<Budget> = Mutex::new(Budget { hour: 0, used: 0 });
static PRUNED: Once = Once::new();

/// How many provider lookups this hour has already used.
struct Budget {
    hour: i64,
    used: u32,
}

impl Budget {
    /// Takes one lookup, or says when the next hour begins.
    fn take(&mut self, now: i64, limit: u32) -> Result<(), i64> {
        let hour = now.div_euclid(HOUR_SECS);
        if hour != self.hour {
            *self = Self { hour, used: 0 };
        }
        if self.used >= limit {
            return Err((hour + 1) * HOUR_SECS);
        }
        self.used += 1;
        Ok(())
    }
}

/// How long lookups wait after the provider or picture host could not be reached.
const RETRY_UNREACHABLE_SECS: i64 = 10 * 60;
/// Commits of one author tried before giving up on them for the session, when the host has none
/// of them (they were never pushed there).
const MAX_UNKNOWN_COMMITS: u8 = 3;
/// Longest server message kept for a refusal, in characters.
const MAX_SERVER_TEXT: usize = 160;

/// Why a repository's pictures stopped, when the cause is the repository or the token rather
/// than any author. It holds until the tab's hosting thread restarts (a new token, a new remote,
/// the next start), and no author is marked as having no picture because of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocked {
    /// HTTP 404 for the repository itself. GitHub: private, and the token (or its absence) cannot
    /// see it. GitLab: the server has no picture lookup.
    NotFound,
    /// HTTP 401, or a 403 that is not a rate limit, with the server's message as plain text
    /// (`server_text`).
    Refused(String),
    /// A redirect: the repository was renamed or moved, so the remote URL is out of date.
    Moved,
}

/// What a finished lookup says about the repository rather than the author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// The host does not have the commit asked about; another commit by the author may work.
    UnknownCommit,
    Blocked(Blocked),
    /// The provider or the picture host could not be reached, or answered with a server error.
    Unreachable,
}

/// Why lookups are waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pause {
    /// The provider's request limit, or the share of it pictures may use, is spent.
    Limit,
    Unreachable,
}

/// Where a tab's pictures stand, for telling the user why authors show initials.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "views adopt it: contracts/avatars.md")
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status<'a> {
    /// Lookups run when an author is due (or nothing has been asked yet).
    Working,
    Blocked(&'a Blocked),
    /// No lookup before `until` (unix seconds).
    Paused {
        why: Pause,
        until: i64,
    },
}

/// What decides whether the active repository's authors can have pictures.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "views adopt it: contracts/avatars.md")
)]
#[derive(Debug, Clone, Copy)]
pub struct Situation<'a> {
    /// Settings → Appearance → Author avatars. Off, no avatar is drawn and there is nothing to
    /// explain.
    pub show_avatars: bool,
    /// Settings → Privacy → Download profile pictures.
    pub download: bool,
    /// Settings → Integrations → Pull and merge requests (pictures use the same API).
    pub hosting_api: bool,
    /// The Terms, the Privacy Policy and the crash question are answered.
    pub consented: bool,
    /// The provider of the repository's main GitHub or GitLab remote, if it has one.
    pub forge: Option<canopy_core::forge::ForgeKind>,
    /// Whether the hosting thread found a token; `None` until it has said.
    pub token: Option<bool>,
    pub status: Status<'a>,
}

/// Where in Settings the user can change what keeps pictures away.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "views adopt it: contracts/avatars.md")
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    /// Add or replace the provider's token.
    Accounts,
    /// Turn Pull and merge requests on.
    Integrations,
    /// Turn Download profile pictures on.
    Privacy,
}

/// Why authors show initials instead of pictures, in words for the user.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "views adopt it: contracts/avatars.md")
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    /// A few words, for a tooltip's first line or a row label.
    pub title: &'static str,
    /// One or two sentences: the cause and what to do.
    pub detail: String,
    pub fix: Option<Fix>,
}

/// Why the active repository's authors show initials, or `None` when nothing keeps pictures
/// away (they are loading, cached or the authors have none) or there is nothing to tell.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the views adopt it through contracts/avatars.md")
)]
pub fn explain(s: &Situation<'_>, now: i64) -> Option<Explanation> {
    use canopy_core::forge::ForgeKind;
    if !s.show_avatars || !s.consented {
        return None;
    }
    let off = |detail: &str, fix| {
        Some(Explanation {
            title: "Profile pictures are off",
            detail: detail.to_owned(),
            fix: Some(fix),
        })
    };
    if !s.hosting_api {
        return off(
            "Pictures come from the repository's GitHub or GitLab API, which is off in Settings → Integrations.",
            Fix::Integrations,
        );
    }
    if !s.download {
        return off(
            "Download profile pictures is off in Settings → Privacy.",
            Fix::Privacy,
        );
    }
    let Some(kind) = s.forge else {
        return Some(Explanation {
            title: "No GitHub or GitLab remote",
            detail:
                "Pictures come from GitHub or GitLab, and this repository has no remote on either."
                    .into(),
            fix: None,
        });
    };
    let provider = kind.name();
    let token = s.token == Some(true);
    match s.status {
        Status::Working => None,
        Status::Blocked(Blocked::NotFound) if kind == ForgeKind::GitLab => Some(Explanation {
            title: "No picture lookup on this server",
            detail: "This GitLab server does not answer profile picture lookups.".into(),
            fix: None,
        }),
        Status::Blocked(Blocked::NotFound) if token => Some(Explanation {
            title: "Token can't see this repository",
            detail: format!(
                "The {provider} token Canopy uses cannot see this repository. Use a token with access to it in Settings → Accounts."
            ),
            fix: Some(Fix::Accounts),
        }),
        Status::Blocked(Blocked::NotFound) => Some(Explanation {
            title: "Private repository",
            detail: format!(
                "{provider} shows who wrote these commits only to an account that can see this repository. Add a {provider} token in Settings → Accounts."
            ),
            fix: Some(Fix::Accounts),
        }),
        Status::Blocked(Blocked::Refused(message)) if !token => Some(Explanation {
            title: "Sign-in required",
            detail: format!(
                "{provider} answered: {}. Add a {provider} token in Settings → Accounts.",
                message.trim_end_matches('.')
            ),
            fix: Some(Fix::Accounts),
        }),
        Status::Blocked(Blocked::Refused(message)) => Some(Explanation {
            title: "Token refused",
            detail: format!(
                "{provider} refused the token: {}. Replace it in Settings → Accounts.",
                message.trim_end_matches('.')
            ),
            fix: Some(Fix::Accounts),
        }),
        Status::Blocked(Blocked::Moved) => Some(Explanation {
            title: "Repository moved",
            detail: format!(
                "{provider} says this repository was renamed or moved. Update the remote's URL to show pictures."
            ),
            fix: None,
        }),
        Status::Paused { why, until } => {
            let minutes = (until - now).max(60).div_euclid(60);
            let when = if minutes <= 1 {
                "in a minute".to_owned()
            } else {
                format!("in {minutes} minutes")
            };
            Some(match why {
                Pause::Unreachable => Explanation {
                    title: "Can't reach the picture service",
                    detail: format!("Canopy will try again {when}."),
                    fix: None,
                },
                Pause::Limit if !token => Explanation {
                    title: "Picture lookups paused",
                    detail: format!(
                        "{provider}'s hourly request limit for pictures is used up; lookups resume {when}. A {provider} token in Settings → Accounts raises the limit."
                    ),
                    fix: Some(Fix::Accounts),
                },
                Pause::Limit => Explanation {
                    title: "Picture lookups paused",
                    detail: format!(
                        "{provider}'s hourly request limit for pictures is used up; lookups resume {when}."
                    ),
                    fix: None,
                },
            })
        }
    }
}

#[derive(Default)]
pub struct Avatars {
    pub enabled: bool,
    pictures: HashMap<String, Picture>,
    pending: Option<(Oid, String)>,
    retry_after: i64,
    paused: Option<Pause>,
    blocked: Option<Blocked>,
    /// Commits the host does not have, and how many of each author's were tried.
    unknown_commits: std::collections::HashSet<Oid>,
    unknown_by_author: HashMap<String, u8>,
    /// Counts uses, to find the least recently shown picture.
    clock: Cell<u64>,
}

struct Picture {
    texture: Option<TextureHandle>,
    checked_at: i64,
    last_used: Cell<u64>,
}

impl std::fmt::Debug for Avatars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Avatars")
            .field("pictures", &self.pictures.len())
            .finish_non_exhaustive()
    }
}

impl Avatars {
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled && !enabled {
            *self = Self::default();
        }
        self.enabled = enabled;
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn get(&self, email: &str) -> Option<&TextureHandle> {
        if !self.enabled {
            return None;
        }
        let picture = self.pictures.get(email)?;
        picture.last_used.set(self.tick());
        picture.texture.as_ref()
    }

    fn tick(&self) -> u64 {
        self.clock.set(self.clock.get() + 1);
        self.clock.get()
    }

    /// Whether `request` would start a lookup for `email` now; cheap enough to ask for every
    /// visible row on every frame.
    pub fn wants_now(&self, email: &str) -> bool {
        self.wants(email, chrono::Utc::now().timestamp())
    }

    fn wants(&self, email: &str, now: i64) -> bool {
        self.enabled
            && self.blocked.is_none()
            && now >= self.retry_after
            && self.pending.is_none()
            && self
                .unknown_by_author
                .get(email)
                .is_none_or(|tried| *tried < MAX_UNKNOWN_COMMITS)
            && !self
                .pictures
                .get(email)
                .is_some_and(|p| fresh(p.checked_at, now))
    }

    /// Where this tab's pictures stand, for explaining missing ones ([`explain`]).
    #[expect(dead_code, reason = "the views adopt it through contracts/avatars.md")]
    pub fn status(&self) -> Status<'_> {
        self.status_at(chrono::Utc::now().timestamp())
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "views adopt it: contracts/avatars.md")
    )]
    fn status_at(&self, now: i64) -> Status<'_> {
        if let Some(blocked) = &self.blocked {
            return Status::Blocked(blocked);
        }
        match self.paused {
            Some(why) if now < self.retry_after => Status::Paused {
                why,
                until: self.retry_after,
            },
            _ => Status::Working,
        }
    }

    pub fn request(&mut self, oid: Oid, email: &str) -> bool {
        self.request_at(oid, email, chrono::Utc::now().timestamp())
    }

    fn request_at(&mut self, oid: Oid, email: &str, now: i64) -> bool {
        if !self.wants(email, now) || self.unknown_commits.contains(&oid) {
            return false;
        }
        self.pending = Some((oid, email.to_owned()));
        true
    }

    pub fn finish(&mut self, ctx: &egui::Context, oid: Oid, email: String, result: Cached) {
        if self.pending.as_ref() != Some(&(oid, email.clone())) || !self.enabled {
            return;
        }
        self.pending = None;
        if let Some(at) = result.retry_at {
            self.retry_after = at;
        }
        self.paused = result.retry_at.map(|_| {
            if result.note == Some(Note::Unreachable) {
                Pause::Unreachable
            } else {
                Pause::Limit
            }
        });
        match result.note {
            Some(Note::UnknownCommit) => {
                self.unknown_commits.insert(oid);
                let tried = self.unknown_by_author.entry(email.clone()).or_default();
                *tried = tried.saturating_add(1);
            }
            Some(Note::Blocked(blocked)) => self.blocked = Some(blocked),
            Some(Note::Unreachable) | None => {}
        }
        if !self.pictures.contains_key(&email) && self.pictures.len() >= MAX_PICTURES {
            let oldest = self
                .pictures
                .iter()
                .min_by_key(|(_, p)| p.last_used.get())
                .map(|(email, _)| email.clone());
            if let Some(oldest) = oldest {
                self.pictures.remove(&oldest);
            }
        }
        let used = self.tick();
        let entry = self.pictures.entry(email).or_insert(Picture {
            texture: None,
            checked_at: 0,
            last_used: Cell::new(used),
        });
        entry.checked_at = result.checked_at;
        if let Some(image) = result.image {
            entry.texture = Some(ctx.load_texture(
                format!("author-picture-{oid}"),
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
    }
}

fn fresh(checked_at: i64, now: i64) -> bool {
    checked_at <= now && now.saturating_sub(checked_at) < CACHE_SECS
}

#[derive(Serialize, Deserialize, Default)]
struct Record {
    checked_at: i64,
    // Store the bounded premultiplied RGBA thumbnail, not an untrusted compressed image or an author email.
    pixels: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct Cached {
    pub image: Option<ColorImage>,
    pub checked_at: i64,
    /// Do not ask a provider again before this time (unix seconds): its limit, or the share of
    /// it pictures may use, is spent, or it could not be reached.
    pub retry_at: Option<i64>,
    /// What the lookup found out about the repository rather than the author.
    pub note: Option<Note>,
}

/// The answer for a lookup that could not even be handed to a picture thread.
pub fn unavailable(now: i64) -> Cached {
    Cached {
        image: None,
        checked_at: 0,
        retry_at: Some(now + HOUR_SECS),
        note: None,
    }
}

fn read_record(path: &Path) -> Option<Record> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return None;
    }
    let record: Record = serde_json::from_slice(&bytes).ok()?;
    if record
        .pixels
        .as_ref()
        .is_some_and(|p| p.len() != PIXEL_BYTES)
    {
        return None;
    }
    Some(record)
}

fn write_record(path: &Path, record: &Record) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("new");
    crate::hosting::write_private(&tmp, &bytes)?;
    std::fs::rename(tmp, path)
}

/// Why a refresh produced no picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Miss {
    /// Nothing was asked: pictures were switched off, or the tab went to the background.
    Skipped,
    /// The answer was about the repository or the commit, not the author ([`Note`]): the record
    /// is left as it was.
    Kept,
    Failed,
    /// The provider's limit, or the share of it pictures may use, is spent until this time. The
    /// author is not at fault, so the record is left as it was and stays due.
    Later(i64),
}

/// Reads the cache and, only when due and still enabled, asks the provider once.
pub fn load(
    client: &canopy_hosting::Client,
    forge: &canopy_core::forge::Forge,
    oid: Oid,
    email: &str,
    enabled: &AtomicBool,
) -> Cached {
    let dir = eframe::storage_dir("canopy").map(|p| p.join("avatars"));
    if let Some(dir) = &dir {
        PRUNED.call_once(|| {
            prune(dir, SystemTime::now(), PRUNE_AFTER);
        });
    }
    let path = dir.map(|dir| {
        dir.join(format!(
            "{}.json",
            canopy_hosting::avatars::cache_key(forge, email)
        ))
    });
    load_at(path, client, oid, email, enabled)
}

/// Deletes the cache files nobody has refreshed for `max_age`. Returns how many went.
fn prune(dir: &Path, now: SystemTime, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > max_age)
        })
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

fn load_at(
    path: Option<std::path::PathBuf>,
    client: &canopy_hosting::Client,
    oid: Oid,
    email: &str,
    enabled: &AtomicBool,
) -> Cached {
    let _lock = CACHE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = chrono::Utc::now().timestamp();
    let old = path.as_deref().and_then(read_record);
    let due = old.as_ref().is_none_or(|r| !fresh(r.checked_at, now));
    let mut note = None;
    let (record, retry_at) = refresh(old, now, || {
        let limit = if client.has_token() {
            LOOKUPS_PER_HOUR_SIGNED_IN
        } else {
            LOOKUPS_PER_HOUR_ANONYMOUS
        };
        let mut take = || {
            BUDGET
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(now, limit)
        };
        fetch(client, oid, email, enabled, now, &mut take, &mut note)
    });
    if due
        && record.checked_at == now
        && enabled.load(Ordering::SeqCst)
        && let Some(path) = path
        && let Err(e) = write_record(&path, &record)
    {
        tracing::warn!(error = %e, "could not save the author-picture cache");
    }
    Cached {
        image: record
            .pixels
            .map(|pixels| ColorImage::from_rgba_premultiplied([64, 64], &pixels)),
        checked_at: record.checked_at,
        retry_at,
        note,
    }
}

/// Asks the provider about one author and downloads the picture. Before each request it checks
/// that pictures are still on and takes one lookup from the hourly budget (`take`; the download
/// is not a provider request). What it learns about the repository goes into `note`.
#[expect(
    clippy::map_err_ignore,
    reason = "a failed picture is cached as a miss; why it failed is not kept"
)]
fn fetch(
    client: &canopy_hosting::Client,
    oid: Oid,
    email: &str,
    enabled: &AtomicBool,
    now: i64,
    take: &mut dyn FnMut() -> Result<(), i64>,
    note: &mut Option<Note>,
) -> Result<Vec<u8>, Miss> {
    use canopy_hosting::Error;
    let mut may_ask = || {
        if !enabled.load(Ordering::SeqCst) {
            return Err(Miss::Skipped);
        }
        take().map_err(Miss::Later)
    };
    may_ask()?;
    let avatar = match client.author_avatar(&oid.to_string(), email) {
        Ok(avatar) => avatar,
        // GitHub answers 404 both for a commit it does not have and for a repository it will not
        // show; only the repository itself tells them apart.
        Err(Error::NotFound(_)) if client.kind() == canopy_core::forge::ForgeKind::GitHub => {
            may_ask()?;
            return Err(match client.author_repository() {
                Ok(()) => {
                    *note = Some(Note::UnknownCommit);
                    Miss::Kept
                }
                Err(e) => miss(e, now, note),
            });
        }
        Err(e) => return Err(miss(e, now, note)),
    };
    if !enabled.load(Ordering::SeqCst) {
        return Err(Miss::Skipped);
    }
    let Some(avatar) = avatar else {
        return Err(Miss::Failed);
    };
    let bytes = avatar.download().map_err(|e| match e {
        Error::Network(_) => pause_unreachable(now, note),
        Error::Api { status, .. } if status >= 500 => pause_unreachable(now, note),
        _ => Miss::Failed,
    })?;
    let image = decode(&bytes).map_err(|_| Miss::Failed)?;
    Ok(image
        .pixels
        .iter()
        .flat_map(egui::Color32::to_array)
        .collect())
}

/// What a failed provider request means for the author's record and the tab.
fn miss(error: canopy_hosting::Error, now: i64, note: &mut Option<Note>) -> Miss {
    use canopy_hosting::Error;
    let mut kept = |about| {
        *note = Some(about);
        Miss::Kept
    };
    match error {
        Error::RateLimited { reset } => {
            Miss::Later(reset.map_or(now + HOUR_SECS, |at| at.clamp(now + 60, now + CACHE_SECS)))
        }
        Error::Network(_) => pause_unreachable(now, note),
        Error::Api { status, .. } if status >= 500 => pause_unreachable(now, note),
        e if canopy_hosting::avatars::is_unknown_commit(&e) => kept(Note::UnknownCommit),
        Error::NotFound(_) => kept(Note::Blocked(Blocked::NotFound)),
        Error::Unauthorized(message) => {
            kept(Note::Blocked(Blocked::Refused(server_text(&message))))
        }
        Error::Api { status, .. } if (300..400).contains(&status) => {
            kept(Note::Blocked(Blocked::Moved))
        }
        Error::Api { .. } | Error::Parse(_) => Miss::Failed,
    }
}

/// A server's own words made safe to show among Canopy's and to log: control characters become
/// spaces, characters that reorder or hide text (bidirectional controls, zero-width and other
/// invisible formatting) are dropped, whitespace collapses, and the length is capped.
fn server_text(message: &str) -> String {
    let hides = |c: char| {
        matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E0FFF}'
        )
    };
    let visible: String = message
        .chars()
        .filter(|c| !hides(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let words = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.is_empty() {
        return "no reason given".to_owned();
    }
    if words.chars().count() <= MAX_SERVER_TEXT {
        return words;
    }
    let mut short: String = words.chars().take(MAX_SERVER_TEXT).collect();
    short.truncate(short.trim_end().len());
    short.push('…');
    short
}

fn pause_unreachable(now: i64, note: &mut Option<Note>) -> Miss {
    *note = Some(Note::Unreachable);
    Miss::Later(now + RETRY_UNREACHABLE_SECS)
}

/// The record after one due refresh, and when to ask a provider again if it was out of requests
/// or unreachable. A skipped, kept or postponed refresh changes nothing, so the author stays due.
fn refresh(
    old: Option<Record>,
    now: i64,
    fetch: impl FnOnce() -> Result<Vec<u8>, Miss>,
) -> (Record, Option<i64>) {
    if old.as_ref().is_some_and(|r| fresh(r.checked_at, now)) {
        return (old.unwrap_or_default(), None);
    }
    let mut record = old.unwrap_or_default();
    match fetch() {
        Ok(pixels) => {
            record.pixels = Some(pixels);
            record.checked_at = now;
            (record, None)
        }
        Err(Miss::Skipped | Miss::Kept) => (record, None),
        Err(Miss::Later(at)) => (record, Some(at)),
        Err(Miss::Failed) => {
            record.checked_at = now;
            (record, None)
        }
    }
}

pub fn decode(bytes: &[u8]) -> Result<ColorImage, String> {
    if bytes.len() as u64 > canopy_hosting::avatars::MAX_AVATAR_BYTES {
        return Err("Profile picture exceeds 2 MiB".into());
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
    ) {
        return Err("Profile picture is not PNG or JPEG".into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| e.to_string())?;
    let rgba = decoded
        .resize_to_fill(64, 64, image::imageops::FilterType::Triangle)
        .to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([64, 64], rgba.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_pictures_and_failures_are_requested_once_every_twelve_hours() {
        let pixels = vec![42; PIXEL_BYTES];
        let (record, _) = refresh(None, 100, || Ok(pixels.clone()));
        let (record, _) = refresh(Some(record), 100 + CACHE_SECS - 1, || {
            panic!("fresh cache fetched again")
        });
        let (record, _) = refresh(Some(record), 100 + CACHE_SECS, || Err(Miss::Failed));
        assert_eq!(record.pixels, Some(pixels));
        let (_, _) = refresh(Some(record), 100 + 2 * CACHE_SECS - 1, || {
            panic!("failed refresh retried early")
        });
        let (missing, _) = refresh(None, 100, || Err(Miss::Failed));
        assert!(missing.pixels.is_none());
        let (missing, _) = refresh(Some(missing), 100 + CACHE_SECS - 1, || {
            panic!("negative cache fetched again")
        });
        let (record, _) = refresh(Some(missing), 100 + CACHE_SECS, || Ok(vec![1; PIXEL_BYTES]));
        assert!(record.pixels.is_some());
    }
    #[test]
    fn disk_cache_survives_reopening_and_rejects_malformed_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.json");
        write_record(
            &path,
            &Record {
                checked_at: 100,
                pixels: Some(vec![42; PIXEL_BYTES]),
            },
        )
        .unwrap();
        let old = read_record(&path).unwrap();
        let (record, _) = refresh(Some(old), 101, || panic!("reopening fetched again"));
        assert_eq!(record.checked_at, 100);
        std::fs::write(&path, br#"{"checked_at":100,"pixels":[1]}"#).unwrap();
        assert!(read_record(&path).is_none());
        assert!(!fresh(101, 100));
    }
    #[test]
    fn disabling_clears_pending_work_and_due_time_is_per_author() {
        let mut pictures = Avatars::default();
        let oid = Oid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        assert!(!pictures.request_at(oid, "a@example.com", 100));
        pictures.set_enabled(true);
        assert!(pictures.request_at(oid, "a@example.com", 100));
        pictures.finish(
            &egui::Context::default(),
            oid,
            "a@example.com".into(),
            Cached {
                image: None,
                checked_at: 100,
                retry_at: None,
                note: None,
            },
        );
        assert!(!pictures.request_at(oid, "a@example.com", 100 + CACHE_SECS - 1));
        assert!(pictures.request_at(oid, "a@example.com", 100 + CACHE_SECS));
        pictures.set_enabled(false);
        assert!(!pictures.pending());
    }
    #[test]
    fn a_lookup_skipped_while_pictures_are_off_does_not_mark_the_author_checked() {
        let forge = canopy_core::forge::Forge::parse("https://github.com/example/repo", &[])
            .expect("github forge");
        let client = canopy_hosting::Client::new(&forge, None);
        let oid = Oid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.json");
        let skipped = load_at(
            Some(path.clone()),
            &client,
            oid,
            "skipped-lookup@example.invalid",
            &AtomicBool::new(false),
        );
        assert_eq!(
            skipped.checked_at, 0,
            "no request was made, so nothing is checked"
        );
        assert!(skipped.image.is_none() && skipped.retry_at.is_none());
        assert!(!path.exists(), "a skipped lookup stores no negative record");

        // A stale picture stays due, and visible, until a request really happens.
        let (record, _) = refresh(
            Some(Record {
                checked_at: 100,
                pixels: Some(vec![7; PIXEL_BYTES]),
            }),
            100 + CACHE_SECS,
            || Err(Miss::Skipped),
        );
        assert_eq!(record.checked_at, 100);
        assert!(record.pixels.is_some());
        assert!(!fresh(record.checked_at, 100 + CACHE_SECS));
    }

    #[test]
    fn pictures_leave_half_of_the_anonymous_hourly_quota_to_pull_requests() {
        const { assert!(LOOKUPS_PER_HOUR_ANONYMOUS <= 30) };
        let mut budget = Budget { hour: 0, used: 0 };
        let hour = 5 * HOUR_SECS;
        for _ in 0..LOOKUPS_PER_HOUR_ANONYMOUS {
            assert_eq!(budget.take(hour + 10, LOOKUPS_PER_HOUR_ANONYMOUS), Ok(()));
        }
        // The next lookup waits for the next hour, and then the budget starts again.
        assert_eq!(
            budget.take(hour + 3000, LOOKUPS_PER_HOUR_ANONYMOUS),
            Err(6 * HOUR_SECS)
        );
        assert_eq!(
            budget.take(6 * HOUR_SECS, LOOKUPS_PER_HOUR_ANONYMOUS),
            Ok(())
        );
    }

    #[test]
    fn a_postponed_lookup_keeps_the_record_due_and_holds_every_request_back() {
        let old = Record {
            checked_at: 100,
            pixels: Some(vec![9; PIXEL_BYTES]),
        };
        let (record, retry_at) = refresh(Some(old), 100 + CACHE_SECS, || Err(Miss::Later(5000)));
        assert_eq!(retry_at, Some(5000));
        assert_eq!(record.checked_at, 100, "the author is not marked checked");
        assert!(record.pixels.is_some());

        let ctx = egui::Context::default();
        let mut pictures = Avatars::default();
        pictures.set_enabled(true);
        let oid = Oid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        let now = 1_800_000_000;
        assert!(pictures.request_at(oid, "a@example.com", now));
        pictures.finish(
            &ctx,
            oid,
            "a@example.com".into(),
            Cached {
                image: None,
                checked_at: 0,
                retry_at: Some(now + 5000),
                note: None,
            },
        );
        assert!(!pictures.wants("a@example.com", now + 4999));
        assert!(pictures.wants("a@example.com", now + 5000));
    }

    #[test]
    fn past_the_cap_the_least_recently_shown_picture_makes_room() {
        let ctx = egui::Context::default();
        let mut pictures = Avatars::default();
        pictures.set_enabled(true);
        let oid = Oid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        let load = |pictures: &mut Avatars, email: &str| {
            assert!(pictures.request_at(oid, email, 100));
            pictures.finish(
                &ctx,
                oid,
                email.to_owned(),
                Cached {
                    image: None,
                    checked_at: 100,
                    retry_at: None,
                    note: None,
                },
            );
        };
        for i in 0..MAX_PICTURES {
            load(&mut pictures, &format!("{i}@example.com"));
        }
        // Showing the first author again makes the second the one that has waited longest.
        pictures.get("0@example.com");
        load(&mut pictures, "new@example.com");
        assert_eq!(pictures.pictures.len(), MAX_PICTURES);
        assert!(pictures.pictures.contains_key("0@example.com"));
        assert!(pictures.pictures.contains_key("new@example.com"));
        assert!(!pictures.pictures.contains_key("1@example.com"));
    }

    #[test]
    fn rows_ask_for_a_lookup_only_while_one_is_due() {
        let mut pictures = Avatars::default();
        assert!(!pictures.wants("a@example.com", 100), "pictures are off");
        pictures.set_enabled(true);
        assert!(pictures.wants("a@example.com", 100));
        let oid = Oid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
        assert!(pictures.request_at(oid, "a@example.com", 100));
        assert!(
            !pictures.wants("b@example.com", 100),
            "one lookup at a time"
        );
    }

    #[test]
    fn cache_files_nobody_refreshed_for_a_month_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let age = |name: &str, days: u64| {
            let path = dir.path().join(name);
            std::fs::write(&path, b"{}").unwrap();
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(now - Duration::from_secs(days * 24 * 60 * 60))
                .unwrap();
        };
        age("old.json", 31);
        age("recent.json", 29);
        assert_eq!(prune(dir.path(), now, PRUNE_AFTER), 1);
        assert!(!dir.path().join("old.json").exists());
        assert!(dir.path().join("recent.json").exists());
        assert_eq!(prune(&dir.path().join("missing"), now, PRUNE_AFTER), 0);
    }

    #[test]
    fn png_and_jpeg_decode_to_bounded_pictures_and_other_input_is_refused() {
        let image = image::DynamicImage::new_rgb8(80, 100);
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, format).unwrap();
            assert_eq!(decode(bytes.get_ref()).unwrap().size, [64, 64]);
        }
        decode(b"not a picture").unwrap_err();
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2049, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        decode(bytes.get_ref()).unwrap_err();
        decode(&vec![0; 2 * 1024 * 1024 + 1]).unwrap_err();
    }

    /// A stand-in for GitHub's API and picture host on 127.0.0.1: answers each request from a
    /// table keyed by the exact target, else by its path, else 404 like GitHub.
    struct Stub {
        base: String,
        routes: Routes,
        /// Every request target, in order.
        seen: std::sync::Arc<Mutex<Vec<String>>>,
        /// A path whose request turns the switch off before it is answered, as a user can while
        /// a lookup is in flight.
        switch_off: Switch,
    }

    /// Canned answers by request target: status and body.
    type Routes = std::sync::Arc<Mutex<HashMap<String, (u16, Vec<u8>)>>>;
    type Switch = std::sync::Arc<Mutex<Option<(String, std::sync::Arc<AtomicBool>)>>>;

    impl Stub {
        fn start() -> Self {
            use std::io::{BufRead, BufReader, Write};
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let routes = Routes::default();
            let table = std::sync::Arc::clone(&routes);
            let seen = std::sync::Arc::<Mutex<Vec<String>>>::default();
            let log = std::sync::Arc::clone(&seen);
            let switch_off = Switch::default();
            let switch = std::sync::Arc::clone(&switch_off);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut reader = BufReader::new(&stream);
                    let mut line = String::new();
                    let _ = reader.read_line(&mut line);
                    let target = line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_owned();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0
                            || header.trim().is_empty()
                        {
                            break;
                        }
                    }
                    let path = target.split('?').next().unwrap_or_default();
                    log.lock().unwrap().push(target.clone());
                    if let Some((at, on)) = &*switch.lock().unwrap()
                        && at == path
                    {
                        on.store(false, Ordering::SeqCst);
                    }
                    let (status, body) = {
                        let table = table.lock().unwrap();
                        table
                            .get(&target)
                            .or_else(|| table.get(path))
                            .cloned()
                            .unwrap_or((404, br#"{"message":"Not Found"}"#.to_vec()))
                    };
                    let mut out = &stream;
                    let _ = write!(
                        out,
                        "HTTP/1.1 {status} Stub\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = out.write_all(&body);
                }
            });
            Self {
                base,
                routes,
                seen,
                switch_off,
            }
        }

        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }

        fn route(&self, target: &str, status: u16, body: impl Into<Vec<u8>>) {
            self.routes
                .lock()
                .unwrap()
                .insert(target.to_owned(), (status, body.into()));
        }

        fn client(&self, token: Option<&str>) -> canopy_hosting::Client {
            let forge = canopy_core::forge::Forge::parse("https://github.com/octo/hello", &[])
                .expect("github forge");
            canopy_hosting::Client::with_api_base(&forge, token.map(str::to_owned), &self.base)
        }

        /// Serves a real PNG at `/picture.png` as `sha`'s author picture.
        fn picture_for(&self, sha: &str) {
            let mut png = Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(8, 8)
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            self.route("/picture.png", 200, png.into_inner());
            let commits = serde_json::json!([{
                "sha": sha,
                "author": {"avatar_url": format!("{}/picture.png", self.base)},
            }]);
            self.route(
                &format!("/repos/octo/hello/commits?sha={sha}&per_page=1"),
                200,
                commits.to_string(),
            );
        }
    }

    const PUSHED: &str = "1111111111111111111111111111111111111111";
    const UNPUSHED: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn a_repository_hidden_without_a_token_marks_no_author_and_a_token_brings_pictures() {
        let stub = Stub::start();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.json");
        let oid = Oid::from_str(PUSHED).unwrap();
        let on = AtomicBool::new(true);
        // A private repository without a token: GitHub answers 404 for the repository itself.
        let hidden = load_at(
            Some(path.clone()),
            &stub.client(None),
            oid,
            "a@example.com",
            &on,
        );
        assert!(hidden.image.is_none());
        assert!(
            !path.exists(),
            "the author is not at fault, so no 12-hour negative record is written"
        );

        // With a token that can see it, the same author's picture arrives at once.
        stub.route("/repos/octo/hello", 200, "{}");
        stub.picture_for(PUSHED);
        let shown = load_at(
            Some(path),
            &stub.client(Some("t")),
            oid,
            "a@example.com",
            &on,
        );
        assert!(shown.image.is_some());
    }

    #[test]
    fn a_commit_not_pushed_yet_does_not_hide_its_authors_picture() {
        let stub = Stub::start();
        stub.route("/repos/octo/hello", 200, "{}");
        stub.picture_for(PUSHED);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.json");
        let on = AtomicBool::new(true);
        let client = stub.client(Some("t"));
        let unpushed = load_at(
            Some(path.clone()),
            &client,
            Oid::from_str(UNPUSHED).unwrap(),
            "me@example.com",
            &on,
        );
        assert!(unpushed.image.is_none());
        assert!(
            !path.exists(),
            "an unknown commit says nothing about its author"
        );
        let pushed = load_at(
            Some(path),
            &client,
            Oid::from_str(PUSHED).unwrap(),
            "me@example.com",
            &on,
        );
        assert!(
            pushed.image.is_some(),
            "the author's pushed commit finds the picture"
        );
    }

    #[test]
    fn being_offline_postpones_lookups_instead_of_caching_a_failure() {
        // A port nothing listens on: the connection is refused.
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", listener.local_addr().unwrap())
        };
        let forge = canopy_core::forge::Forge::parse("https://github.com/octo/hello", &[])
            .expect("github forge");
        let client = canopy_hosting::Client::with_api_base(&forge, Some("t".into()), &closed);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.json");
        let offline = load_at(
            Some(path.clone()),
            &client,
            Oid::from_str(PUSHED).unwrap(),
            "a@example.com",
            &AtomicBool::new(true),
        );
        assert!(offline.image.is_none());
        assert!(
            !path.exists(),
            "nothing was looked up, so nothing is cached"
        );
        assert!(
            offline.retry_at.is_some(),
            "the tab waits before it asks again instead of asking every frame"
        );
        assert_eq!(offline.note, Some(Note::Unreachable));
    }

    #[test]
    fn the_lookup_says_whether_the_repository_or_the_commit_is_at_fault() {
        let stub = Stub::start();
        let on = AtomicBool::new(true);
        let dir = tempfile::tempdir().unwrap();
        let oid = Oid::from_str(UNPUSHED).unwrap();
        let note = |client: &canopy_hosting::Client| {
            load_at(
                Some(dir.path().join("p.json")),
                client,
                oid,
                "a@example.com",
                &on,
            )
            .note
        };
        assert_eq!(
            note(&stub.client(None)),
            Some(Note::Blocked(Blocked::NotFound))
        );
        stub.route(
            "/repos/octo/hello/commits",
            401,
            r#"{"message":"Bad credentials"}"#,
        );
        assert_eq!(
            note(&stub.client(Some("t"))),
            Some(Note::Blocked(Blocked::Refused("Bad credentials".into())))
        );
        stub.route(
            "/repos/octo/hello/commits",
            301,
            r#"{"message":"Moved Permanently"}"#,
        );
        assert_eq!(
            note(&stub.client(Some("t"))),
            Some(Note::Blocked(Blocked::Moved))
        );
        stub.route("/repos/octo/hello/commits", 502, "Bad gateway");
        assert_eq!(note(&stub.client(Some("t"))), Some(Note::Unreachable));
        stub.route(
            "/repos/octo/hello/commits",
            404,
            r#"{"message":"Not Found"}"#,
        );
        stub.route("/repos/octo/hello", 200, "{}");
        assert_eq!(note(&stub.client(Some("t"))), Some(Note::UnknownCommit));
        // An author without an account is the author's own answer: cached, with no note.
        stub.route(
            "/repos/octo/hello/commits",
            200,
            format!(r#"[{{"sha":"{UNPUSHED}","author":null}}]"#),
        );
        let path = dir.path().join("p.json");
        let none = load_at(
            Some(path.clone()),
            &stub.client(Some("t")),
            oid,
            "a@example.com",
            &on,
        );
        assert_eq!(none.note, None);
        assert!(
            none.image.is_none() && path.exists(),
            "a real miss is cached for 12 hours"
        );
    }

    #[test]
    fn turning_pictures_off_during_a_lookup_stops_the_repository_check() {
        let stub = Stub::start();
        stub.route("/repos/octo/hello", 200, "{}");
        let on = std::sync::Arc::new(AtomicBool::new(true));
        // The user turns pictures off while GitHub is answering the commit lookup with a 404.
        *stub.switch_off.lock().unwrap() = Some((
            "/repos/octo/hello/commits".to_owned(),
            std::sync::Arc::clone(&on),
        ));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("picture.json");
        let skipped = load_at(
            Some(path.clone()),
            &stub.client(Some("t")),
            Oid::from_str(UNPUSHED).unwrap(),
            "a@example.com",
            &on,
        );
        assert_eq!(
            stub.seen(),
            [format!(
                "/repos/octo/hello/commits?sha={UNPUSHED}&per_page=1"
            )],
            "nothing more is asked once pictures are off"
        );
        assert_eq!(skipped.note, None);
        assert!(skipped.image.is_none() && !path.exists());
    }

    #[test]
    fn a_refusal_from_the_server_reaches_the_explanation_and_the_log_as_plain_text() {
        let stub = Stub::start();
        // A hostile server dresses a lure up as Canopy's own text: bidirectional overrides and
        // isolates, invisible characters, control characters and a long tail.
        let lure = format!(
            "Bad\u{202E}credentials\u{2066}\u{200B}\u{FEFF}\n\u{7}\r\tpaste token {}",
            "x".repeat(500)
        );
        stub.route(
            "/repos/octo/hello/commits",
            401,
            serde_json::json!({ "message": lure }).to_string(),
        );
        let dir = tempfile::tempdir().unwrap();
        let note = load_at(
            Some(dir.path().join("p.json")),
            &stub.client(Some("t")),
            Oid::from_str(PUSHED).unwrap(),
            "a@example.com",
            &AtomicBool::new(true),
        )
        .note;
        let Some(Note::Blocked(Blocked::Refused(message))) = note else {
            panic!("a 401 refuses the repository: {note:?}");
        };
        let hidden = |c: char| {
            c.is_control()
                || matches!(
                    c,
                    '\u{061C}'
                        | '\u{200B}'..='\u{200F}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{FEFF}'
                )
        };
        assert!(
            message.starts_with("Badcredentials paste token xxx"),
            "{message:?}"
        );
        assert!(!message.chars().any(hidden), "{message:?}");
        assert!(
            message.chars().count() <= MAX_SERVER_TEXT + 1,
            "{} characters",
            message.chars().count()
        );
        let refused = Blocked::Refused(message);
        let shown = explain(
            &Situation {
                show_avatars: true,
                download: true,
                hosting_api: true,
                consented: true,
                forge: Some(canopy_core::forge::ForgeKind::GitHub),
                token: Some(true),
                status: Status::Blocked(&refused),
            },
            T0,
        )
        .unwrap();
        assert!(!shown.detail.chars().any(hidden), "{:?}", shown.detail);
    }

    #[test]
    fn the_repository_check_is_charged_to_the_budget_before_it_is_sent() {
        let stub = Stub::start();
        stub.route("/repos/octo/hello", 200, "{}");
        let mut granted = 0;
        // The budget has room for the commit lookup only.
        let mut take = || {
            granted += 1;
            if granted == 1 {
                Ok(())
            } else {
                Err(T0 + HOUR_SECS)
            }
        };
        let mut note = None;
        let result = fetch(
            &stub.client(None),
            Oid::from_str(UNPUSHED).unwrap(),
            "a@example.com",
            &AtomicBool::new(true),
            T0,
            &mut take,
            &mut note,
        );
        assert_eq!(result, Err(Miss::Later(T0 + HOUR_SECS)));
        assert_eq!(note, None);
        assert_eq!(
            stub.seen(),
            [format!(
                "/repos/octo/hello/commits?sha={UNPUSHED}&per_page=1"
            )],
            "the repository check waits for the next hour"
        );
    }

    fn finish_with(pictures: &mut Avatars, oid: Oid, email: &str, now: i64, result: Cached) {
        assert!(pictures.request_at(oid, email, now), "{email} is due");
        pictures.finish(&egui::Context::default(), oid, email.to_owned(), result);
    }

    fn kept(note: Note, retry_at: Option<i64>) -> Cached {
        Cached {
            image: None,
            checked_at: 0,
            retry_at,
            note: Some(note),
        }
    }

    /// A realistic time: the tab state treats a never-checked author (checked at 0) as due.
    const T0: i64 = 1_800_000_000;

    #[test]
    fn a_blocked_repository_stops_every_lookup_until_the_thread_restarts() {
        let mut pictures = Avatars::default();
        pictures.set_enabled(true);
        let oid = Oid::from_str(PUSHED).unwrap();
        assert_eq!(pictures.status_at(T0), Status::Working);
        finish_with(
            &mut pictures,
            oid,
            "a@example.com",
            T0,
            kept(Note::Blocked(Blocked::NotFound), None),
        );
        assert_eq!(pictures.status_at(T0), Status::Blocked(&Blocked::NotFound));
        assert!(!pictures.wants("b@example.com", T0));
        assert!(!pictures.wants("a@example.com", T0 + 2 * CACHE_SECS));
        // A new token or remote restarts the hosting thread with a fresh tab state
        // (`RepoTab::ensure_host`), and lookups start again.
        let mut pictures = Avatars::default();
        pictures.set_enabled(true);
        assert!(pictures.wants("a@example.com", T0));
    }

    #[test]
    fn an_unknown_commit_is_skipped_and_the_authors_next_commit_is_tried() {
        let mut pictures = Avatars::default();
        pictures.set_enabled(true);
        let commit = |n: u8| Oid::from_str(&format!("{n:02x}").repeat(20)).unwrap();
        finish_with(
            &mut pictures,
            commit(1),
            "me@example.com",
            T0,
            kept(Note::UnknownCommit, None),
        );
        assert_eq!(pictures.status_at(T0), Status::Working);
        assert!(
            !pictures.request_at(commit(1), "me@example.com", T0 + 1),
            "the same commit is not asked about again"
        );
        assert!(
            pictures.wants("other@example.com", T0 + 1),
            "other authors carry on"
        );
        finish_with(
            &mut pictures,
            commit(2),
            "me@example.com",
            T0 + 1,
            kept(Note::UnknownCommit, None),
        );
        finish_with(
            &mut pictures,
            commit(3),
            "me@example.com",
            T0 + 2,
            kept(Note::UnknownCommit, None),
        );
        assert!(
            !pictures.wants("me@example.com", T0 + 3),
            "after {MAX_UNKNOWN_COMMITS} unpushed commits the author waits for the next session"
        );
        assert!(pictures.wants("other@example.com", T0 + 3));
    }

    #[test]
    fn an_unreachable_service_pauses_lookups_and_a_success_resumes_them() {
        let mut pictures = Avatars::default();
        pictures.set_enabled(true);
        let oid = Oid::from_str(PUSHED).unwrap();
        let until = T0 + RETRY_UNREACHABLE_SECS;
        finish_with(
            &mut pictures,
            oid,
            "a@example.com",
            T0,
            kept(Note::Unreachable, Some(until)),
        );
        assert_eq!(
            pictures.status_at(T0),
            Status::Paused {
                why: Pause::Unreachable,
                until
            }
        );
        assert!(!pictures.wants("a@example.com", until - 1));
        assert_eq!(pictures.status_at(until), Status::Working);
        finish_with(
            &mut pictures,
            oid,
            "a@example.com",
            until,
            Cached {
                image: Some(ColorImage::new([64, 64], vec![egui::Color32::RED; 64 * 64])),
                checked_at: until,
                retry_at: None,
                note: None,
            },
        );
        assert_eq!(pictures.status_at(until), Status::Working);
        assert!(pictures.get("a@example.com").is_some());
    }

    #[test]
    fn missing_pictures_are_explained_with_the_setting_that_brings_them() {
        use canopy_core::forge::ForgeKind;
        let refused = Blocked::Refused("Bad credentials.".into());
        let base = Situation {
            show_avatars: true,
            download: true,
            hosting_api: true,
            consented: true,
            forge: Some(ForgeKind::GitHub),
            token: Some(false),
            status: Status::Working,
        };
        let why = |s: Situation<'_>| explain(&s, 1000).map(|e| (e.title, e.fix));
        assert_eq!(why(base), None, "working: nothing to explain");
        assert_eq!(
            why(Situation {
                show_avatars: false,
                hosting_api: false,
                ..base
            }),
            None
        );
        assert_eq!(
            why(Situation {
                consented: false,
                ..base
            }),
            None
        );
        assert_eq!(
            why(Situation {
                hosting_api: false,
                ..base
            }),
            Some(("Profile pictures are off", Some(Fix::Integrations)))
        );
        assert_eq!(
            why(Situation {
                download: false,
                ..base
            }),
            Some(("Profile pictures are off", Some(Fix::Privacy)))
        );
        assert_eq!(
            why(Situation {
                forge: None,
                ..base
            }),
            Some(("No GitHub or GitLab remote", None))
        );
        let hidden = Situation {
            status: Status::Blocked(&Blocked::NotFound),
            ..base
        };
        assert_eq!(
            why(hidden),
            Some(("Private repository", Some(Fix::Accounts)))
        );
        assert_eq!(
            why(Situation {
                token: Some(true),
                ..hidden
            }),
            Some(("Token can't see this repository", Some(Fix::Accounts)))
        );
        assert_eq!(
            why(Situation {
                forge: Some(ForgeKind::GitLab),
                ..hidden
            }),
            Some(("No picture lookup on this server", None))
        );
        let refused = Situation {
            status: Status::Blocked(&refused),
            ..base
        };
        assert_eq!(
            why(refused),
            Some(("Sign-in required", Some(Fix::Accounts))),
            "a server that wants a token before it answers"
        );
        let token = explain(
            &Situation {
                token: Some(true),
                ..refused
            },
            1000,
        )
        .unwrap();
        assert_eq!(token.title, "Token refused");
        assert_eq!(token.fix, Some(Fix::Accounts));
        assert!(
            token
                .detail
                .contains("refused the token: Bad credentials. Replace"),
            "{}",
            token.detail
        );
        let limit = Situation {
            status: Status::Paused {
                why: Pause::Limit,
                until: 1000 + 25 * 60,
            },
            ..base
        };
        let paused = explain(&limit, 1000).unwrap();
        assert!(paused.detail.contains("in 25 minutes"), "{}", paused.detail);
        assert_eq!(paused.fix, Some(Fix::Accounts), "a token raises the limit");
        assert_eq!(
            why(Situation {
                token: Some(true),
                ..limit
            })
            .unwrap()
            .1,
            None
        );
        let offline = Situation {
            status: Status::Paused {
                why: Pause::Unreachable,
                until: 1030,
            },
            ..base
        };
        assert!(
            explain(&offline, 1000)
                .unwrap()
                .detail
                .contains("in a minute")
        );
    }
}

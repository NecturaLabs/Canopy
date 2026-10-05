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
const PRUNE_AFTER: Duration = Duration::from_secs(30 * 24 * 60 * 60);
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

#[derive(Default)]
pub struct Avatars {
    pub enabled: bool,
    pictures: HashMap<String, Picture>,
    pending: Option<(Oid, String)>,
    retry_after: i64,
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
            && now >= self.retry_after
            && self.pending.is_none()
            && !self
                .pictures
                .get(email)
                .is_some_and(|p| fresh(p.checked_at, now))
    }

    pub fn request(&mut self, oid: Oid, email: &str) -> bool {
        self.request_at(oid, email, chrono::Utc::now().timestamp())
    }

    fn request_at(&mut self, oid: Oid, email: &str, now: i64) -> bool {
        if !self.wants(email, now) {
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
    /// it pictures may use, is spent.
    pub retry_at: Option<i64>,
}

/// The answer for a lookup that could not even be handed to a picture thread.
pub fn unavailable(now: i64) -> Cached {
    Cached {
        image: None,
        checked_at: 0,
        retry_at: Some(now + HOUR_SECS),
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
    let (record, retry_at) = refresh(old, now, || {
        if !enabled.load(Ordering::SeqCst) {
            return Err(Miss::Skipped);
        }
        let limit = if client.has_token() {
            LOOKUPS_PER_HOUR_SIGNED_IN
        } else {
            LOOKUPS_PER_HOUR_ANONYMOUS
        };
        BUDGET
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take(now, limit)
            .map_err(Miss::Later)?;
        let avatar = client
            .author_avatar(&oid.to_string(), email)
            .map_err(|e| match e {
                canopy_hosting::Error::RateLimited { reset } => Miss::Later(
                    reset.map_or(now + HOUR_SECS, |at| at.clamp(now + 60, now + CACHE_SECS)),
                ),
                _ => Miss::Failed,
            })?;
        if !enabled.load(Ordering::SeqCst) {
            return Err(Miss::Skipped);
        }
        let Some(avatar) = avatar else {
            return Err(Miss::Failed);
        };
        let bytes = avatar.download().map_err(|_| Miss::Failed)?;
        let image = decode(&bytes).map_err(|_| Miss::Failed)?;
        Ok(image.pixels.iter().flat_map(|p| p.to_array()).collect())
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
    }
}

/// The record after one due refresh, and when to ask a provider again if it was out of requests.
/// A skipped or postponed refresh changes nothing, so the author stays due.
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
        Err(Miss::Skipped) => (record, None),
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
}

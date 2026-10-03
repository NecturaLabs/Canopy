//! Author pictures cached locally for twelve hours, including failed lookups.
//! All cache I/O, provider requests and image decoding run on the hosting thread.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use canopy_core::Oid;
use egui::{ColorImage, TextureHandle};
use serde::{Deserialize, Serialize};

const MAX_PICTURES: usize = 256;
pub const CACHE_SECS: i64 = 12 * 60 * 60;
const PIXEL_BYTES: usize = 64 * 64 * 4;
const MAX_RECORD_BYTES: u64 = 128 * 1024;
// Tabs may have separate hosting threads. Serialize cache misses so they never fetch an author
// twice while the first request is still in flight.
static CACHE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default)]
pub struct Avatars {
    pub enabled: bool,
    pictures: HashMap<String, Picture>,
    pending: Option<(Oid, String)>,
    retry_after: i64,
}

struct Picture {
    texture: Option<TextureHandle>,
    checked_at: i64,
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
        self.enabled
            .then(|| self.pictures.get(email).and_then(|p| p.texture.as_ref()))
            .flatten()
    }

    pub fn request(&mut self, oid: Oid, email: &str) -> bool {
        self.request_at(oid, email, chrono::Utc::now().timestamp())
    }

    fn request_at(&mut self, oid: Oid, email: &str, now: i64) -> bool {
        if !self.enabled
            || now < self.retry_after
            || self.pending.is_some()
            || self
                .pictures
                .get(email)
                .is_some_and(|p| fresh(p.checked_at, now))
            || (!self.pictures.contains_key(email) && self.pictures.len() >= MAX_PICTURES)
        {
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
        if result.rate_limited {
            self.retry_after = result.checked_at.saturating_add(CACHE_SECS);
        }
        let entry = self.pictures.entry(email).or_insert(Picture {
            texture: None,
            checked_at: 0,
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
    pub rate_limited: bool,
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
    RateLimited,
}

/// Reads the cache and, only when due and still enabled, asks the provider once.
pub fn load(
    client: &canopy_hosting::Client,
    forge: &canopy_core::forge::Forge,
    oid: Oid,
    email: &str,
    enabled: &AtomicBool,
) -> Cached {
    let path = eframe::storage_dir("canopy").map(|p| {
        p.join("avatars").join(format!(
            "{}.json",
            canopy_hosting::avatars::cache_key(forge, email)
        ))
    });
    load_at(path, client, oid, email, enabled)
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
    let (record, rate_limited) = refresh(old, now, || {
        if !enabled.load(Ordering::SeqCst) {
            return Err(Miss::Skipped);
        }
        let avatar = client
            .author_avatar(&oid.to_string(), email)
            .map_err(|e| match e {
                canopy_hosting::Error::RateLimited { .. } => Miss::RateLimited,
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
        rate_limited,
    }
}

/// The record after one due refresh. A skipped refresh changes nothing, so the author stays due.
fn refresh(
    old: Option<Record>,
    now: i64,
    fetch: impl FnOnce() -> Result<Vec<u8>, Miss>,
) -> (Record, bool) {
    if old.as_ref().is_some_and(|r| fresh(r.checked_at, now)) {
        return (old.unwrap_or_default(), false);
    }
    let mut record = old.unwrap_or_default();
    match fetch() {
        Ok(pixels) => {
            record.pixels = Some(pixels);
            record.checked_at = now;
            (record, false)
        }
        Err(Miss::Skipped) => (record, false),
        Err(miss) => {
            record.checked_at = now;
            (record, miss == Miss::RateLimited)
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
                rate_limited: false,
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
        assert!(skipped.image.is_none() && !skipped.rate_limited);
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
    fn png_and_jpeg_decode_to_bounded_pictures_and_other_input_is_refused() {
        let image = image::DynamicImage::new_rgb8(80, 100);
        for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, format).unwrap();
            assert_eq!(decode(bytes.get_ref()).unwrap().size, [64, 64]);
        }
        assert!(decode(b"not a picture").is_err());
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2049, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(decode(bytes.get_ref()).is_err());
        assert!(decode(&vec![0; 2 * 1024 * 1024 + 1]).is_err());
    }
}

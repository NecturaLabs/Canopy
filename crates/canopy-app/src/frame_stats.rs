//! Frame-time bookkeeping for the `frame stats` and `first frame` log events (issue #62), which
//! `tools/stress_report.py` reads.
//!
//! A frame's time is what eframe reports for the previous frame: the CPU time of the UI pass,
//! tessellation and painting, without the wait for the display (`IntegrationInfo::cpu_usage`).
//! Frames are counted into a fixed histogram, so a frame costs one array increment and the
//! window allocates nothing. About every [`WINDOW`] of produced frames the window is summed up
//! and cleared. An idle app produces no frames, so it logs nothing; frames left over when it goes
//! idle are written when the next one arrives.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How long frames are gathered before one `frame stats` event summarises them.
const WINDOW: Duration = Duration::from_secs(5);
/// Width of a histogram bucket, in microseconds: the resolution of the reported percentiles.
const BUCKET_MICROS: u32 = 250;
/// Buckets below the overflow bucket: frame times up to 100 ms.
const BUCKETS: usize = 400;
/// A frame over this many microseconds missed 30 frames per second.
const HITCH_33_MICROS: u32 = 33_000;
/// A frame over this many microseconds is a visible stall.
const HITCH_100_MICROS: u32 = 100_000;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Records when the process started, for `first frame`. Call first thing in `main`.
pub fn mark_process_start() {
    let _ = PROCESS_START.set(Instant::now());
}

/// What one `frame stats` event reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub frames: u32,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Frames over 33 ms.
    pub hitches_33: u32,
    /// Frames over 100 ms.
    pub hitches_100: u32,
}

impl Summary {
    fn log(&self) {
        tracing::debug!(
            frames = self.frames,
            p50_ms = self.p50_ms,
            p95_ms = self.p95_ms,
            p99_ms = self.p99_ms,
            max_ms = self.max_ms,
            hitches_33 = self.hitches_33,
            hitches_100 = self.hitches_100,
            "frame stats"
        );
    }
}

/// Frame times of one window as a fixed histogram plus the exact maximum and hitch counts.
#[derive(Debug)]
struct Window {
    /// Frames per bucket; the last one holds every frame of 100 ms or more.
    counts: [u32; BUCKETS + 1],
    frames: u32,
    max_micros: u32,
    hitches_33: u32,
    hitches_100: u32,
}

impl Window {
    const fn new() -> Self {
        Self {
            counts: [0; BUCKETS + 1],
            frames: 0,
            max_micros: 0,
            hitches_33: 0,
            hitches_100: 0,
        }
    }

    /// Counts one frame. O(1), no allocation.
    fn record(&mut self, micros: u32) {
        let bucket = usize::try_from(micros / BUCKET_MICROS).map_or(BUCKETS, |b| b.min(BUCKETS));
        if let Some(count) = self.counts.get_mut(bucket) {
            *count = count.saturating_add(1);
        }
        self.frames = self.frames.saturating_add(1);
        self.max_micros = self.max_micros.max(micros);
        if micros > HITCH_33_MICROS {
            self.hitches_33 = self.hitches_33.saturating_add(1);
        }
        if micros > HITCH_100_MICROS {
            self.hitches_100 = self.hitches_100.saturating_add(1);
        }
    }

    /// The frame time below which `percent` of the frames fall (nearest rank), as the upper edge
    /// of its bucket, never above the slowest frame.
    fn percentile_micros(&self, percent: u32) -> u32 {
        let rank = u64::from(self.frames) * u64::from(percent);
        let rank = rank.div_ceil(100).max(1);
        let mut seen = 0_u64;
        for (bucket, count) in self.counts.iter().enumerate() {
            seen += u64::from(*count);
            if seen >= rank {
                let edge = u32::try_from(bucket.saturating_add(1))
                    .map_or(u32::MAX, |b| b.saturating_mul(BUCKET_MICROS));
                return if bucket >= BUCKETS {
                    self.max_micros
                } else {
                    edge.min(self.max_micros)
                };
            }
        }
        self.max_micros
    }

    /// The window's summary, or `None` when it holds no frame.
    fn summary(&self) -> Option<Summary> {
        (self.frames > 0).then(|| Summary {
            frames: self.frames,
            p50_ms: millis(self.percentile_micros(50)),
            p95_ms: millis(self.percentile_micros(95)),
            p99_ms: millis(self.percentile_micros(99)),
            max_ms: millis(self.max_micros),
            hitches_33: self.hitches_33,
            hitches_100: self.hitches_100,
        })
    }

    fn clear(&mut self) {
        *self = Self::new();
    }
}

fn millis(micros: u32) -> f64 {
    f64::from(micros) / 1_000.0
}

/// The app's frame statistics: the running window and whether `first frame` was logged.
#[derive(Debug)]
pub struct FrameStats {
    window: Window,
    /// When the window's first frame arrived.
    since: Option<Instant>,
    first_frame_logged: bool,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            window: Window::new(),
            since: None,
            first_frame_logged: false,
        }
    }
}

impl FrameStats {
    /// Called at the start of every frame with the previous frame's CPU seconds (`None` on the
    /// first). Returns the summary of a window that has run its course.
    fn begin(&mut self, now: Instant, previous_secs: Option<f32>) -> Option<Summary> {
        let due = self
            .since
            .is_some_and(|since| now.saturating_duration_since(since) >= WINDOW);
        let finished = if due {
            let summary = self.window.summary();
            self.window.clear();
            self.since = None;
            summary
        } else {
            None
        };
        if let Some(secs) = previous_secs
            && let Ok(frame) = Duration::try_from_secs_f32(secs)
        {
            let micros = u32::try_from(frame.as_micros()).unwrap_or(u32::MAX);
            self.window.record(micros);
            self.since.get_or_insert(now);
        }
        finished
    }

    /// Logs a `frame stats` event when a window has run its course. Call at the start of a frame.
    pub fn frame_started(&mut self, previous_secs: Option<f32>) {
        if let Some(summary) = self.begin(Instant::now(), previous_secs) {
            summary.log();
        }
    }

    /// The milliseconds from process start to the end of the first UI pass, once.
    fn first_frame_ms(&mut self, now: Instant, start: Instant) -> Option<u64> {
        if std::mem::replace(&mut self.first_frame_logged, true) {
            return None;
        }
        Some(u64::try_from(now.saturating_duration_since(start).as_millis()).unwrap_or(u64::MAX))
    }

    /// Logs `first frame` once. Call at the end of every frame's UI pass.
    pub fn frame_finished(&mut self) {
        if self.first_frame_logged {
            return;
        }
        let Some(start) = PROCESS_START.get() else {
            return;
        };
        if let Some(ms) = self.first_frame_ms(Instant::now(), *start) {
            tracing::info!(ms, "first frame");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window_of(frames_ms: &[f64]) -> Window {
        let mut w = Window::new();
        for ms in frames_ms {
            let micros = Duration::from_secs_f64(ms / 1_000.0).as_micros();
            w.record(u32::try_from(micros).expect("fits"));
        }
        w
    }

    #[test]
    fn an_empty_window_has_no_summary() {
        assert_eq!(Window::new().summary(), None);
    }

    #[test]
    fn percentiles_use_the_nearest_rank_within_a_bucket() {
        // 1 to 100 ms in 1 ms steps: the p50 is the 50th frame, 50 ms.
        let frames: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = window_of(&frames).summary().expect("frames");
        assert_eq!(s.frames, 100);
        assert!((s.p50_ms - 50.0).abs() <= 0.25, "p50 {}", s.p50_ms);
        assert!((s.p95_ms - 95.0).abs() <= 0.25, "p95 {}", s.p95_ms);
        assert!((s.p99_ms - 99.0).abs() <= 0.25, "p99 {}", s.p99_ms);
        assert!((s.max_ms - 100.0).abs() < 0.001, "max {}", s.max_ms);
    }

    #[test]
    fn one_slow_frame_moves_the_max_and_the_tail_but_not_the_median() {
        let mut frames = vec![4.0; 99];
        frames.push(250.0);
        let s = window_of(&frames).summary().expect("frames");
        assert!(s.p50_ms <= 4.25, "p50 {}", s.p50_ms);
        assert!(s.p95_ms <= 4.25, "p95 {}", s.p95_ms);
        assert!(
            s.p99_ms <= 4.25,
            "the 99th of 100 frames is still fast: {}",
            s.p99_ms
        );
        assert!(
            (s.max_ms - 250.0).abs() < 0.001,
            "an overflow frame keeps its exact time"
        );
        assert_eq!((s.hitches_33, s.hitches_100), (1, 1));
    }

    #[test]
    fn a_percentile_in_the_overflow_bucket_is_the_slowest_frame() {
        let s = window_of(&[300.0, 400.0, 500.0]).summary().expect("frames");
        assert!((s.p50_ms - 500.0).abs() < 0.001, "p50 {}", s.p50_ms);
        assert!((s.p99_ms - 500.0).abs() < 0.001, "p99 {}", s.p99_ms);
    }

    #[test]
    fn a_percentile_never_exceeds_the_slowest_frame() {
        // 1.1 ms sits in the bucket whose upper edge is 1.25 ms.
        let s = window_of(&[1.1]).summary().expect("frames");
        assert!((s.p50_ms - 1.1).abs() < 0.001, "p50 {}", s.p50_ms);
    }

    #[test]
    fn hitches_count_frames_over_33_and_over_100_ms() {
        let s = window_of(&[16.0, 33.0, 33.5, 99.0, 100.0, 101.0])
            .summary()
            .expect("frames");
        // Exactly 33 and exactly 100 ms are not over.
        assert_eq!(s.hitches_33, 4, "33.5, 99, 100 and 101 ms");
        assert_eq!(s.hitches_100, 1, "101 ms");
    }

    #[test]
    fn the_window_clears() {
        let mut w = window_of(&[5.0, 200.0]);
        w.clear();
        assert_eq!(w.summary(), None);
        assert_eq!((w.hitches_33, w.hitches_100, w.max_micros), (0, 0, 0));
    }

    #[test]
    fn a_summary_comes_out_once_the_window_has_run_its_course() {
        let mut stats = FrameStats::default();
        let t0 = Instant::now();
        assert_eq!(
            stats.begin(t0, None),
            None,
            "the first frame has no time yet"
        );
        assert_eq!(
            stats.begin(t0 + Duration::from_millis(16), Some(0.004)),
            None
        );
        assert_eq!(stats.begin(t0 + Duration::from_secs(2), Some(0.004)), None);
        let s = stats
            .begin(t0 + Duration::from_millis(16) + WINDOW, Some(0.05))
            .expect("5 s after the window's first frame");
        assert_eq!(s.frames, 2);
        assert_eq!(
            s.hitches_33, 0,
            "the frame that closed it belongs to the next window"
        );
        // The next window starts with the frame that closed the old one.
        let next = stats
            .begin(t0 + Duration::from_millis(16) + WINDOW * 2, Some(0.004))
            .expect("the second window");
        assert_eq!((next.frames, next.hitches_33), (1, 1));
    }

    #[test]
    fn an_idle_gap_does_not_count_as_a_frame_time() {
        let mut stats = FrameStats::default();
        let t0 = Instant::now();
        stats.begin(t0, Some(0.002));
        // A minute later the first frame after the gap flushes the old window; the gap itself is
        // never a sample.
        let s = stats
            .begin(t0 + Duration::from_mins(1), Some(0.003))
            .expect("old window");
        assert_eq!(s.frames, 1);
        assert!(s.max_ms < 3.0, "max {}", s.max_ms);
    }

    #[test]
    fn a_nonsense_frame_time_is_ignored() {
        let mut stats = FrameStats::default();
        let now = Instant::now();
        for bad in [f32::NAN, -1.0, f32::INFINITY] {
            stats.begin(now, Some(bad));
        }
        assert_eq!(stats.window.summary(), None);
    }

    #[test]
    fn the_first_frame_is_reported_once_with_the_time_since_start() {
        let mut stats = FrameStats::default();
        let start = Instant::now();
        let later = start + Duration::from_millis(1_234);
        assert_eq!(stats.first_frame_ms(later, start), Some(1_234));
        assert_eq!(stats.first_frame_ms(later, start), None);
    }
}

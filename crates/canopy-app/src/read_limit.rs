//! A process-wide limit on the refreshes repository threads run for tabs nobody is looking at.
//!
//! Every loaded tab has its own thread, so an external process that touches many repositories at
//! once (a build, a branch switch in a monorepo's checkouts, a sync tool) wakes all their file
//! watchers together. Without a limit each of those threads would start a `git status` read
//! together. A thread asks its [`Lane`] for a [`Permit`] before such a read and keeps it for the
//! read's duration; the tab being shown has a priority lane that never waits.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, PoisonError};

/// The fewest background reads that run at once, however few cores there are: one slow read
/// (a large repository, a network drive) must not hold up every other tab's.
const MIN_PERMITS: usize = 2;

/// The most background reads that run at once, however many cores there are. A status or history
/// read is mostly disk bound, and more than this only makes them slow each other down.
const MAX_PERMITS: usize = 4;

/// How many background reads may run at once on this machine: the number of cores, between
/// [`MIN_PERMITS`] and [`MAX_PERMITS`].
pub fn default_permits() -> usize {
    std::thread::available_parallelism()
        .map_or(MIN_PERMITS, std::num::NonZero::get)
        .clamp(MIN_PERMITS, MAX_PERMITS)
}

/// The limit every repository thread of this process shares.
pub fn shared() -> Arc<Limiter> {
    static SHARED: LazyLock<Arc<Limiter>> =
        LazyLock::new(|| Arc::new(Limiter::new(default_permits())));
    Arc::clone(&SHARED)
}

/// A counting semaphore. Waiting holds no lock: a waiter sleeps on the condition variable, which
/// releases the mutex, and the mutex guards only the counters, never a read.
#[derive(Debug)]
pub struct Limiter {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Debug)]
struct State {
    free: usize,
    waiting: usize,
}

impl Limiter {
    pub fn new(permits: usize) -> Self {
        Self {
            state: Mutex::new(State {
                free: permits,
                waiting: 0,
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wakes every waiter to look again at its own lane's flags. Taking the lock first means a
    /// waiter that has just checked them and is about to sleep is woken, not missed.
    fn wake(&self) {
        drop(self.lock());
        self.changed.notify_all();
    }

    /// How many threads wait for a permit now.
    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.lock().waiting
    }
}

/// One repository thread's way to the limit, shared with its [`Worker`](crate::worker::Worker).
#[derive(Debug)]
pub struct Lane {
    limiter: Arc<Limiter>,
    priority: AtomicBool,
    closed: AtomicBool,
}

impl Lane {
    pub fn new(limiter: Arc<Limiter>) -> Self {
        Self {
            limiter,
            priority: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }

    /// Whether this thread's reads skip the limit: set for the tab the user is looking at, which
    /// must never wait behind tabs in the background. Takes effect at once, also for a thread
    /// already waiting.
    pub fn set_priority(&self, on: bool) {
        if self.priority.swap(on, Ordering::AcqRel) != on {
            self.limiter.wake();
        }
    }

    #[cfg(test)]
    pub fn has_priority(&self) -> bool {
        self.priority.load(Ordering::Acquire)
    }

    /// The worker this lane belongs to is gone: a thread waiting for a permit stops waiting.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.limiter.wake();
    }

    /// Waits for permission to run one background read. `None` when the lane was closed meanwhile
    /// (the tab is gone, nothing wants the read). A priority lane gets a permit that counts for
    /// nothing, at once.
    pub fn acquire(&self) -> Option<Permit> {
        let mut state = self.limiter.lock();
        let mut counted = false;
        let permit = loop {
            if self.closed.load(Ordering::Acquire) {
                break None;
            }
            if self.priority.load(Ordering::Acquire) {
                break Some(Permit { limiter: None });
            }
            if state.free > 0 {
                state.free -= 1;
                break Some(Permit {
                    limiter: Some(Arc::clone(&self.limiter)),
                });
            }
            if !counted {
                counted = true;
                state.waiting += 1;
            }
            state = self
                .limiter
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        };
        if counted {
            state.waiting -= 1;
        }
        permit
    }
}

/// Permission to run one read; dropping it lets the next waiter go.
#[derive(Debug)]
pub struct Permit {
    /// The limiter to give the permit back to; `None` for a priority read, which took none.
    limiter: Option<Arc<Limiter>>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        if let Some(limiter) = self.limiter.take() {
            limiter.lock().free += 1;
            // Every waiter looks again: one takes the permit, and a waiter whose lane closed or
            // became the shown one in the meantime must not swallow the wake-up.
            limiter.changed.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    use super::*;

    /// Runs `body` on a thread and returns its result, or `None` if it has not finished within
    /// `within`: a call that blocks for ever fails the test instead of hanging it.
    fn finishes<T: Send + 'static>(
        within: Duration,
        body: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send(body());
        });
        rx.recv_timeout(within).ok()
    }

    fn wait_until_waiting(limiter: &Limiter, count: usize) {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while limiter.waiting() < count {
            assert!(std::time::Instant::now() < end, "no thread started waiting");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The limit bounds how many fake reads run at once, and still lets them run in parallel.
    #[test]
    fn the_limit_bounds_concurrent_reads() {
        const PERMITS: usize = 3;
        let limiter = Arc::new(Limiter::new(PERMITS));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let lane = Lane::new(Arc::clone(&limiter));
                let (running, peak, finished) = (
                    Arc::clone(&running),
                    Arc::clone(&peak),
                    Arc::clone(&finished),
                );
                std::thread::spawn(move || {
                    let permit = lane.acquire().expect("the lane is open");
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(15));
                    running.fetch_sub(1, Ordering::SeqCst);
                    finished.fetch_add(1, Ordering::SeqCst);
                    drop(permit);
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("a read thread panicked");
        }
        assert_eq!(finished.load(Ordering::SeqCst), 16, "every read ran");
        let peak = peak.load(Ordering::SeqCst);
        assert!(peak <= PERMITS, "{peak} reads ran at once");
        assert!(peak >= 2, "the reads did run in parallel: {peak}");
    }

    /// A permit given back goes to a waiting thread.
    #[test]
    fn a_finished_read_lets_a_waiting_one_start() {
        let limiter = Arc::new(Limiter::new(1));
        let first = Lane::new(Arc::clone(&limiter))
            .acquire()
            .expect("a free permit");
        let lane = Lane::new(Arc::clone(&limiter));
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send(lane.acquire().is_some());
        });
        wait_until_waiting(&limiter, 1);
        assert!(rx.try_recv().is_err(), "no permit is free yet");
        drop(first);
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(true));
    }

    /// The tab being shown never waits: with every permit taken its read still starts, and a
    /// thread that is already waiting starts when its tab becomes the shown one.
    #[test]
    fn a_priority_lane_bypasses_the_limit() {
        let limiter = Arc::new(Limiter::new(1));
        let _held = Lane::new(Arc::clone(&limiter))
            .acquire()
            .expect("a free permit");

        let shown = Lane::new(Arc::clone(&limiter));
        shown.set_priority(true);
        assert!(
            finishes(Duration::from_secs(10), move || shown.acquire().is_some()) == Some(true),
            "the shown tab waited behind a background read"
        );

        let waiting = Arc::new(Lane::new(Arc::clone(&limiter)));
        let (tx, rx) = crossbeam_channel::bounded(1);
        let lane = Arc::clone(&waiting);
        std::thread::spawn(move || {
            let _ = tx.send(lane.acquire().is_some());
        });
        wait_until_waiting(&limiter, 1);
        assert!(rx.try_recv().is_err(), "a background tab waits");
        waiting.set_priority(true);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(10)),
            Ok(true),
            "showing the tab ends its wait"
        );
        // A priority read takes nothing from the others.
        assert_eq!(limiter.lock().free, 0);
    }

    /// A tab that is closed (or no longer shown, once it asks again) goes back to the limit.
    #[test]
    fn a_lane_that_loses_its_priority_waits_again() {
        let limiter = Arc::new(Limiter::new(0));
        let lane = Arc::new(Lane::new(Arc::clone(&limiter)));
        lane.set_priority(true);
        assert!(lane.acquire().is_some());
        lane.set_priority(false);
        let (tx, rx) = crossbeam_channel::bounded(1);
        let waiting = Arc::clone(&lane);
        std::thread::spawn(move || {
            let _ = tx.send(waiting.acquire().is_some());
        });
        wait_until_waiting(&limiter, 1);
        lane.close();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(false));
    }

    /// Closing a lane ends a wait for a permit that would never come.
    #[test]
    fn closing_a_lane_ends_its_wait() {
        let limiter = Arc::new(Limiter::new(0));
        let lane = Arc::new(Lane::new(Arc::clone(&limiter)));
        let (tx, rx) = crossbeam_channel::bounded(1);
        let waiting = Arc::clone(&lane);
        std::thread::spawn(move || {
            let _ = tx.send(waiting.acquire().is_some());
        });
        wait_until_waiting(&limiter, 1);
        lane.close();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(false));
        assert_eq!(limiter.waiting(), 0);
        assert!(lane.acquire().is_none(), "a closed lane stays closed");
    }

    #[test]
    fn the_default_limit_follows_the_cores_within_a_small_range() {
        assert!((MIN_PERMITS..=MAX_PERMITS).contains(&default_permits()));
    }
}

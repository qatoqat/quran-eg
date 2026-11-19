//! Work off the UI's frame.
//!
//! Natively: two small pools of long-lived threads, one for the network (API
//! requests, font files), one for drawing sprite sheets. (Pools rather than a
//! thread per job: glibc gives every thread its own malloc arena and keeps what
//! was freed in it, so a thread per page drawn left tens of megabytes behind
//! after a long scroll.)
//!
//! In a browser there are no threads here: jobs wait in a queue and
//! [`pump`], called once a frame, runs them for a few milliseconds. A job that
//! takes longer is written in steps (see [`spawn_steps`]) and goes on in the
//! next frame, so the page never freezes.
//!
//! Either way a job's result arrives on a channel and the UI is woken. A job
//! that dies (a panic in a parser, say) is reported like any other failure, so
//! nothing waits for it forever.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

// (In a browser there is one queue; network requests go through `fetch`.)
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pool {
    /// Waits on the network; several at once.
    Net,
    /// Uses a core while it runs.
    Draw,
}

/// Until when a step of a job may run. Natively: as long as it likes.
pub struct Deadline(Option<web_time::Instant>);

impl Deadline {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn expired(&self) -> bool {
        self.0.is_some_and(|at| web_time::Instant::now() >= at)
    }
}

/// What became of a job started with [`run`].
pub enum Poll<T> {
    Pending,
    Done(T),
    /// The job died without a result.
    Lost,
}

pub fn poll<T>(rx: &Receiver<T>) -> Poll<T> {
    match rx.try_recv() {
        Ok(value) => Poll::Done(value),
        Err(TryRecvError::Empty) => Poll::Pending,
        Err(TryRecvError::Disconnected) => Poll::Lost,
    }
}

/// Run `work`; if it panics, `otherwise` says what the result is instead.
pub fn guarded<T>(work: impl FnOnce() -> T, otherwise: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|_| otherwise())
}

/// A job in steps: each call does some of the work and returns `true` once it is done.
type Job = Box<dyn FnMut(&Deadline) -> bool + Send>;

#[cfg(not(target_arch = "wasm32"))]
mod backend {
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Arc, Mutex, OnceLock};

    use super::{Deadline, Job, Pool};

    fn sender(pool: Pool) -> &'static Sender<Job> {
        static NET: OnceLock<Sender<Job>> = OnceLock::new();
        static DRAW: OnceLock<Sender<Job>> = OnceLock::new();
        match pool {
            Pool::Net => NET.get_or_init(|| start("net", 4)),
            Pool::Draw => DRAW.get_or_init(|| start("draw", 2)),
        }
    }

    fn start(name: &str, threads: usize) -> Sender<Job> {
        let (tx, rx) = channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..threads {
            let rx = rx.clone();
            let spawned = std::thread::Builder::new().name(format!("{name}-{i}")).spawn(move || {
                loop {
                    // (The guard is released before the job runs.)
                    let job = rx.lock().map(|rx| rx.recv());
                    match job {
                        // A thread of its own: every step runs straight after the last.
                        Ok(Ok(mut job)) => while !job(&Deadline::none()) {},
                        _ => return,
                    }
                }
            });
            if let Err(e) = spawned {
                eprintln!("quran-eg: could not start a worker thread ({name}): {e}");
            }
        }
        tx
    }

    pub fn submit(pool: Pool, job: Job) {
        if sender(pool).send(job).is_err() {
            eprintln!("quran-eg: the worker threads are gone; a background job was dropped");
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod backend {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::{Deadline, Job, Pool};

    /// Time a frame gives to queued jobs, in milliseconds.
    const BUDGET_MS: u64 = 6;

    thread_local! {
        static QUEUE: RefCell<VecDeque<Job>> = const { RefCell::new(VecDeque::new()) };
    }

    pub fn submit(_pool: Pool, job: Job) {
        QUEUE.with_borrow_mut(|queue| queue.push_back(job));
    }

    /// Run queued jobs for this frame's share of time; ask for another frame if some are left.
    pub fn pump(ctx: &egui::Context) {
        let deadline = Deadline(Some(web_time::Instant::now() + std::time::Duration::from_millis(BUDGET_MS)));
        loop {
            // (Taken out while it runs: a job may queue another.)
            let Some(mut job) = QUEUE.with_borrow_mut(VecDeque::pop_front) else { return };
            if !job(&deadline) {
                QUEUE.with_borrow_mut(|queue| queue.push_front(job));
            }
            if deadline.expired() {
                break;
            }
        }
        if QUEUE.with_borrow(|queue| !queue.is_empty()) {
            ctx.request_repaint();
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use backend::pump;

/// Run `work` off the frame; nothing is sent anywhere (it reports by itself).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn detach(pool: Pool, ctx: &egui::Context, work: impl FnOnce() + Send + 'static) {
    let repaint = ctx.clone();
    let mut work = Some(work);
    backend::submit(
        pool,
        Box::new(move |_| {
            if let Some(work) = work.take() {
                let _ = catch_unwind(AssertUnwindSafe(work));
                repaint.request_repaint();
            }
            true
        }),
    );
    #[cfg(target_arch = "wasm32")]
    ctx.request_repaint();
}

/// Run a job written in steps: `step` is called until it returns a result,
/// which is sent to `tx`. In a browser a step should return `None` once the
/// deadline has expired, and is called again in the next frame.
pub fn spawn_steps<T: Send + 'static>(
    pool: Pool,
    ctx: &egui::Context,
    tx: &Sender<T>,
    mut step: impl FnMut(&Deadline) -> Option<T> + Send + 'static,
) {
    let (repaint, tx) = (ctx.clone(), Some(tx.clone()));
    let mut tx = tx;
    backend::submit(
        pool,
        Box::new(move |deadline| {
            // Dropping `tx` without sending (a panic) tells a `run` receiver the job is lost.
            let done = match catch_unwind(AssertUnwindSafe(|| step(deadline))) {
                Ok(None) => return false,
                Ok(Some(result)) => Some(result),
                Err(_) => None,
            };
            if let (Some(tx), Some(result)) = (tx.take(), done) {
                // The receiver is gone when the view that asked has been closed.
                let _ = tx.send(result);
            }
            repaint.request_repaint();
            true
        }),
    );
    #[cfg(target_arch = "wasm32")]
    ctx.request_repaint();
}

/// Run `work` off the frame and send its result to `tx`.
pub fn spawn<T: Send + 'static>(pool: Pool, ctx: &egui::Context, tx: &Sender<T>, work: impl FnOnce() -> T + Send + 'static) {
    let mut work = Some(work);
    spawn_steps(pool, ctx, tx, move |_| work.take().map(|work| work()));
}

/// Run `work` off the frame; its result arrives on the returned channel.
pub fn run<T: Send + 'static>(pool: Pool, ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = channel();
    spawn(pool, ctx, &tx, work);
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn wait<T>(rx: &Receiver<T>) -> Poll<T> {
        for _ in 0..500 {
            match poll(rx) {
                Poll::Pending => std::thread::sleep(Duration::from_millis(10)),
                done => return done,
            }
        }
        Poll::Pending
    }

    #[test]
    fn jobs_report_their_result_or_that_they_died() {
        let ctx = egui::Context::default();
        let rx = run(Pool::Draw, &ctx, || 6 * 7);
        assert!(matches!(wait(&rx), Poll::Done(42)));
        // A job that panics is lost, not pending forever, and its thread lives on.
        let rx = run(Pool::Draw, &ctx, || -> u32 { panic!("a parser gave up") });
        assert!(matches!(wait(&rx), Poll::Lost));
        for _ in 0..4 {
            let rx = run(Pool::Draw, &ctx, || 1);
            assert!(matches!(wait(&rx), Poll::Done(1)));
        }
        assert_eq!(guarded(|| -> u32 { panic!("no") }, || 7), 7);
    }

    #[test]
    fn a_job_in_steps_runs_until_it_has_a_result() {
        let ctx = egui::Context::default();
        let (tx, rx) = channel();
        let mut count = 0;
        spawn_steps(Pool::Draw, &ctx, &tx, move |deadline| {
            assert!(!deadline.expired(), "a thread of its own has all the time it needs");
            count += 1;
            (count == 5).then_some(count)
        });
        assert!(matches!(wait(&rx), Poll::Done(5)));
    }
}

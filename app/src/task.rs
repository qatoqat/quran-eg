//! Work off the UI thread, on two small pools of long-lived threads: one for
//! the network (API requests, font files), one for drawing sprite sheets. Each
//! job wakes the UI when it is done. A job that dies (a panic in a parser,
//! say) is reported like any other failure, so nothing waits for it forever.
//!
//! Pools rather than a thread per job: glibc gives every thread its own malloc
//! arena and keeps what was freed in it, so a thread per page drawn left tens
//! of megabytes behind after a long scroll.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pool {
    /// Waits on the network; several at once.
    Net,
    /// Uses a core while it runs.
    Draw,
}

impl Pool {
    fn sender(self) -> &'static Sender<Job> {
        static NET: OnceLock<Sender<Job>> = OnceLock::new();
        static DRAW: OnceLock<Sender<Job>> = OnceLock::new();
        match self {
            Pool::Net => NET.get_or_init(|| start("net", 4)),
            Pool::Draw => DRAW.get_or_init(|| start("draw", 2)),
        }
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
                    Ok(Ok(job)) => job(),
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

/// Run `work` on a pool thread and send its result to `tx`.
pub fn spawn<T: Send + 'static>(pool: Pool, ctx: &egui::Context, tx: &Sender<T>, work: impl FnOnce() -> T + Send + 'static) {
    let (ctx, tx) = (ctx.clone(), tx.clone());
    let job: Job = Box::new(move || {
        // Dropping `tx` without sending (a panic) tells a `run` receiver the job is lost.
        if let Ok(result) = catch_unwind(AssertUnwindSafe(work)) {
            // The receiver is gone when the view that asked has been closed.
            let _ = tx.send(result);
        }
        drop(tx);
        ctx.request_repaint();
    });
    if pool.sender().send(job).is_err() {
        eprintln!("quran-eg: the worker threads are gone; a background job was dropped");
    }
}

/// Run `work` on a pool thread; its result arrives on the returned channel.
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
}

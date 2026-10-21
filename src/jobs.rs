//! Background work for the app: syncing sources and looking up movie info.
//!
//! One worker thread runs a queue of jobs in order. Running them one at a
//! time keeps the media server's load predictable and means only one
//! writer competes with the UI for the database. The worker owns its own
//! SQLite connection; the UI's connection keeps serving reads (WAL).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock};

use anyhow::Result;
use rusqlite::params;
use serde::Serialize;

use crate::config::{Config, Root};
use crate::crawler::{self, CrawlMode, CrawlStats, Progress};
use crate::index::{unix_now, Index};
use crate::library;
use crate::tmdb::{self, LookupProgress};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// Crawl one source and regroup its titles. `full` forgets checkpoints.
    Sync { root: String, full: bool },
    /// Look up TMDB info for titles that don't have it.
    Lookup { retry_misses: bool },
}

const LOG_LINES: usize = 300;

/// What the UI's status pill and source list show.
#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub running: bool,
    /// e.g. "Animation Movies · syncing" or "Looking up movie info".
    pub label: String,
    /// e.g. "1,332 videos · 210 folders".
    pub detail: String,
    pub done: Option<usize>,
    pub total: Option<usize>,
    pub current_root: Option<String>,
    /// Sources waiting their turn.
    pub queued_roots: Vec<String>,
    pub lookup_queued: bool,
    pub message: String,
    pub log: VecDeque<String>,
}

struct Shared {
    status: Mutex<Status>,
    queue: Mutex<VecDeque<Job>>,
    wake: Condvar,
    cancel: AtomicBool,
    config: Arc<RwLock<Config>>,
}

#[derive(Clone)]
pub struct Jobs {
    shared: Arc<Shared>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Jobs {
    pub fn start(config: Arc<RwLock<Config>>) -> Jobs {
        let shared = Arc::new(Shared {
            status: Mutex::new(Status::default()),
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            cancel: AtomicBool::new(false),
            config,
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("medialib-jobs".into())
            .spawn(move || worker_loop(&worker))
            .expect("spawning the job thread");
        Jobs { shared }
    }

    /// Adds a job unless the same one is already waiting.
    pub fn enqueue(&self, job: Job) {
        {
            let mut q = lock(&self.shared.queue);
            let duplicate = q.iter().any(|j| match (j, &job) {
                (Job::Sync { root: a, .. }, Job::Sync { root: b, .. }) => a == b,
                (Job::Lookup { .. }, Job::Lookup { .. }) => true,
                _ => false,
            });
            if !duplicate {
                q.push_back(job);
            }
            self.publish_queue(&q);
        }
        self.shared.wake.notify_one();
    }

    /// Stops the running job and empties the queue.
    pub fn stop_all(&self) {
        let mut q = lock(&self.shared.queue);
        q.clear();
        self.publish_queue(&q);
        drop(q);
        if lock(&self.shared.status).running {
            self.shared.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Stops one source: drops it from the queue, or cancels it if running.
    pub fn stop_root(&self, root: &str) {
        let mut q = lock(&self.shared.queue);
        q.retain(|j| !matches!(j, Job::Sync { root: r, .. } if r == root));
        self.publish_queue(&q);
        drop(q);
        if lock(&self.shared.status).current_root.as_deref() == Some(root) {
            self.shared.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    /// True while a source is being synced or waits to be.
    pub fn involves(&self, root: &str) -> bool {
        let s = lock(&self.shared.status);
        s.current_root.as_deref() == Some(root) || s.queued_roots.iter().any(|r| r == root)
    }

    fn publish_queue(&self, q: &VecDeque<Job>) {
        let mut s = lock(&self.shared.status);
        s.queued_roots = q
            .iter()
            .filter_map(|j| match j {
                Job::Sync { root, .. } => Some(root.clone()),
                _ => None,
            })
            .collect();
        s.lookup_queued = q.iter().any(|j| matches!(j, Job::Lookup { .. }));
    }
}

fn worker_loop(shared: &Arc<Shared>) {
    let jobs = Jobs {
        shared: Arc::clone(shared),
    };
    loop {
        let job = {
            let mut q = lock(&shared.queue);
            while q.is_empty() {
                q = shared.wake.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
            let job = q.pop_front().unwrap();
            jobs.publish_queue(&q);
            job
        };
        shared.cancel.store(false, Ordering::Relaxed);
        {
            let mut s = lock(&shared.status);
            s.running = true;
            s.done = None;
            s.total = None;
            s.detail.clear();
        }
        let cfg = shared.config.read().unwrap_or_else(PoisonError::into_inner).clone();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &job {
            Job::Sync { root, full } => run_sync(&jobs, &cfg, root, *full),
            Job::Lookup { retry_misses } => run_lookup(&jobs, &cfg, *retry_misses),
        }));
        let message = match outcome {
            Ok(Ok(msg)) => msg,
            Ok(Err(e)) => format!("{e:#}"),
            Err(_) => "The job stopped unexpectedly.".into(),
        };
        jobs.log(&message);
        // Lock order is always queue, then status (see `enqueue`); read the
        // queue first rather than nesting the other way round.
        let more = !lock(&shared.queue).is_empty();
        let mut s = lock(&shared.status);
        s.message = message;
        s.current_root = None;
        s.label.clear();
        s.detail.clear();
        s.done = None;
        s.total = None;
        s.running = more;
    }
}

impl Jobs {
    fn log(&self, msg: &str) {
        let mut s = lock(&self.shared.status);
        s.log.push_back(msg.to_string());
        while s.log.len() > LOG_LINES {
            s.log.pop_front();
        }
    }

    fn cancelled(&self) -> bool {
        self.shared.cancel.load(Ordering::Relaxed)
    }
}

struct SyncProgress<'a> {
    jobs: &'a Jobs,
    root: String,
}

impl Progress for SyncProgress<'_> {
    fn log(&self, msg: &str) {
        self.jobs.log(msg);
    }
    fn update(&self, _root: &str, stats: &CrawlStats) {
        let mut s = lock(&self.jobs.shared.status);
        s.label = format!("{} · syncing", self.root);
        s.detail = format!("{} videos · {} folders", group(stats.files), group(stats.dirs));
    }
    fn cancelled(&self) -> bool {
        self.jobs.cancelled()
    }
}

fn run_sync(jobs: &Jobs, cfg: &Config, name: &str, full: bool) -> Result<String> {
    let Some(root) = cfg.root(name).cloned() else {
        return Ok(format!("{name}: no longer configured"));
    };
    {
        let mut s = lock(&jobs.shared.status);
        s.current_root = Some(name.to_string());
        s.label = format!("{name} · syncing");
        s.detail = "starting".into();
    }
    jobs.log(&format!("Syncing {name} ({})", root.location()));
    let mut index = Index::open(cfg.db_file())?;
    let progress = SyncProgress {
        jobs,
        root: name.to_string(),
    };
    let mode = if full { CrawlMode::Fresh } else { CrawlMode::Auto };
    let result = crawler::crawl_root(cfg, &mut index, &root, mode, &progress);

    // Regroup whatever is on record, even after a stop or error: files
    // already written should show up.
    lock(&jobs.shared.status).detail = "organising".into();
    let titles = library::rebuild_titles(&mut index, &root)?;

    let (status, error, msg) = match &result {
        Ok(s) if s.cancelled => ("stopped", None, format!("{name}: stopped, {titles} titles so far")),
        Ok(s) if s.errors > 0 => (
            "error",
            Some(format!("{} folders could not be read", s.errors)),
            format!("{name}: {titles} titles, {} folders failed", s.errors),
        ),
        Ok(_) => ("ok", None, format!("{name}: {titles} titles")),
        Err(e) => ("error", Some(format!("{e:#}")), format!("{name}: {e:#}")),
    };
    set_source_state(&index, name, status, error.as_deref())?;

    let cancelled = matches!(&result, Ok(s) if s.cancelled);
    if !cancelled && cfg.tmdb.auto_lookup && !cfg.tmdb.api_key.is_empty() {
        jobs.enqueue(Job::Lookup { retry_misses: false });
    }
    Ok(msg)
}

pub fn set_source_state(index: &Index, root: &str, status: &str, error: Option<&str>) -> Result<()> {
    index.conn().execute(
        "INSERT INTO sources_state (root, last_sync, status, error) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(root) DO UPDATE SET last_sync = excluded.last_sync,
             status = excluded.status, error = excluded.error",
        params![root, unix_now(), status, error],
    )?;
    Ok(())
}

impl LookupProgress for Jobs {
    fn progress(&self, done: usize, total: usize) {
        let mut s = lock(&self.shared.status);
        s.label = "Looking up movie info".into();
        s.detail = format!("{} / {}", group(done), group(total));
        s.done = Some(done);
        s.total = Some(total);
    }
    fn cancelled(&self) -> bool {
        Jobs::cancelled(self)
    }
    fn log(&self, msg: &str) {
        Jobs::log(self, msg);
    }
}

fn run_lookup(jobs: &Jobs, cfg: &Config, retry_misses: bool) -> Result<String> {
    lock(&jobs.shared.status).label = "Looking up movie info".into();
    let mut index = Index::open(cfg.db_file())?;
    let (ok, none, failed) = tmdb::lookup_all(&cfg.tmdb, &mut index, retry_misses, jobs)?;
    Ok(format!(
        "Movie info: {ok} matched, {none} not found{}",
        if failed > 0 { format!(", {failed} failed") } else { String::new() }
    ))
}

/// Queues a sync of every source.
pub fn sync_all(jobs: &Jobs, roots: &[Root], full: bool) {
    for r in roots {
        jobs.enqueue(Job::Sync {
            root: r.name.clone(),
            full,
        });
    }
}

/// 11575 -> "11,575".
pub fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn grouping() {
        assert_eq!(super::group(11575), "11,575");
        assert_eq!(super::group(999), "999");
        assert_eq!(super::group(1234567), "1,234,567");
    }
}

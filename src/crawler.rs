//! Building the index: a breadth-first crawl over HTTP directory listings,
//! and a plain directory walk for local folders.
//!
//! The HTTP crawl runs level by level: fetch every directory of the current
//! depth concurrently, collect the results, then write them to SQLite in one
//! transaction, then descend. That ordering is structural, not stylistic.
//! `rusqlite::Connection` is `Send` but not `Sync`, so a `&Connection` cannot
//! live inside a future that runs concurrently with others. Rather than wrap
//! it in a lock or an async SQLite layer, the code keeps the two phases apart:
//! `fetch_level` is the only async code and never sees the database, and
//! `crawl_http` is plain synchronous code that drives it with `block_on`.
//! One transaction per level is also much faster than a lock per row.

use std::collections::{HashSet, VecDeque};
use std::io::ErrorKind;
use std::path::{Path, MAIN_SEPARATOR};
use std::time::Duration;

use anyhow::{bail, Result};
use futures::stream::{self, StreamExt};
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use scraper::{ElementRef, Html, Selector};
use serde::Serialize;
use url::Url;

use crate::config::{Config, Root, RootKind};
use crate::index::{unix_now, FileRow, ImageRow, Index};
use crate::naming;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: Url,
    /// Percent-decoded last path segment, without the trailing `/`.
    pub name: String,
    pub size: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Listing {
    pub dirs: Vec<Link>,
    pub files: Vec<Link>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct CrawlStats {
    pub resumed: bool,
    pub cancelled: bool,
    pub dirs: usize,
    pub files: usize,
    pub errors: usize,
    pub skipped_dirs: usize,
    pub too_deep: usize,
    pub pruned: usize,
}

/// How a crawl reports progress and learns it should stop. The CLI prints
/// to stderr; the desktop app updates its status panel and Stop button.
pub trait Progress {
    fn log(&self, msg: &str);
    fn update(&self, _root: &str, _stats: &CrawlStats) {}
    fn cancelled(&self) -> bool {
        false
    }
}

pub struct StderrProgress;

impl Progress for StderrProgress {
    fn log(&self, msg: &str) {
        eprintln!("{msg}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrawlMode {
    /// Continue from saved checkpoints (the CLI default).
    Resume,
    /// Forget checkpoints and re-walk everything (`--fresh`).
    Fresh,
    /// Resume if an earlier crawl was interrupted, otherwise re-walk to pick
    /// up new files. What the app's "Update library" button does.
    Auto,
}

pub fn client(cfg: &Config) -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(cfg.timeout_secs))
        .gzip(true)
        .user_agent(concat!("medialib/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

pub fn crawl_root(
    cfg: &Config,
    index: &mut Index,
    root: &Root,
    mode: CrawlMode,
    progress: &dyn Progress,
) -> Result<CrawlStats> {
    match root.kind()? {
        RootKind::Http(base) => crawl_http(cfg, index, root, &base, mode, progress),
        // Local walks are fast enough to always redo in full.
        RootKind::Local(dir) => crawl_local(cfg, index, root, &dir, progress),
    }
}

fn crawl_http(
    cfg: &Config,
    index: &mut Index,
    root: &Root,
    base: &Url,
    mode: CrawlMode,
    progress: &dyn Progress,
) -> Result<CrawlStats> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let client = client(cfg)?;
    let started = unix_now();
    let mut stats = CrawlStats::default();

    let fresh = match mode {
        CrawlMode::Fresh => true,
        CrawlMode::Resume => false,
        CrawlMode::Auto => index.pending(&root.name)?.is_empty(),
    };
    if fresh {
        index.reset_crawl_state(&root.name)?;
    }
    let pending = index.pending(&root.name)?;
    stats.resumed = !fresh && !pending.is_empty();
    if pending.is_empty() && !index.is_visited(base.as_str())? {
        // First crawl of this root. Queue it in the database too, so a failed
        // fetch of the root itself is retried by the next run.
        index.enqueue(&root.name, base.as_str(), 0)?;
    }
    let mut frontier: Vec<(Url, usize)> = index
        .pending(&root.name)?
        .into_iter()
        .filter_map(|(u, d)| Url::parse(&u).ok().map(|u| (u, d)))
        .collect();

    while !frontier.is_empty() {
        // Filter against `visited` (one directory reachable from two parents)
        // and dedupe within the level. Unless --fresh, `visited` was cleared.
        let mut seen = HashSet::new();
        let mut level = Vec::with_capacity(frontier.len());
        for (url, depth) in frontier.drain(..) {
            if seen.insert(url.to_string()) && !index.is_visited(url.as_str())? {
                level.push((url, depth));
            }
        }
        if level.is_empty() {
            break;
        }
        let depth_label = level.iter().map(|(_, d)| *d).min().unwrap_or(0);
        let mut next = Vec::new();
        let (mut level_dirs, mut level_files, mut level_errors) = (0, 0, 0);

        // A level can be thousands of folders wide (one per movie), so it is
        // fetched and written in chunks: progress moves, Stop responds, and
        // each chunk is its own checkpoint. Cancellation is checked between
        // chunks, never in the middle of one, so nothing is half-recorded.
        for chunk in level.chunks(CHUNK) {
            if progress.cancelled() {
                stats.cancelled = true;
                break;
            }
            let results = rt.block_on(fetch_level(&client, chunk.to_vec(), cfg.concurrency));

            let mut rows = Vec::new();
            let mut images = Vec::new();
            let mut done = Vec::new();
            let mut queued = Vec::new();
            for ((url, depth), result) in results {
                let listing = match result {
                    Ok(listing) => listing,
                    Err(e) => {
                        // One bad directory must not sink a crawl of
                        // thousands. It stays in `pending`, so the next
                        // crawl retries it.
                        progress.log(&format!("warning: skipping {url}: {e:#}"));
                        level_errors += 1;
                        continue;
                    }
                };
                // Decoded, or every path in the UI reads /Shows/Breaking%20Bad/.
                let dir = percent_decode(url.path());
                for file in listing.files {
                    if is_image(&file.name) {
                        images.push(ImageRow {
                            root: root.name.clone(),
                            dir: url.to_string(),
                            url: file.url.to_string(),
                            name: file.name,
                        });
                        continue;
                    }
                    if !cfg.is_media(&file.name) {
                        continue;
                    }
                    let parsed = naming::parse(&file.name);
                    rows.push(FileRow {
                        root: root.name.clone(),
                        url: file.url.to_string(),
                        dir: dir.clone(),
                        size: file.size.and_then(|s| i64::try_from(s).ok()),
                        title: parsed.title,
                        season: parsed.season,
                        episode: parsed.episode,
                        year: parsed.year,
                        name: file.name,
                    });
                }
                for sub in listing.dirs {
                    if cfg.should_skip_dir(&sub.name) {
                        stats.skipped_dirs += 1;
                    } else if depth + 1 > cfg.max_depth {
                        stats.too_deep += 1;
                    } else {
                        queued.push((sub.url.to_string(), depth + 1));
                        next.push((sub.url, depth + 1));
                    }
                }
                done.push(url.to_string());
            }
            index.record_level(&root.name, &rows, &images, &done, &queued)?;

            level_dirs += done.len();
            level_files += rows.len();
            stats.dirs += done.len();
            stats.files += rows.len();
            progress.update(&root.name, &stats);
        }
        stats.errors += level_errors;

        progress.log(&format!(
            "[{}] depth {depth_label}: {level_dirs} dirs, {level_files} media files{}",
            root.name,
            if level_errors > 0 { format!(", {level_errors} failed") } else { String::new() }
        ));
        if stats.cancelled {
            break;
        }

        frontier = next;
        if !frontier.is_empty() && cfg.delay_ms > 0 {
            // Plain thread sleep: no futures are in flight between levels.
            std::thread::sleep(Duration::from_millis(cfg.delay_ms));
        }
    }

    // A clean full re-walk saw everything that still exists, so anything
    // older was removed upstream. After a partial or failed walk we can't
    // tell "deleted" from "not reached", so leave the rows alone.
    if fresh && stats.errors == 0 && !stats.cancelled {
        stats.pruned = index.prune(&root.name, started)?;
    }
    Ok(stats)
}

/// Folders fetched and written per step of an HTTP crawl.
const CHUNK: usize = 200;

/// Rows are written in batches of this many while walking a local folder.
const LOCAL_BATCH: usize = 500;

/// Walks a folder on this machine. Every run is a full walk (a local disk
/// lists thousands of directories a second), so there are no checkpoints:
/// a clean walk simply prunes whatever it didn't see.
fn crawl_local(
    cfg: &Config,
    index: &mut Index,
    root: &Root,
    dir: &Path,
    progress: &dyn Progress,
) -> Result<CrawlStats> {
    if !dir.is_dir() {
        bail!("folder not found: {}", dir.display());
    }
    let started = unix_now();
    let mut stats = CrawlStats::default();
    let mut rows = Vec::new();
    let mut images = Vec::new();
    let mut queue = VecDeque::from([(dir.to_path_buf(), 0usize)]);

    while let Some((path, depth)) = queue.pop_front() {
        if progress.cancelled() {
            stats.cancelled = true;
            break;
        }
        let entries = match std::fs::read_dir(&path) {
            Ok(entries) => entries,
            // Protected folders aren't failures, just not ours to read. Only
            // real errors count, because errors also block pruning.
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                stats.skipped_dirs += 1;
                continue;
            }
            Err(e) => {
                progress.log(&format!("warning: skipping {}: {e}", path.display()));
                stats.errors += 1;
                continue;
            }
        };
        let mut dir_label = path.to_string_lossy().into_owned();
        if !dir_label.ends_with(MAIN_SEPARATOR) {
            dir_label.push(MAIN_SEPARATOR);
        }
        let dir_url = Url::from_directory_path(&path).map(|u| u.to_string()).unwrap_or_default();

        for entry in entries.flatten() {
            // `file_type` doesn't follow links, so symlinks and junctions are
            // neither dirs nor files here. That keeps link loops out entirely.
            let Ok(kind) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if kind.is_dir() {
                if is_system_dir(&name) || cfg.should_skip_dir(&name) {
                    stats.skipped_dirs += 1;
                } else if depth + 1 > cfg.max_depth {
                    stats.too_deep += 1;
                } else {
                    queue.push_back((entry.path(), depth + 1));
                }
            } else if kind.is_file() && is_image(&name) {
                let Ok(url) = Url::from_file_path(entry.path()) else { continue };
                images.push(ImageRow {
                    root: root.name.clone(),
                    dir: dir_url.clone(),
                    url: url.to_string(),
                    name,
                });
            } else if kind.is_file() && cfg.is_media(&name) {
                let Ok(url) = Url::from_file_path(entry.path()) else { continue };
                let parsed = naming::parse(&name);
                rows.push(FileRow {
                    root: root.name.clone(),
                    url: url.to_string(),
                    dir: dir_label.clone(),
                    size: entry.metadata().ok().and_then(|m| i64::try_from(m.len()).ok()),
                    title: parsed.title,
                    season: parsed.season,
                    episode: parsed.episode,
                    year: parsed.year,
                    name,
                });
            }
        }
        stats.dirs += 1;

        if rows.len() + images.len() >= LOCAL_BATCH {
            index.record_level(&root.name, &rows, &images, &[], &[])?;
            stats.files += rows.len();
            rows.clear();
            images.clear();
            progress.update(&root.name, &stats);
        }
    }
    index.record_level(&root.name, &rows, &images, &[], &[])?;
    stats.files += rows.len();

    if stats.errors == 0 && !stats.cancelled {
        stats.pruned = index.prune(&root.name, started)?;
    }
    progress.log(&format!(
        "[{}] {} folders, {} media files",
        root.name, stats.dirs, stats.files
    ));
    progress.update(&root.name, &stats);
    Ok(stats)
}

/// Images are kept as poster candidates for the folder they sit in.
pub fn is_image(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| {
        matches!(ext.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp")
    })
}

/// Windows bookkeeping folders found at drive roots.
fn is_system_dir(name: &str) -> bool {
    name.starts_with('$') || name.eq_ignore_ascii_case("System Volume Information")
}

/// Fetches one BFS level concurrently. Deliberately takes no `Index`.
async fn fetch_level(
    client: &Client,
    level: Vec<(Url, usize)>,
    concurrency: usize,
) -> Vec<((Url, usize), Result<Listing>)> {
    stream::iter(level)
        .map(|(url, depth)| async move {
            let result = fetch_listing(client, &url).await;
            ((url, depth), result)
        })
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await
}

async fn fetch_listing(client: &Client, url: &Url) -> Result<Listing> {
    let resp = client.get(url.clone()).send().await?.error_for_status()?;
    // Check before reading the body: if a "directory" turns out to be a
    // multi-gigabyte video, dropping the response here never downloads it.
    let ctype = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !ctype.contains("text/html") {
        bail!("not an HTML directory listing (Content-Type: {ctype:?})");
    }
    let html = resp.text().await?;
    Ok(parse_listing(url, &html))
}

/// Extracts the files and subdirectories linked from a listing page.
///
/// Works on nginx `autoindex`, Apache `mod_autoindex`, Caddy `browse`,
/// lighttpd and `python -m http.server`, which all agree on one thing: each
/// entry is an `<a href>` relative to the directory, with a trailing `/` for
/// subdirectories.
pub fn parse_listing(base: &Url, html: &str) -> Listing {
    let doc = Html::parse_document(html);
    let anchors = Selector::parse("a[href]").expect("static selector");
    let mut seen = HashSet::new();
    let mut listing = Listing::default();

    for a in doc.select(&anchors) {
        let href = a.value().attr("href").unwrap_or("").trim();
        // Column-sort links (?C=N;O=D), in-page anchors, parent, site root.
        if href.is_empty()
            || href.starts_with('?')
            || href.starts_with('#')
            || href == "../"
            || href == "/"
        {
            continue;
        }
        let Ok(mut url) = base.join(href) else {
            continue;
        };
        url.set_fragment(None);
        // Stay on the same server and strictly below the base directory.
        // Without this, one offsite link and the crawler walks the internet;
        // the path check also catches absolute parent links like Apache's
        // "Parent Directory" -> /Movies/ that slip past the `../` test.
        if !same_origin(base, &url) || url.query().is_some() {
            continue;
        }
        let (is_dir, name) = {
            let Some(rest) = url.path().strip_prefix(base.path()) else {
                continue;
            };
            let raw = rest.trim_end_matches('/').rsplit('/').next().unwrap_or("");
            if raw.is_empty() || raw == "." || raw == ".." {
                continue;
            }
            (rest.ends_with('/'), percent_decode(raw))
        };
        // Some listings link both an icon and the name to the same target.
        if !seen.insert(url.to_string()) {
            continue;
        }
        if is_dir {
            listing.dirs.push(Link { url, name, size: None });
        } else {
            let size = listed_size(&a);
            listing.files.push(Link { url, name, size });
        }
    }
    listing
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Best-effort file size from the text around a link. `None` is normal:
/// `python -m http.server`, for one, never shows sizes.
fn listed_size(a: &ElementRef) -> Option<u64> {
    // nginx / Apache <pre> layout: `<a>name</a>   21-Oct-2024 10:00   1234`.
    // We read the text node right after the link instead of searching the
    // page for the filename, because nginx truncates long display names
    // ("Some Long Na..>") so the name often isn't in the text at all.
    if let Some(text) = a.next_sibling().and_then(|n| n.value().as_text()) {
        let line = text.split('\n').next().unwrap_or("");
        if let Some(size) = line.split_whitespace().last().and_then(parse_size) {
            return Some(size);
        }
    }
    // Table layouts (Apache HTMLTable, Caddy, lighttpd): scan the cells after
    // the link's cell. Caddy puts the exact byte count in `data-size`.
    let cell = a
        .parent()
        .and_then(ElementRef::wrap)
        .filter(|e| e.value().name() == "td")?;
    for sibling in cell.next_siblings().filter_map(ElementRef::wrap) {
        if let Some(n) = sibling.value().attr("data-size").and_then(|v| v.parse().ok()) {
            return Some(n);
        }
        if let Some(n) = parse_size(sibling.text().collect::<String>().trim()) {
            return Some(n);
        }
    }
    None
}

/// Parses `1234`, `734M`, `1.2G`, `12K`, `3.1GiB`, and h5ai's `208232 KB`.
/// Binary multiples, as nginx and Apache use.
fn parse_size(token: &str) -> Option<u64> {
    let compact: String = token.split_whitespace().collect();
    let t = compact.as_str();
    let t = t
        .strip_suffix("iB")
        .or_else(|| t.strip_suffix('B'))
        .unwrap_or(t);
    let (num, shift) = match t.chars().last()? {
        'K' | 'k' => (&t[..t.len() - 1], 10),
        'M' | 'm' => (&t[..t.len() - 1], 20),
        'G' | 'g' => (&t[..t.len() - 1], 30),
        'T' | 't' => (&t[..t.len() - 1], 40),
        _ => (t, 0),
    };
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let value: f64 = num.parse().ok()?;
    Some((value * (1u64 << shift) as f64) as u64)
}

/// Decodes `%XX` escapes. Invalid UTF-8 is replaced rather than rejected,
/// since this is only used for display and matching.
pub fn percent_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let (Some(h), Some(l)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            ) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Most servers percent-encode UTF-8, but some (h5ai on Windows, old
    // Apache setups) send Windows-1252 bytes: "L%E9on". Bytes that aren't
    // valid UTF-8 are read as Latin-1 rather than shown as "�".
    match String::from_utf8(out) {
        Ok(s) => s,
        Err(e) => e.into_bytes().into_iter().map(char::from).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed copy of a real nginx autoindex page.
    const NGINX: &str = r#"<html>
<head><title>Index of /Shows/</title></head>
<body>
<h1>Index of /Shows/</h1><hr><pre><a href="../">../</a>
<a href="Breaking%20Bad/">Breaking Bad/</a>                                      21-Oct-2024 10:00                   -
<a href="The.Wire.S02E05.720p.mkv">The.Wire.S02E05.720p.mkv</a>                           21-Oct-2024 10:00           734003200
<a href="Some%20Documentary%20With%20A%20Very%20Long%20Name.mkv">Some Documentary With A Very Long Name.m..&gt;</a> 21-Oct-2024 10:00    1.5G
<a href="http://elsewhere.example/Shows/">mirror</a>
<a href="/Movies/">movies</a>
<a href="/">home</a>
</pre><hr></body>
</html>
"#;

    fn base() -> Url {
        Url::parse("http://media.local/Shows/").unwrap()
    }

    #[test]
    fn nginx_listing() {
        let l = parse_listing(&base(), NGINX);
        assert_eq!(l.dirs.len(), 1);
        assert_eq!(l.dirs[0].name, "Breaking Bad");
        assert_eq!(l.dirs[0].url.as_str(), "http://media.local/Shows/Breaking%20Bad/");

        assert_eq!(l.files.len(), 2);
        assert_eq!(l.files[0].name, "The.Wire.S02E05.720p.mkv");
        assert_eq!(l.files[0].size, Some(734003200));
        assert_eq!(l.files[1].name, "Some Documentary With A Very Long Name.mkv");
        assert_eq!(l.files[1].size, Some((1.5 * (1u64 << 30) as f64) as u64));
    }

    #[test]
    fn rejects_parent_and_offsite_links() {
        let l = parse_listing(&base(), NGINX);
        let all: Vec<&str> = l.dirs.iter().chain(&l.files).map(|e| e.url.as_str()).collect();
        assert!(all.iter().all(|u| u.starts_with("http://media.local/Shows/")), "{all:?}");
        assert!(!all.iter().any(|u| u.contains("elsewhere")));
        assert!(!all.contains(&"http://media.local/"));

        let tricky = r#"<a href="../">up</a><a href="https://evil.example/x.mkv">x</a>
                        <a href="Season%201/../../Movies/">sneaky</a><a href="?C=N;O=D">sort</a>"#;
        let l = parse_listing(&base(), tricky);
        assert!(l.dirs.is_empty() && l.files.is_empty(), "{l:?}");
    }

    #[test]
    fn python_http_server_listing_has_no_sizes() {
        let html = r#"<ul><li><a href="Arrival%20%282016%29.mp4">Arrival (2016).mp4</a></li>
                      <li><a href="Shows/">Shows/</a></li></ul>"#;
        let l = parse_listing(&base(), html);
        assert_eq!(l.files[0].name, "Arrival (2016).mp4");
        assert_eq!(l.files[0].size, None);
        assert_eq!(l.dirs[0].name, "Shows");
    }

    #[test]
    fn table_listing_sizes() {
        let html = r#"<table><tr><td><a href="a.mkv">a.mkv</a></td><td>2024-10-21 10:00</td><td>12K</td></tr>
                      <tr><td><a href="b.mkv"><span>b.mkv</span></a></td><td class="size" data-size="999">999 B</td></tr></table>"#;
        let l = parse_listing(&base(), html);
        assert_eq!(l.files[0].size, Some(12 * 1024));
        assert_eq!(l.files[1].size, Some(999));
    }

    #[test]
    fn decoding() {
        assert_eq!(percent_decode("/Shows/Breaking%20Bad/"), "/Shows/Breaking Bad/");
        assert_eq!(percent_decode("caf%C3%A9%2"), "café%2");
        assert_eq!(percent_decode("L%E9on"), "Léon");
        assert_eq!(parse_size("-"), None);
        assert_eq!(parse_size("734M"), Some(734 << 20));
        assert_eq!(parse_size("208232 KB"), Some(208232 << 10));
        assert_eq!(parse_size("2024-10-21 10:00"), None);
    }
}

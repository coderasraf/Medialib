//! SQLite storage: the file table, its FTS5 search index, and crawl checkpoints.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS files (
    id       INTEGER PRIMARY KEY,
    root     TEXT NOT NULL,
    url      TEXT NOT NULL UNIQUE,
    name     TEXT NOT NULL,
    dir      TEXT NOT NULL,
    size     INTEGER,
    title    TEXT NOT NULL,
    season   INTEGER,
    episode  INTEGER,
    year     INTEGER,
    seen_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS files_root_seen ON files(root, seen_at);

-- External-content FTS5: the index points at rows of `files` instead of
-- storing its own copy of every name/dir/title. At a few hundred thousand
-- filenames that roughly halves the database. The cost is that FTS5 can no
-- longer see changes by itself, hence the triggers below.
CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
    name, dir, title,
    content='files', content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS files_ai AFTER INSERT ON files BEGIN
    INSERT INTO files_fts(rowid, name, dir, title)
    VALUES (new.id, new.name, new.dir, new.title);
END;
CREATE TRIGGER IF NOT EXISTS files_ad AFTER DELETE ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, name, dir, title)
    VALUES ('delete', old.id, old.name, old.dir, old.title);
END;
-- Only the indexed columns: a re-crawl touches `seen_at` on every row, and
-- rewriting the FTS entry for that would be pure churn.
CREATE TRIGGER IF NOT EXISTS files_au AFTER UPDATE OF name, dir, title ON files BEGIN
    INSERT INTO files_fts(files_fts, rowid, name, dir, title)
    VALUES ('delete', old.id, old.name, old.dir, old.title);
    INSERT INTO files_fts(rowid, name, dir, title)
    VALUES (new.id, new.name, new.dir, new.title);
END;

-- Directories whose files have been written. Lets an interrupted crawl resume.
CREATE TABLE IF NOT EXISTS visited (
    url     TEXT PRIMARY KEY,
    root    TEXT NOT NULL,
    done_at INTEGER NOT NULL
);
-- Directories discovered but not yet fetched: the persisted BFS frontier.
-- `visited` alone can't resume a crawl, because the children of a finished
-- directory are only known by re-fetching it.
CREATE TABLE IF NOT EXISTS pending (
    url   TEXT PRIMARY KEY,
    root  TEXT NOT NULL,
    depth INTEGER NOT NULL
);

-- Image files seen next to the media (posters such as a_AL_.jpg).
CREATE TABLE IF NOT EXISTS images (
    id      INTEGER PRIMARY KEY,
    root    TEXT NOT NULL,
    dir     TEXT NOT NULL,          -- URL of the folder holding the image
    url     TEXT NOT NULL UNIQUE,
    name    TEXT NOT NULL,
    seen_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS images_dir ON images(dir);

-- A movie or a show: files grouped by library::rebuild_titles. (root, key)
-- is stable across re-syncs, so watchlist entries and looked-up info stay
-- attached to the same title.
CREATE TABLE IF NOT EXISTS titles (
    id           INTEGER PRIMARY KEY,
    root         TEXT NOT NULL,
    key          TEXT NOT NULL,
    kind         TEXT NOT NULL,     -- 'movie' | 'show'
    name         TEXT NOT NULL,
    year         INTEGER,
    year_end     INTEGER,
    ongoing      INTEGER NOT NULL DEFAULT 0,
    quality      TEXT,
    audio        TEXT,              -- 'dual' | 'multi'
    platform     TEXT,
    folder       TEXT,              -- top-level folder under the source
    dir_url      TEXT NOT NULL,
    poster       TEXT,              -- image from the title's own folder
    file_count   INTEGER NOT NULL DEFAULT 0,
    season_count INTEGER NOT NULL DEFAULT 0,
    total_size   INTEGER,
    added_at     INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    -- TMDB. tmdb_status: NULL = not looked up yet, 'ok', 'none', 'error'.
    tmdb_status  TEXT,
    tmdb_id      INTEGER,
    tmdb_title   TEXT,
    overview     TEXT,
    tagline      TEXT,
    rating       REAL,
    genres       TEXT,              -- ",Action,Drama," for LIKE filters
    country      TEXT,
    credits      TEXT,
    tmdb_poster  TEXT,
    backdrop     TEXT,
    runtime      INTEGER,
    language     TEXT,
    network      TEXT,
    tmdb_ongoing INTEGER,
    UNIQUE(root, key)
);
CREATE INDEX IF NOT EXISTS titles_kind ON titles(kind, added_at);

-- Plain (not external-content) FTS: titles are few and rebuilt per source.
CREATE VIRTUAL TABLE IF NOT EXISTS titles_fts USING fts5(
    name, alt, folder,
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TABLE IF NOT EXISTS episode_meta (
    title_id INTEGER NOT NULL,
    season   INTEGER NOT NULL,
    episode  INTEGER NOT NULL,
    name     TEXT,
    overview TEXT,
    air_date TEXT,
    runtime  INTEGER,
    still    TEXT,
    PRIMARY KEY (title_id, season, episode)
);
CREATE TABLE IF NOT EXISTS season_fetched (
    title_id INTEGER NOT NULL,
    season   INTEGER NOT NULL,
    PRIMARY KEY (title_id, season)
);

CREATE TABLE IF NOT EXISTS watchlist (
    title_id INTEGER PRIMARY KEY,
    added_at INTEGER NOT NULL
);
-- Latest play per file, for "Recently played" and "Up next".
CREATE TABLE IF NOT EXISTS plays (
    file_id   INTEGER PRIMARY KEY,
    title_id  INTEGER NOT NULL,
    played_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS plays_title ON plays(title_id, played_at);
CREATE TABLE IF NOT EXISTS watched (
    file_id INTEGER PRIMARY KEY,
    at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chat (
    id    INTEGER PRIMARY KEY,
    role  TEXT NOT NULL,            -- 'user' | 'assistant'
    text  TEXT NOT NULL,
    picks TEXT,                     -- JSON array of title ids
    at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sources_state (
    root       TEXT PRIMARY KEY,
    last_sync  INTEGER,
    status     TEXT,                -- 'ok' | 'error' | 'stopped'
    error      TEXT
);
"#;

// ON CONFLICT DO UPDATE rather than INSERT OR REPLACE: REPLACE deletes and
// re-inserts, which would hand the row a new `id` and force a full FTS rewrite.
const UPSERT: &str = r#"
INSERT INTO files (root, url, name, dir, size, title, season, episode, year, seen_at, added_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
ON CONFLICT(url) DO UPDATE SET
    root = excluded.root, name = excluded.name, dir = excluded.dir,
    size = excluded.size, title = excluded.title, season = excluded.season,
    episode = excluded.episode, year = excluded.year, seen_at = excluded.seen_at
"#;

const HIT_COLUMNS: &str = "f.id, f.root, f.url, f.name, f.dir, f.size, f.title, f.season, f.episode, f.year";

// bm25 weights per column (name, dir, title); lower scores rank higher.
// `dir` is weighted lowest because every file in a show's folder shares it.
const SEARCH: &str = r#"
SELECT f.id, f.root, f.url, f.name, f.dir, f.size, f.title, f.season, f.episode, f.year
FROM files_fts
JOIN files f ON f.id = files_fts.rowid
WHERE files_fts MATCH ?1
ORDER BY bm25(files_fts, 10.0, 1.0, 5.0), f.title, f.season, f.episode
LIMIT ?2
"#;

pub struct Index {
    conn: Connection,
}

/// An image file found during a crawl, ready to write.
pub struct ImageRow {
    pub root: String,
    pub dir: String,
    pub url: String,
    pub name: String,
}

/// One media file, ready to write.
pub struct FileRow {
    pub root: String,
    pub url: String,
    pub name: String,
    pub dir: String,
    pub size: Option<i64>,
    pub title: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub year: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub id: i64,
    pub root: String,
    pub url: String,
    pub name: String,
    pub dir: String,
    pub size: Option<i64>,
    pub title: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub year: Option<u32>,
}

pub struct Stats {
    pub files: i64,
    pub bytes: i64,
    pub per_root: Vec<(String, i64)>,
    pub visited: i64,
    pub pending: i64,
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Index {
    pub fn open(path: impl AsRef<Path>) -> Result<Index> {
        let path = path.as_ref();
        let conn = Connection::open(path)
            .with_context(|| format!("opening database {}", path.display()))?;
        // WAL lets `serve` keep reading while a `crawl` writes; NORMAL sync is
        // safe under WAL (a crash can lose the last commit, never corrupt).
        // journal_mode returns a row, so it has to go through query_row.
        conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0))?;
        conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
        // The app scans on one connection while searching on another; wait
        // briefly for the other's write lock instead of failing at once.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA).context("creating schema")?;
        migrate(&conn).context("upgrading schema")?;
        Ok(Index { conn })
    }

    /// For modules (library, jobs) that run their own queries.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Writes one crawl level atomically: the files found, the directories
    /// that are now finished, and the directories queued for the next level.
    /// Because it is one transaction, an interrupt never leaves a directory
    /// marked done whose files weren't written.
    pub fn record_level(
        &mut self,
        root: &str,
        files: &[FileRow],
        images: &[ImageRow],
        done: &[String],
        next: &[(String, usize)],
    ) -> Result<()> {
        let now = unix_now();
        let tx = self.conn.transaction()?;
        {
            let mut upsert = tx.prepare_cached(UPSERT)?;
            for f in files {
                upsert.execute(params![
                    f.root, f.url, f.name, f.dir, f.size, f.title, f.season, f.episode, f.year,
                    now
                ])?;
            }
            let mut image = tx.prepare_cached(
                "INSERT INTO images (root, dir, url, name, seen_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(url) DO UPDATE SET root = excluded.root, dir = excluded.dir,
                     name = excluded.name, seen_at = excluded.seen_at",
            )?;
            for i in images {
                image.execute(params![i.root, i.dir, i.url, i.name, now])?;
            }
            let mut mark = tx.prepare_cached(
                "INSERT INTO visited (url, root, done_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(url) DO UPDATE SET done_at = excluded.done_at",
            )?;
            let mut dequeue = tx.prepare_cached("DELETE FROM pending WHERE url = ?1")?;
            for url in done {
                mark.execute(params![url, root, now])?;
                dequeue.execute([url])?;
            }
            let mut enqueue = tx.prepare_cached(
                "INSERT OR IGNORE INTO pending (url, root, depth)
                 SELECT ?1, ?2, ?3 WHERE NOT EXISTS (SELECT 1 FROM visited WHERE url = ?1)",
            )?;
            for (url, depth) in next {
                enqueue.execute(params![url, root, *depth as i64])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn enqueue(&self, root: &str, url: &str, depth: usize) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO pending (url, root, depth) VALUES (?1, ?2, ?3)",
            params![url, root, depth as i64],
        )?;
        Ok(())
    }

    pub fn pending(&self, root: &str) -> Result<Vec<(String, usize)>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT url, depth FROM pending WHERE root = ?1 ORDER BY depth")?;
        let rows = stmt.query_map([root], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn is_visited(&self, url: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare_cached("SELECT 1 FROM visited WHERE url = ?1")?;
        Ok(stmt.exists([url])?)
    }

    /// Forgets crawl progress for a root, so the next crawl re-walks it.
    pub fn reset_crawl_state(&self, root: &str) -> Result<()> {
        self.conn.execute("DELETE FROM visited WHERE root = ?1", [root])?;
        self.conn.execute("DELETE FROM pending WHERE root = ?1", [root])?;
        Ok(())
    }

    /// Deletes files of `root` not seen since `before` (removed upstream).
    pub fn prune(&self, root: &str, before: i64) -> Result<usize> {
        self.conn.execute(
            "DELETE FROM images WHERE root = ?1 AND seen_at < ?2",
            params![root, before],
        )?;
        Ok(self.conn.execute(
            "DELETE FROM files WHERE root = ?1 AND seen_at < ?2",
            params![root, before],
        )?)
    }

    /// Follows a source rename everywhere its name is stored.
    pub fn rename_root(&mut self, old: &str, new: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        for table in ["files", "images", "titles", "visited", "pending", "sources_state"] {
            tx.execute(&format!("UPDATE {table} SET root = ?2 WHERE root = ?1"), [old, new])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Drops everything indexed from a root, plus its crawl checkpoints.
    pub fn remove_root(&mut self, root: &str) -> Result<usize> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM titles_fts WHERE rowid IN (SELECT id FROM titles WHERE root = ?1)",
            [root],
        )?;
        for table in ["watchlist", "plays", "episode_meta", "season_fetched"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE title_id IN (SELECT id FROM titles WHERE root = ?1)"),
                [root],
            )?;
        }
        tx.execute(
            "DELETE FROM watched WHERE file_id IN (SELECT id FROM files WHERE root = ?1)",
            [root],
        )?;
        let n = tx.execute("DELETE FROM files WHERE root = ?1", [root])?;
        for table in ["titles", "images", "visited", "pending", "sources_state"] {
            tx.execute(&format!("DELETE FROM {table} WHERE root = ?1"), [root])?;
        }
        tx.commit()?;
        Ok(n)
    }

    pub fn hit_by_url(&self, url: &str) -> Result<Option<Hit>> {
        let sql = format!("SELECT {HIT_COLUMNS} FROM files f WHERE f.url = ?1");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        Ok(stmt.query_row([url], hit_from_row).optional()?)
    }

    pub fn hit_by_id(&self, id: i64) -> Result<Option<Hit>> {
        let sql = format!("SELECT {HIT_COLUMNS} FROM files f WHERE f.id = ?1");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        Ok(stmt.query_row([id], hit_from_row).optional()?)
    }

    /// Most recently added files, for the library's empty-search view.
    pub fn recent(&self, limit: usize) -> Result<Vec<Hit>> {
        let sql = format!("SELECT {HIT_COLUMNS} FROM files f ORDER BY f.id DESC LIMIT ?1");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map([limit as i64], hit_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn search(&self, input: &str, limit: usize) -> Result<Vec<Hit>> {
        let Some(query) = fts_query(input) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare_cached(SEARCH)?;
        let rows = stmt.query_map(params![query, limit as i64], hit_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn stats(&self) -> Result<Stats> {
        let (files, bytes) = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM files",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?;
        let mut stmt = self
            .conn
            .prepare("SELECT root, COUNT(*) FROM files GROUP BY root ORDER BY root")?;
        let per_root = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let count = |sql: &str| -> Result<i64> {
            Ok(self
                .conn
                .query_row(sql, [], |r| r.get::<_, i64>(0))
                .optional()?
                .unwrap_or(0))
        };
        Ok(Stats {
            files,
            bytes,
            per_root,
            visited: count("SELECT COUNT(*) FROM visited")?,
            pending: count("SELECT COUNT(*) FROM pending")?,
        })
    }
}

/// Brings databases created by older versions up to the current schema.
/// `CREATE TABLE IF NOT EXISTS` covers new tables; new columns on existing
/// tables need an explicit ALTER.
fn migrate(conn: &Connection) -> Result<()> {
    add_column(conn, "files", "title_id", "INTEGER")?;
    if add_column(conn, "files", "added_at", "INTEGER NOT NULL DEFAULT 0")? {
        conn.execute("UPDATE files SET added_at = seen_at", [])?;
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS files_title ON files(title_id, season, episode);")?;
    Ok(())
}

/// Adds a column if it is missing; returns whether it did.
fn add_column(conn: &Connection, table: &str, column: &str, decl: &str) -> Result<bool> {
    let exists = conn
        .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?
        .exists([column])?;
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
    }
    Ok(!exists)
}

/// Maps a row selected with `HIT_COLUMNS` (or the same order) to a `Hit`.
fn hit_from_row(r: &rusqlite::Row) -> rusqlite::Result<Hit> {
    Ok(Hit {
        id: r.get(0)?,
        root: r.get(1)?,
        url: r.get(2)?,
        name: r.get(3)?,
        dir: r.get(4)?,
        size: r.get(5)?,
        title: r.get(6)?,
        season: r.get(7)?,
        episode: r.get(8)?,
        year: r.get(9)?,
    })
}

/// Turns free text into a safe FTS5 query.
///
/// FTS5 parses its MATCH argument as a query language, so punctuation in raw
/// input is syntax: `spider-man` or `what's up` is an error, not zero
/// results. Quoting each token makes it a literal phrase (the tokenizer still
/// splits `spider-man` inside the quotes), and the trailing `*` gives prefix
/// matching so results appear while the user is still typing.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(url: &str, name: &str, dir: &str, title: &str) -> FileRow {
        FileRow {
            root: "r".into(),
            url: url.into(),
            name: name.into(),
            dir: dir.into(),
            size: None,
            title: title.into(),
            season: None,
            episode: None,
            year: None,
        }
    }

    #[test]
    fn fts_query_escapes_punctuation() {
        assert_eq!(fts_query("Breaking  bad").unwrap(), r#""Breaking"* "bad"*"#);
        assert_eq!(fts_query("spider-man").unwrap(), r#""spider-man"*"#);
        assert_eq!(fts_query("(what's up?)").unwrap(), r#""what's"* "up"*"#);
        assert!(fts_query(" -- ").is_none());
    }

    #[test]
    fn upsert_search_and_punctuated_queries() {
        let mut idx = Index::open(":memory:").unwrap();
        let rows = [
            row("http://h/a.mkv", "Spider-Man.2002.mkv", "/Movies/", "Spider-Man"),
            row("http://h/b.mkv", "What's Up Doc.mkv", "/Movies/", "What's Up Doc"),
        ];
        idx.record_level("r", &rows, &[], &[], &[]).unwrap();
        // Re-crawl: same URL updates in place rather than duplicating.
        idx.record_level("r", &rows[..1], &[], &[], &[]).unwrap();
        assert_eq!(idx.stats().unwrap().files, 2);

        assert_eq!(idx.search("spider-man", 10).unwrap().len(), 1);
        assert_eq!(idx.search("spid", 10).unwrap().len(), 1);
        assert_eq!(idx.search("what's up", 10).unwrap()[0].title, "What's Up Doc");
        assert!(idx.search("\"unbalanced", 10).unwrap().is_empty());
    }

    #[test]
    fn checkpoints() {
        let mut idx = Index::open(":memory:").unwrap();
        idx.enqueue("r", "http://h/", 0).unwrap();
        idx.record_level("r", &[], &[], &["http://h/".into()], &[("http://h/a/".into(), 1)])
            .unwrap();
        assert!(idx.is_visited("http://h/").unwrap());
        assert_eq!(idx.pending("r").unwrap(), vec![("http://h/a/".to_string(), 1)]);
        // An already-visited dir is never re-queued.
        idx.record_level("r", &[], &[], &[], &[("http://h/".into(), 0)]).unwrap();
        assert_eq!(idx.pending("r").unwrap().len(), 1);
    }
}

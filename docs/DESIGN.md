# medialib — design notes

How medialib works inside, and why it is built the way it is. For what the app
does and how to use it, see the [README](../README.md).

## Configuration gotcha

**Every bare key must come before the first `[player]` or `[[roots]]`
header.** In TOML a bare key belongs to the most recent table header above
it. A `skip_dirs = [...]` placed under `[[roots]]` would be read as a field of
that root, and `skip_dirs` itself would stay empty. medialib marks those
tables `deny_unknown_fields`, so this mistake stops the config from loading
instead of quietly indexing every `sample/` folder. See
`config.example.toml`.

## Layout

```
src/config.rs     TOML config: sources, filters, player, TMDB, assistant
src/crawler.rs    BFS over directory listings (and local folder walks)
src/naming.rs     title / season / episode / year / quality / audio tags
src/index.rs      SQLite schema, FTS5 search, crawl checkpoints
src/library.rs    files grouped into movies and shows; page queries
src/tmdb.rs       TMDB lookups (posters, genres, ratings, episodes)
src/assistant.rs  Gemini watch assistant, constrained to the library
src/jobs.rs       background worker: syncs and lookups, with progress
src/player.rs     URL validation, spawning the external player
src/web.rs        axum API + UI server (loopback only)
src/ui/           the single-page UI (HTML, CSS, JS)
src/main.rs       clap CLI
src/desktop.rs    the desktop window (WebView2 via wry/tao)
```

## Design decisions

### Crawl and search are separate phases

Walking a large tree takes minutes, while a search has to come back in
milliseconds. So `crawl` writes everything into SQLite, and `search`,
`play` and `serve` only read from that file and never touch the network.

### Fetch, then write

The crawl is breadth-first and runs one depth level at a time:

1. Drop directories already in `visited` (unless `--fresh`).
2. Fetch the whole level concurrently with
   `stream::iter(..).map(..).buffer_unordered(concurrency)`.
3. Collect the results, then write them to SQLite in one transaction.
4. Queue the subdirectories, sleep `delay_ms`, and repeat.

This ordering is forced by the types. `rusqlite::Connection` is `Send` but not
`Sync`, so a `&Connection` can't sit inside futures that run concurrently.
The usual workarounds are an `Arc<Mutex<Connection>>` locked for every row, or
an async SQLite wrapper. Both add complexity, and both are slower than one
transaction per level. medialib avoids the problem instead: `crawl_root` is
ordinary synchronous code that calls `block_on(fetch_level(..))`, and
`fetch_level` is never given the database. The connection stays out of every
future.

Before parsing a response, the crawler checks that its `Content-Type`
contains `text/html`. If a link that looked like a directory turns out to be
a video file, the response is dropped before its body is downloaded. Links
are only followed when they stay on the same scheme, host and port and under
the root's path. Without that check, a single offsite link would send the
crawler out onto the open internet.

### Resume checkpoints: `visited` + `pending`

The checkpoint is two tables:

- `visited(url, root, done_at)` lists the directories whose files have been
  written.
- `pending(url, root, depth)` is the BFS frontier: directories that have been
  discovered but not yet fetched.

`visited` alone isn't enough to resume. A directory's children are only known
by fetching it, so an interrupted crawl that skipped every visited directory
would never reach the unvisited ones below them. Saving the frontier fixes
that.

Each level is committed in a single transaction. That transaction upserts the
level's files, marks its directories visited, removes them from `pending`, and
queues their children. An interrupted crawl therefore can't leave a directory
marked done when its files were never written, and the next run continues
from exactly the saved frontier.

`max_depth` stays as a separate guard. A symlink loop produces URLs that keep
getting longer and are all different, so `visited` never catches it.

### External-content FTS5

```sql
CREATE VIRTUAL TABLE files_fts USING fts5(
  name, dir, title,
  content='files', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);
```

In external-content mode, the FTS index refers to rows in `files` instead of
keeping its own copy of the text. With a few hundred thousand filenames, that
roughly halves the size of the database. The trade-off is that FTS5 no longer
notices changes to `files` by itself, so `AFTER INSERT/DELETE/UPDATE` triggers
keep it in sync. The update trigger fires only for the indexed columns. A
re-crawl updates `seen_at` on every row, and rewriting the FTS entries for
that would be wasted work.

Ranking uses `bm25(files_fts, 10.0, 1.0, 5.0)`. The filename gets the most
weight and the directory the least, because every file in a show's folder
shares the same `dir`.

Writes use `INSERT ... ON CONFLICT(url) DO UPDATE` rather than
`INSERT OR REPLACE`. `REPLACE` deletes the row and inserts it again, which
would give it a new `id` and force a full FTS rewrite.

### Escaping search input

FTS5 parses its `MATCH` argument as a query language. A raw `spider-man` or
`what's up` is a syntax error, not a search that finds nothing. User input is
therefore split on whitespace, stripped of punctuation at each token's edges,
quoted, and given a trailing `*` for prefix matching:
`"breaking"* "bad"*`.

### Filename parsing

`naming::parse` normalises `.` and `_` to spaces. It then cuts at the earliest
of three markers: an episode marker (`S01E02`, `1x02`), a year that isn't at
position 0, or the first release tag (`1080p`, `BluRay`, `x265`, `REMUX`, …).
Everything before the cut is the title. When several years appear, the last
one before the other markers is used, so `Blade Runner 2049 2017` keeps the
2049. Every regex is compiled once in a `OnceLock`. Compiling a regex per
filename would take longer than the rest of the crawl's CPU work.

### Movies and shows from folders

`library::rebuild_titles` runs after every sync and groups a source's files
into titles. For a *Movies* source, a folder holding one movie (the common
`Name (2024) 720p [Dual Audio]/` layout) is one title named by the folder;
loose files in a year folder each name themselves. For a *TV shows* source,
it walks up past `Season N` folders to the show folder, which names the show
(`Show (TV Series 2023– )` also gives the year and "ongoing"). Episode numbers
come from `S01E02`, `1x02`, `E02` or a trailing number. Quality, Dual/Multi
audio and platform tags (NF, AMZN, DSNP…) are read from folder and file
names, and an image in the folder becomes the poster. Titles are keyed by
(source, folder), so re-syncs keep watchlist entries and looked-up info
attached.

### The assistant can't invent titles

The model never decides on its own what to show. A first Gemini call turns
the request into title guesses and filters; the app finds the titles that
really exist (exact title matches, then genre/language/year matches ranked
by rating); a second call picks from that candidate list by id. Ids that
weren't offered are dropped, so every pick links to something playable.

### Player safety

`player::play` accepts only `http` and `https` URLs that have a host. The
reason isn't shell injection: `std::process::Command` doesn't use a shell, so
a crafted filename is just bytes in one argument. The check stops the player
from being pointed at `file:///…`, `javascript:`, or any other URL scheme the
player understands. Local files are handed over as plain paths, and only
when they are in the index. The API refuses anything that isn't in the
index, opens only allow-listed help links, and rejects requests with a
non-loopback `Host` header to block DNS rebinding.

## Tests

```sh
cargo test
```

The tests cover filename and folder parsing, listing parsing (nginx, h5ai,
python and table layouts, including rejection of parent and offsite links),
grouping DhakaFlix-style layouts into titles, FTS escaping, upserts,
checkpoints, config key ordering, TMDB matching, URL scheme validation and
playability.

## Ideas not built yet

- `ffmpeg` remux/transcode streamed through the local server for files the
  browser refuses, which would remove the playability problem entirely
- Per-file resume position (needs VLC's HTTP interface to read it back)

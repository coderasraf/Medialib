# medialib

A desktop media library for the files on HTTP directory-listing servers (and
folders on your own PC). It indexes them into a poster library of movies and
TV shows with filters, show pages with seasons and episodes, a watchlist,
watch history, movie info from TMDB and an AI watch assistant, and plays
everything in VLC or inside the app.

Any server that produces a directory listing works: h5ai, nginx `autoindex`,
Apache `mod_autoindex`, Caddy `file_server browse`, lighttpd, or
`python -m http.server`. Nothing about a particular host is hardcoded; you add
links in the app.

Use it against servers you have legitimate access to, such as your own NAS,
your ISP's media server, a Jellyfin share, a mirror, or a public archive.

## Desktop app

Start **medialib** from the Start Menu or the desktop shortcut.

1. **Settings → Sources → Add a link.** Paste a category folder (for example
   `http://172.16.50.14/DHAKA-FLIX-14/English Movies (1080p)/`), pick
   *Movies* or *TV shows*, and press **Add and sync**. *Check link* tests a
   link first and guesses the type. The folder button picks a folder on this
   PC instead.
2. Syncing runs in the background. The pill at the top right shows progress;
   its square button stops it. **Sync** on a source picks up new files later;
   **Full rescan** re-walks everything and drops files that are gone.
3. **Movies / TV Shows** are poster grids with filters (year, genre, quality,
   platform, folder, audio, status) and sorting. Each source in the sidebar
   opens just its own titles. **Ctrl K** searches everything.
4. A title page has **Play** (the next episode for shows), **Play as
   playlist** (the rest of the show, back to back, in VLC), the watchlist
   bookmark and **Open folder**. Episodes show *Up next* and a watched tick;
   episodes released twice (say, dubbed and original) list both versions.

Optional extras, both free:

- **Settings → Movie info**: a TMDB API key adds posters, backdrops,
  genres, ratings, plots, credits and episode names. Get one at
  themoviedb.org/settings/api. *Wrong info? Fix the match* on a title page
  corrects a bad match.
- **Settings → AI assistant**: a Google Gemini key enables the ✦ button. Ask
  in any language; it only recommends titles that are in your library.

Settings, the library database and the playlist file live in
`%APPDATA%\medialib`. Nothing is written next to the program.

## Building

You need a stable Rust toolchain (2021 edition) and a C compiler (MSVC Build
Tools on Windows), because `rusqlite` compiles SQLite from source. The desktop
window uses Microsoft Edge WebView2, which ships with Windows 11.

```sh
cargo build --release
# target/release/medialib-desktop.exe   the app
# target/release/medialib.exe           the command-line tool
```

> **Windows with Smart App Control on:** Smart App Control blocks unsigned
> executables, including the build scripts cargo runs during every build, so
> the build fails with `An Application Control policy has blocked this file
> (os error 4551)`. Build under WSL, or with Smart App Control off.

## Command line

The CLI shares the library code and reads a `config.toml` (see
`config.example.toml`).

```sh
medialib crawl                     # walk every root, resuming if interrupted
medialib crawl --root nas --fresh  # re-walk one root from scratch
medialib search breaking bad       # search the local index (no network)
medialib play breaking bad s03e07  # open the top result in the player
medialib serve --port 8080         # the app's UI in a browser
medialib stats
```

All commands accept `-c/--config <path>`. The default is `config.toml`.

### Crawl behaviour

- A plain `crawl` resumes where the last one stopped. Once a crawl has
  finished, a plain `crawl` has nothing to do and reports 0 new directories.
  (The app's **Sync** resumes when a sync was interrupted, and re-walks
  otherwise.)
- `crawl --fresh` forgets the checkpoints and walks the whole tree again. Rows
  are updated in place, keyed by URL. If the walk finishes with no errors,
  files the server no longer lists are removed from the index.
- When a directory fails (timeout, 403, not HTML), the crawler logs a warning
  and moves on. The failed directory stays queued and is retried next time.

### Playing in the app vs. VLC

MP4 and WebM usually play inside the app; turn that on under **Settings →
Player**. Everything else goes to VLC. The container doesn't say which codec
is inside: Chromium (and so WebView2) has no official Matroska support, and
HEVC/x265 is unlicensed in most builds, so a typical `.mkv` x265 release
won't play in a `<video>` tag. If in-app playback fails, the app says so and
offers VLC.

Why the UI runs on a local server: the media host is usually on a LAN
address. A page served from a public origin has no route to it, an HTTPS page
can't load HTTP media (mixed content), and listing servers send no CORS
headers. A page on `http://127.0.0.1` avoids all three, and `<video>` and
`<img>` need no CORS, so posters and video load straight from the media
server.

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

## Possible extensions

- `ffmpeg` remux/transcode streamed through the local server for files the
  browser refuses, which would remove the playability problem entirely
- Per-file resume position (needs VLC's HTTP interface to read it back)

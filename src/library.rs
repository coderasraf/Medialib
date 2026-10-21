//! The library view of the index: files grouped into movies and shows
//! ("titles"), plus the queries behind the app's pages.
//!
//! Grouping runs after every sync of a source. It is a pure function of the
//! files and images on record, so it can always be re-run, and titles are
//! keyed by (source, key) so a re-sync updates them in place.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::Result;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde::Serialize;
use url::Url;

use crate::config::{Root, RootKind, SourceKind};
use crate::crawler::percent_decode;
use crate::index::{fts_query, unix_now, Index};
use crate::naming;

// ------------------------------------------------------------- grouping ---

struct FileInfo {
    id: i64,
    name: String,
    dir_url: String,
    size: Option<i64>,
    season: Option<u32>,
    episode: Option<u32>,
    added_at: i64,
}

/// What one title looks like before it is written.
#[derive(Default)]
struct Group {
    key: String,
    name: String,
    year: Option<u32>,
    year_end: Option<u32>,
    ongoing: bool,
    dir_url: String,
    folder: Option<String>,
    files: Vec<usize>,
    /// (file index, season, episode) for shows.
    episodes: Vec<(usize, Option<u32>, Option<u32>)>,
}

/// Rebuilds the titles of one source from its files. Returns the number of
/// titles it now has.
pub fn rebuild_titles(index: &mut Index, root: &Root) -> Result<usize> {
    let base = root_prefix(root)?;
    let files = load_files(index.conn(), &root.name)?;
    let images = load_images(index.conn(), &root.name)?;

    let mut groups: Vec<Group> = match root.kind {
        SourceKind::Movies => group_movies(&files, &base),
        SourceKind::Shows => group_shows(&files, &base),
    };
    groups.retain(|g| !g.files.is_empty());

    let now = unix_now();
    let kind = match root.kind {
        SourceKind::Movies => "movie",
        SourceKind::Shows => "show",
    };
    let tx = index.conn_mut().transaction()?;
    let mut keep = Vec::with_capacity(groups.len());
    {
        let mut upsert = tx.prepare_cached(
            "INSERT INTO titles (root, key, kind, name, year, year_end, ongoing, quality, audio,
                 platform, folder, dir_url, poster, file_count, season_count, total_size,
                 added_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
             ON CONFLICT(root, key) DO UPDATE SET
                 kind = excluded.kind, name = excluded.name, year = excluded.year,
                 year_end = excluded.year_end, ongoing = excluded.ongoing,
                 quality = excluded.quality, audio = excluded.audio,
                 platform = excluded.platform, folder = excluded.folder,
                 dir_url = excluded.dir_url, poster = excluded.poster,
                 file_count = excluded.file_count, season_count = excluded.season_count,
                 total_size = excluded.total_size, updated_at = excluded.updated_at,
                 -- A kind change invalidates the TMDB match.
                 tmdb_status = CASE WHEN titles.kind = excluded.kind THEN titles.tmdb_status END
             RETURNING id",
        )?;
        let mut assign = tx.prepare_cached(
            "UPDATE files SET title_id = ?1, season = ?2, episode = ?3 WHERE id = ?4",
        )?;

        for g in &groups {
            let names: Vec<&str> = g.files.iter().map(|&i| files[i].name.as_str()).collect();
            let all_text = format!("{} {}", folder_name_of(&g.dir_url), names.join(" "));
            let quality = naming::quality(&all_text);
            let audio = naming::audio(&all_text);
            let platform = names
                .iter()
                .find_map(|n| naming::platform(n))
                .or_else(|| naming::platform(&folder_name_of(&g.dir_url)));
            let poster = pick_poster(&images, &g.dir_url, g.files.first().map(|&i| &files[i].dir_url));
            let total: i64 = g.files.iter().filter_map(|&i| files[i].size).sum();
            let has_sizes = g.files.iter().any(|&i| files[i].size.is_some());
            let added = g.files.iter().map(|&i| files[i].added_at).min().unwrap_or(now);
            let updated = g.files.iter().map(|&i| files[i].added_at).max().unwrap_or(now);
            let seasons: BTreeSet<u32> = g.episodes.iter().filter_map(|e| e.1).collect();
            // Two versions of an episode (dubbed and original) are one episode.
            let item_count = if g.episodes.is_empty() {
                g.files.len()
            } else {
                let numbered: BTreeSet<(Option<u32>, u32)> =
                    g.episodes.iter().filter_map(|e| e.2.map(|n| (e.1, n))).collect();
                numbered.len() + g.episodes.iter().filter(|e| e.2.is_none()).count()
            };

            let id: i64 = upsert.query_row(
                params![
                    root.name,
                    g.key,
                    kind,
                    g.name,
                    g.year,
                    g.year_end,
                    g.ongoing,
                    quality,
                    audio,
                    platform,
                    g.folder,
                    g.dir_url,
                    poster,
                    item_count as i64,
                    seasons.len() as i64,
                    has_sizes.then_some(total),
                    added,
                    updated
                ],
                |r| r.get(0),
            )?;
            keep.push(id);
            if root.kind == SourceKind::Shows {
                for &(i, s, e) in &g.episodes {
                    assign.execute(params![id, s, e, files[i].id])?;
                }
            } else {
                for &i in &g.files {
                    assign.execute(params![id, files[i].season, files[i].episode, files[i].id])?;
                }
            }
        }
    }

    // Titles whose files are all gone, and everything hanging off them.
    let ids_json = serde_json::to_string(&keep)?;
    let stale = "SELECT id FROM titles WHERE root = ?1
                 AND id NOT IN (SELECT value FROM json_each(?2))";
    for table in ["watchlist", "plays", "episode_meta", "season_fetched"] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE title_id IN ({stale})"),
            params![root.name, ids_json],
        )?;
    }
    tx.execute(
        &format!("DELETE FROM titles_fts WHERE rowid IN ({stale})"),
        params![root.name, ids_json],
    )?;
    tx.execute(
        "DELETE FROM titles WHERE root = ?1 AND id NOT IN (SELECT value FROM json_each(?2))",
        params![root.name, ids_json],
    )?;

    // Search index for this source's titles, rebuilt wholesale.
    tx.execute(
        "DELETE FROM titles_fts WHERE rowid IN (SELECT id FROM titles WHERE root = ?1)",
        [&root.name],
    )?;
    tx.execute(
        "INSERT INTO titles_fts (rowid, name, alt, folder)
         SELECT id, name, COALESCE(tmdb_title, ''), COALESCE(folder, '') FROM titles WHERE root = ?1",
        [&root.name],
    )?;
    tx.commit()?;
    Ok(keep.len())
}

/// The decoded path prefix that `files.dir` values of this root start with.
fn root_prefix(root: &Root) -> Result<String> {
    Ok(match root.kind()? {
        RootKind::Http(url) => percent_decode(url.path()),
        RootKind::Local(path) => {
            let mut s = path.to_string_lossy().into_owned();
            if !s.ends_with(['\\', '/']) {
                s.push(std::path::MAIN_SEPARATOR);
            }
            s
        }
    })
}

fn load_files(conn: &Connection, root: &str) -> Result<Vec<FileInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, url, name, size, season, episode, added_at FROM files WHERE root = ?1 ORDER BY url",
    )?;
    let rows = stmt.query_map([root], |r| {
        let url: String = r.get(1)?;
        Ok(FileInfo {
            id: r.get(0)?,
            dir_url: parent_url(&url),
            name: r.get(2)?,
            size: r.get(3)?,
            season: r.get(4)?,
            episode: r.get(5)?,
            added_at: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn load_images(conn: &Connection, root: &str) -> Result<HashMap<String, Vec<(String, String)>>> {
    let mut stmt = conn.prepare("SELECT dir, url, name FROM images WHERE root = ?1")?;
    let mut map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let rows = stmt.query_map([root], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)))?;
    for row in rows {
        let (dir, url, name) = row?;
        map.entry(dir).or_default().push((url, name));
    }
    Ok(map)
}

/// "http://h/a/b/x.mkv" -> "http://h/a/b/".
fn parent_url(url: &str) -> String {
    match url.rfind('/') {
        Some(i) => url[..=i].to_string(),
        None => url.to_string(),
    }
}

/// Decoded last segment of a folder URL.
fn folder_name_of(dir_url: &str) -> String {
    let trimmed = dir_url.trim_end_matches('/');
    percent_decode(trimmed.rsplit('/').next().unwrap_or(""))
}

/// Folder segments between the source root and this folder, decoded.
fn rel_segments(dir_url: &str, base: &str) -> Vec<String> {
    let path = match Url::parse(dir_url) {
        Ok(u) if u.scheme() == "file" => u
            .to_file_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        Ok(u) => percent_decode(u.path()),
        Err(_) => return Vec::new(),
    };
    let rest = strip_prefix_ci(&path, base).unwrap_or("");
    rest.split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Windows paths compare case-insensitively.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s.is_char_boundary(prefix.len()) && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

/// Parent folder URL of a folder URL.
fn up_one(dir_url: &str) -> String {
    let trimmed = dir_url.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => trimmed[..=i].to_string(),
        None => dir_url.to_string(),
    }
}

fn group_movies(files: &[FileInfo], base: &str) -> Vec<Group> {
    // Files per folder, in order.
    let mut by_dir: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, f) in files.iter().enumerate() {
        by_dir.entry(f.dir_url.as_str()).or_default().push(i);
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();

    for (dir, members) in by_dir {
        let mut rel = rel_segments(dir, base);
        // "Movie (1972)/Movie-CD1/", ".../Movie-CD2/": the parts belong to
        // the parent folder's movie.
        let mut dir = dir.to_string();
        while rel.len() > 1 && rel.last().is_some_and(|f| is_part_folder(f)) {
            rel.pop();
            dir = up_one(&dir);
        }
        let dir = dir.as_str();
        let folder = rel.first().cloned();
        let parsed: Vec<naming::Parsed> = members.iter().map(|&i| naming::parse(&files[i].name)).collect();
        let distinct: BTreeSet<String> = parsed.iter().map(|p| naming::title_key(&p.title)).collect();
        // "002. The Godfather (1972)": a numbered list, not part of the title.
        let folder_parse = rel.last().map(|f| naming::parse(strip_list_number(f)));
        // "Appu (2024) 720p" names a movie; "(2024)" is just a year folder.
        let folder_names_movie = folder_parse.as_ref().is_some_and(|p| {
            p.year.is_some() && naming::title_key(&p.title).chars().any(char::is_alphabetic)
        });

        // A folder holding one movie (DhakaFlix's "Name (2024) 720p/"
        // layout, maybe with an extra or two): the folder names it. A folder
        // of many movies (loose files in a year folder): each file names
        // itself.
        let movie_folder = !rel.is_empty()
            && (distinct.len() == 1 || (folder_names_movie && distinct.len() <= 3));
        if movie_folder {
            let (name, year) = match folder_parse {
                Some(fp) if folder_names_movie => (
                    strip_rank(&fp.title, &parsed[0].title).unwrap_or(fp.title),
                    fp.year.or(parsed[0].year),
                ),
                _ => (parsed[0].title.clone(), parsed[0].year),
            };
            let key = format!("d:{dir}");
            let at = *index_of.entry(key.clone()).or_insert_with(|| {
                groups.push(Group {
                    key,
                    name,
                    year,
                    dir_url: dir.to_string(),
                    folder: if rel.len() > 1 { folder } else { None },
                    ..Group::default()
                });
                groups.len() - 1
            });
            groups[at].files.extend(members);
        } else {
            for (&i, p) in members.iter().zip(&parsed) {
                let key = format!("f:{dir}#{}#{}", naming::title_key(&p.title), p.year.unwrap_or(0));
                let at = *index_of.entry(key.clone()).or_insert_with(|| {
                    groups.push(Group {
                        key,
                        name: p.title.clone(),
                        year: p.year,
                        dir_url: dir.to_string(),
                        folder: folder.clone(),
                        ..Group::default()
                    });
                    groups.len() - 1
                });
                groups[at].files.push(i);
            }
        }
    }
    groups
}

/// "002. The Godfather" -> "The Godfather". The dot after a short number
/// is what marks list numbering; "1917" or "300 (2006)" have none.
fn strip_list_number(name: &str) -> &str {
    let digits = name.chars().take_while(char::is_ascii_digit).count();
    match name[digits..].strip_prefix(". ") {
        Some(rest) if (1..=3).contains(&digits) && !rest.trim().is_empty() => rest,
        _ => name,
    }
}

/// "Pratidwandi-CD2", "Disc 1", "CD 2": one part of a multi-part movie.
fn is_part_folder(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    let stem = n.trim_end_matches(|c: char| c.is_ascii_digit());
    if stem.len() == n.len() {
        return false; // no part number
    }
    let stem = stem.trim_end();
    ["cd", "disc", "disk"].iter().any(|tag| {
        stem.strip_suffix(tag)
            .is_some_and(|before| before.chars().last().is_none_or(|c| !c.is_alphanumeric()))
    })
}

/// Collections number their folders ("130 Batman Begins (2005)"). Drops
/// such a leading number, but only when the file inside is named without
/// it, so "1917" or "300" stay intact.
fn strip_rank(folder_title: &str, file_title: &str) -> Option<String> {
    let (num, rest) = folder_title.split_once(' ')?;
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = rest.trim_start_matches(['-', '.', ' ']);
    let file_key = naming::title_key(file_title);
    (!file_key.is_empty() && naming::title_key(rest) == file_key && naming::title_key(folder_title) != file_key)
        .then(|| rest.to_string())
}

fn group_shows(files: &[FileInfo], base: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();

    for (i, f) in files.iter().enumerate() {
        // Walk up past season folders to the show folder.
        let mut show_dir = f.dir_url.clone();
        let mut season_from_dir = None;
        let mut rel = rel_segments(&show_dir, base);
        while let Some(last) = rel.last() {
            match naming::season_folder(last) {
                Some(n) => {
                    season_from_dir.get_or_insert(n);
                    show_dir = up_one(&show_dir);
                    rel.pop();
                }
                None => break,
            }
        }
        let parsed = naming::parse(&f.name);
        let season = f.season.or(parsed.season).or(season_from_dir).or(Some(1));
        let episode = f.episode.or(parsed.episode).or_else(|| naming::loose_episode(&f.name))
            .or_else(|| naming::trailing_number(&f.name));

        // Episodes straight in the source folder: group by their own title.
        let (key, show) = if rel.is_empty() {
            (
                format!("t:{}", naming::title_key(&parsed.title)),
                naming::ShowFolder {
                    title: parsed.title.clone(),
                    year: parsed.year,
                    year_end: None,
                    ongoing: false,
                },
            )
        } else {
            (format!("d:{show_dir}"), naming::parse_show_folder(rel.last().unwrap()))
        };
        let at = *index_of.entry(key.clone()).or_insert_with(|| {
            groups.push(Group {
                key,
                name: show.title,
                year: show.year,
                year_end: show.year_end,
                ongoing: show.ongoing,
                dir_url: show_dir.clone(),
                folder: if rel.len() > 1 { rel.first().cloned() } else { None },
                ..Group::default()
            });
            groups.len() - 1
        });
        groups[at].files.push(i);
        groups[at].episodes.push((i, season, episode));
    }
    groups
}

/// The poster for a title: an image in its folder (or, for movies whose
/// files sit in a subfolder, the file's folder), preferring names that say
/// what they are.
fn pick_poster(
    images: &HashMap<String, Vec<(String, String)>>,
    dir_url: &str,
    file_dir: Option<&String>,
) -> Option<String> {
    let candidates = images
        .get(dir_url)
        .or_else(|| file_dir.and_then(|d| images.get(d)))?;
    let score = |name: &str| {
        let n = name.to_lowercase();
        if n.contains("poster") || n.contains("cover") || n.starts_with("folder") {
            0
        } else if n.starts_with("a_al") {
            1
        } else if n.contains("backdrop") || n.contains("fanart") || n.contains("banner") {
            3
        } else {
            2
        }
    };
    candidates
        .iter()
        .min_by_key(|(_, name)| score(name))
        .map(|(url, _)| url.clone())
}

// ---------------------------------------------------------------- views ---

pub const TMDB_IMG: &str = "https://image.tmdb.org/t/p/";

/// A poster card in a grid or row.
#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub year: Option<u32>,
    pub year_end: Option<u32>,
    pub ongoing: bool,
    pub quality: Option<String>,
    pub audio: Option<String>,
    pub poster: Option<String>,
    pub rating: Option<f64>,
    pub file_count: i64,
    pub season_count: i64,
    pub root: String,
    pub in_watchlist: bool,
}

// Matched titles show TMDB's spelling ("L.A. Confidential", not "L A ...").
const CARD_COLUMNS: &str = "t.id, t.kind, COALESCE(t.tmdb_title, t.name), t.year, t.year_end,
    COALESCE(t.tmdb_ongoing, t.ongoing) AS ongoing, t.quality, t.audio, t.poster, t.tmdb_poster,
    t.rating, t.file_count, t.season_count, t.root,
    EXISTS(SELECT 1 FROM watchlist w WHERE w.title_id = t.id) AS in_watchlist";

fn card_from_row(r: &Row) -> rusqlite::Result<Card> {
    let id: i64 = r.get(0)?;
    let poster: Option<String> = r.get(8)?;
    let tmdb_poster: Option<String> = r.get(9)?;
    Ok(Card {
        id,
        kind: r.get(1)?,
        name: r.get(2)?,
        year: r.get(3)?,
        year_end: r.get(4)?,
        ongoing: r.get(5)?,
        quality: r.get(6)?,
        audio: r.get(7)?,
        poster: poster_url(id, poster.as_deref(), tmdb_poster.as_deref(), "w342"),
        rating: r.get(10)?,
        file_count: r.get(11)?,
        season_count: r.get(12)?,
        root: r.get(13)?,
        in_watchlist: r.get(14)?,
    })
}

/// Where the UI loads a poster from. Folder images on the media server are
/// used directly (fast on a LAN and what the server's owner chose); local
/// ones go through the app's own /poster route; TMDB is the fallback.
pub fn poster_url(id: i64, folder: Option<&str>, tmdb: Option<&str>, size: &str) -> Option<String> {
    match folder {
        Some(p) if p.starts_with("file:") => Some(format!("/poster/{id}")),
        Some(p) => Some(p.to_string()),
        None => tmdb.map(|t| format!("{TMDB_IMG}{size}{t}")),
    }
}

/// Filters and sort for the grid pages.
#[derive(Debug, Default, serde::Deserialize)]
pub struct Query {
    pub kind: Option<String>,
    pub root: Option<String>,
    pub year: Option<String>,
    pub genre: Option<String>,
    pub quality: Option<String>,
    pub platform: Option<String>,
    pub folder: Option<String>,
    pub audio: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
    pub watchlist: Option<bool>,
    pub sort: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct Page {
    pub total: i64,
    pub items: Vec<Card>,
}

fn non_empty(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty() && *s != "any")
}

/// Builds the WHERE clause shared by grid queries and facet counts. `skip`
/// leaves one filter out, so a dropdown's options aren't narrowed by its own
/// current choice.
fn filter_sql(q: &Query, skip: Option<&str>) -> (String, Vec<rusqlite::types::Value>) {
    use rusqlite::types::Value;
    let mut sql = String::from(" WHERE 1=1");
    let mut args: Vec<Value> = Vec::new();
    let text = |v: &str| Value::Text(v.to_string());
    let mut add = |clause: &str, value: Option<Value>| {
        sql.push_str(clause);
        args.extend(value);
    };
    let active = |name: &str| skip != Some(name);

    if let Some(k) = non_empty(&q.kind) {
        add(" AND t.kind = ?", Some(text(k)));
    }
    if let Some(r) = non_empty(&q.root) {
        add(" AND t.root = ?", Some(text(r)));
    }
    if let Some(y) = non_empty(&q.year).filter(|_| active("year")).and_then(|y| y.parse::<i64>().ok()) {
        add(" AND t.year = ?", Some(Value::Integer(y)));
    }
    if let Some(g) = non_empty(&q.genre).filter(|_| active("genre")) {
        add(" AND t.genres LIKE ?", Some(Value::Text(format!("%,{g},%"))));
    }
    if let Some(v) = non_empty(&q.quality).filter(|_| active("quality")) {
        add(" AND t.quality = ?", Some(text(v)));
    }
    if let Some(v) = non_empty(&q.platform).filter(|_| active("platform")) {
        add(" AND COALESCE(t.platform, t.network) = ?", Some(text(v)));
    }
    if let Some(v) = non_empty(&q.folder).filter(|_| active("folder")) {
        add(" AND t.folder = ?", Some(text(v)));
    }
    match non_empty(&q.audio).filter(|_| active("audio")) {
        Some("single") => add(" AND t.audio IS NULL", None),
        Some(v) => add(" AND t.audio = ?", Some(text(v))),
        None => {}
    }
    match non_empty(&q.status).filter(|_| active("status")) {
        Some("ongoing") => add(" AND COALESCE(t.tmdb_ongoing, t.ongoing)", None),
        Some("ended") => add(" AND NOT COALESCE(t.tmdb_ongoing, t.ongoing)", None),
        _ => {}
    }
    if q.watchlist == Some(true) {
        add(" AND t.id IN (SELECT title_id FROM watchlist)", None);
    }
    if let Some(m) = non_empty(&q.q).and_then(fts_query) {
        add(
            " AND t.id IN (SELECT rowid FROM titles_fts WHERE titles_fts MATCH ?)",
            Some(Value::Text(m)),
        );
    }
    (sql, args)
}

pub fn titles(conn: &Connection, q: &Query) -> Result<Page> {
    let (filter, args) = filter_sql(q, None);
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM titles t{filter}"),
        params_from_iter(args.iter()),
        |r| r.get(0),
    )?;
    let order = match q.sort.as_deref().unwrap_or("added") {
        "updated" => "t.updated_at DESC, t.id DESC",
        "title" => "t.name COLLATE NOCASE ASC",
        "year" => "t.year IS NULL, t.year DESC, t.name COLLATE NOCASE",
        "rating" => "t.rating IS NULL, t.rating DESC, t.name COLLATE NOCASE",
        "watchlist" => "(SELECT added_at FROM watchlist w WHERE w.title_id = t.id) DESC",
        _ => "t.added_at DESC, t.id DESC",
    };
    let limit = q.limit.unwrap_or(60).clamp(1, 500);
    let sql = format!(
        "SELECT {CARD_COLUMNS} FROM titles t{filter} ORDER BY {order} LIMIT {limit} OFFSET {}",
        q.offset.unwrap_or(0)
    );
    let mut stmt = conn.prepare(&sql)?;
    let items = stmt
        .query_map(params_from_iter(args.iter()), card_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Page { total, items })
}

#[derive(Serialize)]
pub struct Facet {
    pub value: String,
    pub label: String,
    pub count: i64,
}

#[derive(Serialize)]
pub struct Facets {
    pub years: Vec<Facet>,
    pub genres: Vec<Facet>,
    pub qualities: Vec<Facet>,
    pub platforms: Vec<Facet>,
    pub folders: Vec<Facet>,
    pub audios: Vec<Facet>,
}

/// Options for each filter dropdown, counted under the other active filters.
pub fn facets(conn: &Connection, q: &Query) -> Result<Facets> {
    let simple = |column: &str, skip: &str, order: &str| -> Result<Vec<Facet>> {
        let (filter, args) = filter_sql(q, Some(skip));
        let sql = format!(
            "SELECT {column} AS v, COUNT(*) FROM titles t{filter} AND {column} IS NOT NULL
             GROUP BY v ORDER BY {order}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter()), |r| {
            let v: rusqlite::types::Value = r.get(0)?;
            let value = match v {
                rusqlite::types::Value::Integer(i) => i.to_string(),
                rusqlite::types::Value::Text(s) => s,
                _ => String::new(),
            };
            Ok(Facet {
                label: value.clone(),
                value,
                count: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    };

    let years = simple("t.year", "year", "v DESC")?;
    let mut qualities = simple("t.quality", "quality", "COUNT(*) DESC")?;
    qualities.sort_by_key(|f| std::cmp::Reverse(quality_rank(&f.value)));
    let platforms = simple("COALESCE(t.platform, t.network)", "platform", "COUNT(*) DESC")?;
    let mut folders = simple("t.folder", "folder", "v COLLATE NOCASE")?;
    // Year folders read best newest first.
    if folders.iter().all(|f| f.value.starts_with('(')) {
        folders.reverse();
    }

    let mut audios = simple("t.audio", "audio", "COUNT(*) DESC")?;
    for a in &mut audios {
        a.label = if a.value == "multi" { "Multi audio" } else { "Dual audio" }.into();
    }

    // Genres are stored as ",A,B," lists, so split them in Rust.
    let (filter, args) = filter_sql(q, Some("genre"));
    let mut stmt = conn.prepare(&format!(
        "SELECT t.genres FROM titles t{filter} AND t.genres IS NOT NULL AND t.genres != ''"
    ))?;
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for row in stmt.query_map(params_from_iter(args.iter()), |r| r.get::<_, String>(0))? {
        for g in row?.split(',').filter(|g| !g.is_empty()) {
            *counts.entry(g.to_string()).or_default() += 1;
        }
    }
    let genres = counts
        .into_iter()
        .map(|(g, count)| Facet {
            label: g.clone(),
            value: g,
            count,
        })
        .collect();

    Ok(Facets {
        years,
        genres,
        qualities,
        platforms,
        folders,
        audios,
    })
}

fn quality_rank(q: &str) -> u8 {
    match q {
        "2160p" => 4,
        "1080p" => 3,
        "720p" => 2,
        "480p" => 1,
        _ => 0,
    }
}

// --------------------------------------------------------------- detail ---

#[derive(Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub card: Card,
    pub dir_url: String,
    pub folder: Option<String>,
    pub platform: Option<String>,
    pub tmdb_id: Option<i64>,
    pub tmdb_status: Option<String>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub genres: Vec<String>,
    pub country: Option<String>,
    pub credits: Option<String>,
    pub credits_label: &'static str,
    pub backdrop: Option<String>,
    pub runtime: Option<i64>,
    pub total_size: Option<i64>,
    pub language: Option<String>,
    pub files: Vec<EpisodeView>,
    pub seasons: Vec<SeasonView>,
    pub next_up: Option<i64>,
}

#[derive(Serialize, Clone)]
pub struct EpisodeView {
    pub file_id: i64,
    pub url: String,
    pub name: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub size: Option<i64>,
    pub quality: Option<&'static str>,
    pub watched: bool,
    pub played_at: Option<i64>,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub runtime: Option<i64>,
    pub still: Option<String>,
    /// Other files of the same episode (e.g. a dubbed and an original
    /// release). The episode row plays `file_id`; these are alternatives.
    pub versions: Vec<Version>,
}

#[derive(Serialize, Clone)]
pub struct Version {
    pub file_id: i64,
    pub url: String,
    pub name: String,
    pub size: Option<i64>,
    pub quality: Option<&'static str>,
    pub audio: Option<&'static str>,
}

#[derive(Serialize)]
pub struct SeasonView {
    pub season: u32,
    pub episodes: Vec<EpisodeView>,
    pub size: i64,
}

pub fn detail(conn: &Connection, id: i64) -> Result<Option<Detail>> {
    let card = conn
        .query_row(
            &format!("SELECT {CARD_COLUMNS} FROM titles t WHERE t.id = ?1"),
            [id],
            card_from_row,
        )
        .optional()?;
    let Some(mut card) = card else {
        return Ok(None);
    };
    #[allow(clippy::type_complexity)]
    let row: (
        String,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn.query_row(
        "SELECT dir_url, folder, platform, tmdb_id, tmdb_status, overview, tagline, genres,
                country, credits, backdrop, runtime, total_size, language, network, tmdb_title
         FROM titles WHERE id = ?1",
        [id],
        |r| {
            Ok((
                r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
                r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?, r.get(13)?,
                r.get(14)?, r.get(15)?,
            ))
        },
    )?;
    let poster_big: Option<String> = conn.query_row(
        "SELECT poster, tmdb_poster FROM titles WHERE id = ?1",
        [id],
        |r| {
            let p: Option<String> = r.get(0)?;
            let t: Option<String> = r.get(1)?;
            Ok(poster_url(id, p.as_deref(), t.as_deref(), "w500"))
        },
    )?;
    card.poster = poster_big;

    let mut stmt = conn.prepare(
        "SELECT f.id, f.url, f.name, f.season, f.episode, f.size,
                (SELECT 1 FROM watched w WHERE w.file_id = f.id) IS NOT NULL,
                (SELECT played_at FROM plays p WHERE p.file_id = f.id),
                m.name, m.overview, m.air_date, m.runtime, m.still
         FROM files f
         LEFT JOIN episode_meta m ON m.title_id = f.title_id AND m.season = f.season AND m.episode = f.episode
         WHERE f.title_id = ?1
         ORDER BY f.season IS NULL, f.season, f.episode IS NULL, f.episode, f.name",
    )?;
    let files: Vec<EpisodeView> = stmt
        .query_map([id], |r| {
            let name: String = r.get(2)?;
            let still: Option<String> = r.get(12)?;
            Ok(EpisodeView {
                file_id: r.get(0)?,
                url: r.get(1)?,
                quality: naming::quality(&name),
                name,
                season: r.get(3)?,
                episode: r.get(4)?,
                size: r.get(5)?,
                watched: r.get(6)?,
                played_at: r.get(7)?,
                title: r.get(8)?,
                overview: r.get(9)?,
                air_date: r.get(10)?,
                runtime: r.get(11)?,
                still: still.map(|s| format!("{TMDB_IMG}w300{s}")),
                versions: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let is_show = card.kind == "show";

    // One row per episode; extra files of the same episode become versions.
    let mut rows: Vec<EpisodeView> = Vec::new();
    for f in files.iter().cloned() {
        let same = rows.last().is_some_and(|last: &EpisodeView| {
            is_show && f.episode.is_some() && last.season == f.season && last.episode == f.episode
        });
        if same {
            let last = rows.last_mut().unwrap();
            if last.versions.is_empty() {
                last.versions.push(version_of(last));
            }
            last.versions.push(version_of(&f));
            last.watched |= f.watched;
            last.played_at = last.played_at.max(f.played_at);
        } else {
            rows.push(f);
        }
    }

    let mut seasons: Vec<SeasonView> = Vec::new();
    if is_show {
        for f in &rows {
            let s = f.season.unwrap_or(1);
            if seasons.last().map(|x| x.season) != Some(s) {
                seasons.push(SeasonView {
                    season: s,
                    episodes: Vec::new(),
                    size: 0,
                });
            }
            let last = seasons.last_mut().unwrap();
            last.size += f.size.unwrap_or(0);
            last.episodes.push(f.clone());
        }
    }

    // Up next: the episode after the most recently played one, or the
    // first unwatched; for a movie, its first file.
    let next_up = if is_show {
        let last_played = rows
            .iter()
            .enumerate()
            .filter_map(|(i, f)| f.played_at.map(|t| (t, i)))
            .max()
            .map(|(_, i)| i);
        match last_played {
            Some(i) if i + 1 < rows.len() => Some(rows[i + 1].file_id),
            Some(_) => rows.iter().find(|f| !f.watched).map(|f| f.file_id),
            None => rows
                .iter()
                .find(|f| !f.watched && f.episode != Some(0) && f.season != Some(0))
                .or_else(|| rows.iter().find(|f| !f.watched))
                .or(rows.first())
                .map(|f| f.file_id),
        }
    } else {
        files.first().map(|f| f.file_id)
    };

    let (dir_url, folder, platform, tmdb_id, tmdb_status, overview, tagline, genres, country, credits, backdrop, runtime, total_size, language, network, _alt) = row;
    Ok(Some(Detail {
        card,
        dir_url,
        folder,
        platform: platform.or(network),
        tmdb_id,
        tmdb_status,
        overview,
        tagline,
        genres: genres
            .unwrap_or_default()
            .split(',')
            .filter(|g| !g.is_empty())
            .map(String::from)
            .collect(),
        country,
        credits,
        credits_label: if is_show { "Created by" } else { "Directed by" },
        backdrop: backdrop.map(|b| format!("{TMDB_IMG}w1280{b}")),
        runtime,
        total_size,
        language,
        files: if is_show { Vec::new() } else { files },
        seasons,
        next_up,
    }))
}

fn version_of(e: &EpisodeView) -> Version {
    Version {
        file_id: e.file_id,
        url: e.url.clone(),
        name: e.name.clone(),
        size: e.size,
        quality: e.quality,
        audio: naming::audio(&e.name).or_else(|| {
            let l = e.name.to_lowercase();
            (l.contains("hindi dubbed") || l.contains("dubbed")).then_some("dubbed")
        }),
    }
}

// ----------------------------------------------------------------- home ---

#[derive(Serialize)]
pub struct HomeRow {
    pub title: String,
    pub link: Option<String>,
    pub items: Vec<Card>,
}

fn cards(conn: &Connection, where_order: &str, limit: usize) -> Result<Vec<Card>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CARD_COLUMNS} FROM titles t {where_order} LIMIT {limit}"
    ))?;
    let items = stmt.query_map([], card_from_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(items)
}

pub fn home(conn: &Connection) -> Result<Vec<HomeRow>> {
    let mut rows = Vec::new();
    let mut push = |title: &str, link: Option<&str>, items: Vec<Card>| {
        if !items.is_empty() {
            rows.push(HomeRow {
                title: title.into(),
                link: link.map(String::from),
                items,
            });
        }
    };
    push(
        "Continue watching",
        Some("#/recent"),
        cards(
            conn,
            "JOIN (SELECT title_id, MAX(played_at) AS last FROM plays GROUP BY title_id) p
             ON p.title_id = t.id ORDER BY p.last DESC",
            20,
        )?,
    );
    push(
        "Recently added movies",
        Some("#/movies"),
        cards(conn, "WHERE t.kind = 'movie' ORDER BY t.added_at DESC, t.id DESC", 20)?,
    );
    push(
        "Recently updated shows",
        Some("#/shows"),
        cards(conn, "WHERE t.kind = 'show' ORDER BY t.updated_at DESC, t.id DESC", 20)?,
    );
    push(
        "From your watchlist",
        Some("#/watchlist"),
        cards(
            conn,
            "JOIN watchlist w ON w.title_id = t.id ORDER BY w.added_at DESC",
            20,
        )?,
    );
    push(
        "Top rated movies",
        None,
        cards(
            conn,
            "WHERE t.kind = 'movie' AND t.rating IS NOT NULL ORDER BY t.rating DESC",
            20,
        )?,
    );
    push(
        "Top rated shows",
        None,
        cards(
            conn,
            "WHERE t.kind = 'show' AND t.rating IS NOT NULL ORDER BY t.rating DESC",
            20,
        )?,
    );
    Ok(rows)
}

/// Titles by most recent play.
pub fn recently_played(conn: &Connection, limit: usize) -> Result<Vec<Card>> {
    cards(
        conn,
        "JOIN (SELECT title_id, MAX(played_at) AS last FROM plays GROUP BY title_id) p
         ON p.title_id = t.id ORDER BY p.last DESC",
        limit,
    )
}

pub fn cards_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<Card>> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare(&format!("SELECT {CARD_COLUMNS} FROM titles t WHERE t.id = ?1"))?;
    for id in ids {
        if let Some(c) = stmt.query_row([id], card_from_row).optional()? {
            out.push(c);
        }
    }
    Ok(out)
}

#[derive(Serialize, Default)]
pub struct Counts {
    pub movies: i64,
    pub shows: i64,
    pub watchlist: i64,
    pub recent: i64,
    pub files: i64,
    pub bytes: i64,
    pub per_root: HashMap<String, i64>,
}

pub fn counts(conn: &Connection) -> Result<Counts> {
    let one = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
    let mut per_root = HashMap::new();
    let mut stmt = conn.prepare("SELECT root, COUNT(*) FROM titles GROUP BY root")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (root, n) = row?;
        per_root.insert(root, n);
    }
    Ok(Counts {
        movies: one("SELECT COUNT(*) FROM titles WHERE kind = 'movie'")?,
        shows: one("SELECT COUNT(*) FROM titles WHERE kind = 'show'")?,
        watchlist: one("SELECT COUNT(*) FROM watchlist")?,
        recent: one("SELECT COUNT(DISTINCT title_id) FROM plays")?,
        files: one("SELECT COUNT(*) FROM files")?,
        bytes: one("SELECT COALESCE(SUM(size), 0) FROM files")?,
        per_root,
    })
}

// -------------------------------------------------------------- actions ---

pub fn set_watchlist(conn: &Connection, title_id: i64, on: bool) -> Result<()> {
    if on {
        conn.execute(
            "INSERT OR IGNORE INTO watchlist (title_id, added_at) VALUES (?1, ?2)",
            params![title_id, unix_now()],
        )?;
    } else {
        conn.execute("DELETE FROM watchlist WHERE title_id = ?1", [title_id])?;
    }
    Ok(())
}

pub fn set_watched(conn: &Connection, file_id: i64, on: bool) -> Result<()> {
    if on {
        conn.execute(
            "INSERT OR REPLACE INTO watched (file_id, at) VALUES (?1, ?2)",
            params![file_id, unix_now()],
        )?;
    } else {
        conn.execute("DELETE FROM watched WHERE file_id = ?1", [file_id])?;
    }
    Ok(())
}

/// Records a play: it shows in "Recently played", moves "Up next" along and
/// ticks the episode as watched.
pub fn record_play(conn: &Connection, file_id: i64) -> Result<()> {
    let now = unix_now();
    conn.execute(
        "INSERT INTO plays (file_id, title_id, played_at)
         SELECT id, title_id, ?2 FROM files WHERE id = ?1 AND title_id IS NOT NULL
         ON CONFLICT(file_id) DO UPDATE SET played_at = excluded.played_at",
        params![file_id, now],
    )?;
    set_watched(conn, file_id, true)
}

/// (url, name, title_id) of a file.
pub fn file_info(conn: &Connection, file_id: i64) -> Result<Option<(String, String, Option<i64>)>> {
    Ok(conn
        .query_row(
            "SELECT url, name, title_id FROM files WHERE id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?)
}

/// Every file of a title in play order, for playlists.
/// Every episode of a title in play order (first version of each), for
/// playlists.
pub fn title_files(conn: &Connection, title_id: i64) -> Result<Vec<(i64, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, url, name, season, episode FROM files WHERE title_id = ?1
         ORDER BY season IS NULL, season, episode IS NULL, episode, name",
    )?;
    let rows = stmt.query_map([title_id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Option<u32>>(3)?, r.get::<_, Option<u32>>(4)?))
    })?;
    let mut out: Vec<(i64, String, String)> = Vec::new();
    let mut last: Option<(Option<u32>, u32)> = None;
    for row in rows {
        let (id, url, name, season, episode) = row?;
        if let Some(e) = episode {
            if last == Some((season, e)) {
                continue;
            }
            last = Some((season, e));
        }
        out.push((id, url, name));
    }
    Ok(out)
}

pub fn title_dir(conn: &Connection, title_id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT dir_url FROM titles WHERE id = ?1", [title_id], |r| r.get(0))
        .optional()?)
}

pub fn local_poster(conn: &Connection, title_id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT poster FROM titles WHERE id = ?1", [title_id], |r| r.get(0))
        .optional()?
        .flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{FileRow, ImageRow};

    fn file(root: &str, url: &str) -> FileRow {
        let name = percent_decode(url.rsplit('/').next().unwrap());
        let p = naming::parse(&name);
        FileRow {
            root: root.into(),
            url: url.into(),
            dir: percent_decode(Url::parse(url).unwrap().path()),
            size: Some(100),
            title: p.title,
            season: p.season,
            episode: p.episode,
            year: p.year,
            name,
        }
    }

    #[test]
    fn rank_prefixes() {
        assert_eq!(strip_rank("130 Batman Begins", "Batman Begins").as_deref(), Some("Batman Begins"));
        assert_eq!(strip_rank("1917", "1917"), None);
        assert_eq!(strip_rank("300 Rise of an Empire", "300 Rise of an Empire"), None);
        assert_eq!(strip_rank("10 Lives", "10 Lives"), None);
        assert_eq!(strip_list_number("002. The Godfather (1972) 1080p"), "The Godfather (1972) 1080p");
        assert_eq!(strip_list_number("1917 (2019)"), "1917 (2019)");
        assert_eq!(strip_list_number("2001. A Space Odyssey"), "2001. A Space Odyssey");
        assert!(is_part_folder("(1972) Pratidwandi-CD2"));
        assert!(is_part_folder("Disc 1"));
        assert!(!is_part_folder("Abcd"));
    }

    #[test]
    fn groups_dhakaflix_layouts() {
        let mut idx = Index::open(":memory:").unwrap();
        let m = "http://h/DF/Animation%20Movies/";
        let movies = Root {
            name: "Anim".into(),
            url: Some(m.into()),
            path: None,
            kind: SourceKind::Movies,
        };
        let rows = [
            file("Anim", &format!("{m}%282024%29/Appu%20%282024%29%20720p%20BMS%20%5BDual%20Audio%5D/Appu.2024.720p.mkv")),
            file("Anim", &format!("{m}%282024%29/10%20Lives%20%282024%29%20720p%20WEBRip/10.Lives.2024.720p.WEBRip.mkv")),
            file("Anim", &format!("{m}%282024%29/10%20Lives%20%282024%29%20720p%20WEBRip/10.Lives.2024.1080p.WEBRip.mkv")),
        ];
        let images = [ImageRow {
            root: "Anim".into(),
            dir: format!("{m}%282024%29/Appu%20%282024%29%20720p%20BMS%20%5BDual%20Audio%5D/"),
            url: format!("{m}%282024%29/Appu%20%282024%29%20720p%20BMS%20%5BDual%20Audio%5D/a_AL_.jpg"),
            name: "a_AL_.jpg".into(),
        }];
        idx.record_level("Anim", &rows, &images, &[], &[]).unwrap();
        assert_eq!(rebuild_titles(&mut idx, &movies).unwrap(), 2);

        let page = titles(idx.conn(), &Query { kind: Some("movie".into()), sort: Some("title".into()), ..Query::default() }).unwrap();
        assert_eq!(page.total, 2);
        let names: Vec<_> = page.items.iter().map(|c| (c.name.as_str(), c.year, c.audio.as_deref(), c.quality.as_deref())).collect();
        assert_eq!(names, vec![("10 Lives", Some(2024), None, Some("1080p")), ("Appu", Some(2024), Some("dual"), Some("720p"))]);
        assert!(page.items[1].poster.as_deref().unwrap().ends_with("a_AL_.jpg"));
        let f = facets(idx.conn(), &Query::default()).unwrap();
        assert_eq!(f.folders[0].value, "(2024)");

        let s = "http://h/DF/KOREAN%20TV/";
        let shows = Root {
            name: "Korean".into(),
            url: Some(s.into()),
            path: None,
            kind: SourceKind::Shows,
        };
        let show = format!("{s}A%20Bloody%20Lucky%20Day%20%28TV%20Series%202023%E2%80%93%20%29%20720p%20%5BDual%20Audio%5D/");
        let rows = [
            file("Korean", &format!("{show}Season%201/A%20Bloody%20Lucky%20Day%20S01E02%20720p.mkv")),
            file("Korean", &format!("{show}Season%201/A%20Bloody%20Lucky%20Day%20S01E01%20720p.mkv")),
            file("Korean", &format!("{show}Season%202/Episode%2001.mkv")),
        ];
        idx.record_level("Korean", &rows, &[], &[], &[]).unwrap();
        assert_eq!(rebuild_titles(&mut idx, &shows).unwrap(), 1);
        let page = titles(idx.conn(), &Query { kind: Some("show".into()), ..Query::default() }).unwrap();
        let c = &page.items[0];
        assert_eq!((c.name.as_str(), c.year, c.ongoing, c.file_count, c.season_count), ("A Bloody Lucky Day", Some(2023), true, 3, 2));
        let d = detail(idx.conn(), c.id).unwrap().unwrap();
        assert_eq!(d.seasons.len(), 2);
        assert_eq!(d.seasons[0].episodes[0].episode, Some(1));
        assert_eq!(d.seasons[1].episodes[0].episode, Some(1));

        // Play S01E01 -> up next is S01E02; the title shows in recents.
        let first = d.seasons[0].episodes[0].file_id;
        record_play(idx.conn(), first).unwrap();
        let d = detail(idx.conn(), c.id).unwrap().unwrap();
        assert_eq!(d.next_up, Some(d.seasons[0].episodes[1].file_id));
        assert!(d.seasons[0].episodes[0].watched);
        assert_eq!(recently_played(idx.conn(), 10).unwrap().len(), 1);

        // Search by title, and stable ids across a rebuild.
        let found = titles(idx.conn(), &Query { q: Some("bloody".into()), ..Query::default() }).unwrap();
        assert_eq!(found.total, 1);
        set_watchlist(idx.conn(), c.id, true).unwrap();
        rebuild_titles(&mut idx, &shows).unwrap();
        let again = titles(idx.conn(), &Query { watchlist: Some(true), ..Query::default() }).unwrap();
        assert_eq!(again.items[0].id, c.id);
    }
}

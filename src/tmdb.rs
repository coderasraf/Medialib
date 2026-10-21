//! The Movie Database (TMDB): posters, backdrops, genres, ratings, credits
//! and episode details for the titles in the library.
//!
//! Lookups follow the crawler's pattern: fetch a batch concurrently, then
//! write it in one transaction, so the SQLite connection never sits inside
//! a future.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures::stream::{self, StreamExt};
use reqwest::{Client, RequestBuilder, StatusCode};
use rusqlite::{params, Connection};
use serde_json::Value;

use crate::config;
use crate::index::Index;
use crate::naming;

const API: &str = "https://api.themoviedb.org/3";

#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub tmdb_id: i64,
    pub title: String,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub rating: Option<f64>,
    pub genres: Vec<String>,
    pub country: Option<String>,
    pub credits: Option<String>,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub runtime: Option<i64>,
    pub language: Option<String>,
    pub network: Option<String>,
    pub ongoing: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct EpisodeMeta {
    pub episode: i64,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub runtime: Option<i64>,
    pub still: Option<String>,
}

#[derive(Clone)]
pub struct Tmdb {
    client: Client,
    key: String,
    language: String,
}

impl Tmdb {
    pub fn new(cfg: &config::Tmdb) -> Result<Tmdb> {
        if cfg.api_key.is_empty() {
            bail!("add a TMDB API key in Settings > Movie info");
        }
        Ok(Tmdb {
            client: Client::builder()
                .timeout(Duration::from_secs(20))
                .user_agent(concat!("medialib/", env!("CARGO_PKG_VERSION")))
                .build()?,
            key: cfg.api_key.clone(),
            language: cfg.language.clone(),
        })
    }

    /// TMDB issues two kinds of credentials: a 32-character v3 "API key"
    /// passed as a query parameter, and a long v4 "read access token" (a
    /// JWT) sent as a Bearer header. Accept whichever the user pasted.
    fn get(&self, path: &str) -> RequestBuilder {
        let req = self
            .client
            .get(format!("{API}{path}"))
            .query(&[("language", self.language.as_str())]);
        if self.key.starts_with("eyJ") {
            req.bearer_auth(&self.key)
        } else {
            req.query(&[("api_key", self.key.as_str())])
        }
    }

    /// Sends a request, waiting and retrying when rate-limited.
    async fn json(&self, req: RequestBuilder) -> Result<Value> {
        for attempt in 0..4 {
            let resp = req
                .try_clone()
                .expect("GET requests have no streaming body")
                .send()
                .await?;
            match resp.status() {
                StatusCode::TOO_MANY_REQUESTS => {
                    let wait = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1u64 + attempt);
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                }
                StatusCode::UNAUTHORIZED => bail!("TMDB rejected the API key"),
                s if !s.is_success() => bail!("TMDB returned {s}"),
                _ => return Ok(resp.json().await?),
            }
        }
        bail!("TMDB kept rate-limiting; try again later")
    }

    /// Cheap call that validates the key.
    pub async fn check(&self) -> Result<()> {
        self.json(self.get("/configuration")).await.map(|_| ())
    }

    /// Finds the best match for a title and fetches its details.
    pub async fn lookup(&self, kind: &str, name: &str, year: Option<u32>) -> Result<Option<Meta>> {
        let Some(id) = self.search(kind, name, year).await? else {
            return Ok(None);
        };
        Ok(Some(self.details(kind, id).await?))
    }

    pub async fn search(&self, kind: &str, name: &str, year: Option<u32>) -> Result<Option<i64>> {
        let (path, year_param) = if kind == "show" {
            ("/search/tv", "first_air_date_year")
        } else {
            ("/search/movie", "year")
        };
        let mut years = vec![year];
        if year.is_some() {
            // Release years in folder names are often off by one, or the
            // year of a later season; retry without it.
            years.push(None);
        }
        // "Das Boot (The Boat)": also try without the bracketed alt title.
        let mut names = vec![name.to_string()];
        let bare = strip_brackets(name);
        if !bare.is_empty() && bare != name {
            names.push(bare);
        }
        for n in &names {
            for y in &years {
                let mut req = self.get(path).query(&[("query", n.as_str()), ("include_adult", "false")]);
                if let Some(y) = y {
                    req = req.query(&[(year_param, y.to_string())]);
                }
                let v = self.json(req).await?;
                let results = v["results"].as_array().cloned().unwrap_or_default();
                if let Some(best) = best_result(&results, kind, n, year) {
                    return Ok(Some(best));
                }
            }
        }
        Ok(None)
    }

    pub async fn details(&self, kind: &str, id: i64) -> Result<Meta> {
        let v = if kind == "show" {
            self.json(self.get(&format!("/tv/{id}"))).await?
        } else {
            self.json(self.get(&format!("/movie/{id}")).query(&[("append_to_response", "credits")]))
                .await?
        };
        let text = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(String::from);
        let names = |k: &str| -> Vec<String> {
            v[k].as_array()
                .map(|a| a.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect())
                .unwrap_or_default()
        };
        let mut meta = Meta {
            tmdb_id: id,
            title: text("title").or_else(|| text("name")).unwrap_or_default(),
            overview: text("overview"),
            tagline: text("tagline"),
            rating: v["vote_average"].as_f64().filter(|r| *r > 0.0),
            genres: names("genres"),
            country: names("production_countries").into_iter().next(),
            poster: text("poster_path"),
            backdrop: text("backdrop_path"),
            language: text("original_language"),
            ..Meta::default()
        };
        if kind == "show" {
            let creators = names("created_by");
            meta.credits = (!creators.is_empty()).then(|| creators.join(", "));
            meta.network = names("networks").into_iter().next();
            meta.runtime = v["episode_run_time"][0].as_i64();
            meta.ongoing = text("status").map(|s| {
                matches!(s.as_str(), "Returning Series" | "In Production" | "Planned" | "Pilot")
            });
            if meta.country.is_none() {
                meta.country = v["origin_country"][0].as_str().map(String::from);
            }
        } else {
            meta.runtime = v["runtime"].as_i64().filter(|r| *r > 0);
            let directors: Vec<String> = v["credits"]["crew"]
                .as_array()
                .map(|crew| {
                    crew.iter()
                        .filter(|c| c["job"] == "Director")
                        .filter_map(|c| c["name"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            meta.credits = (!directors.is_empty()).then(|| directors.join(", "));
        }
        Ok(meta)
    }

    pub async fn season(&self, tv_id: i64, season: u32) -> Result<Vec<EpisodeMeta>> {
        let v = self
            .json(self.get(&format!("/tv/{tv_id}/season/{season}")))
            .await?;
        let episodes = v["episodes"].as_array().cloned().unwrap_or_default();
        Ok(episodes
            .iter()
            .filter_map(|e| {
                let text = |k: &str| e[k].as_str().filter(|s| !s.is_empty()).map(String::from);
                Some(EpisodeMeta {
                    episode: e["episode_number"].as_i64()?,
                    name: text("name"),
                    overview: text("overview"),
                    air_date: text("air_date"),
                    runtime: e["runtime"].as_i64(),
                    still: text("still_path"),
                })
            })
            .collect())
    }
}

/// Removes "(...)" and "[...]" groups.
fn strip_brackets(name: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1i32).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Picks the search result whose title (or original title) matches the
/// name exactly, preferring the right year; otherwise TMDB's top result.
fn best_result(results: &[Value], kind: &str, name: &str, year: Option<u32>) -> Option<i64> {
    let want = naming::title_key(name);
    let (title_k, orig_k, date_k) = if kind == "show" {
        ("name", "original_name", "first_air_date")
    } else {
        ("title", "original_title", "release_date")
    };
    let score = |r: &Value| {
        let titles = [r[title_k].as_str(), r[orig_k].as_str()];
        let exact = titles.iter().flatten().any(|t| naming::title_key(t) == want);
        let ry: Option<u32> = r[date_k].as_str().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok());
        let year_ok = match (year, ry) {
            (Some(a), Some(b)) => a.abs_diff(b) <= 1,
            _ => false,
        };
        (exact as u8) * 2 + year_ok as u8
    };
    results
        .iter()
        .take(10)
        .enumerate()
        .max_by_key(|(i, r)| (score(r), std::cmp::Reverse(*i)))
        .and_then(|(_, r)| r["id"].as_i64())
}

/// Writes a lookup result. `None` records "no match" so it isn't retried
/// on every sync.
pub fn store(conn: &Connection, title_id: i64, meta: Option<&Meta>) -> Result<()> {
    match meta {
        Some(m) => {
            let genres = if m.genres.is_empty() {
                None
            } else {
                Some(format!(",{},", m.genres.join(",")))
            };
            conn.execute(
                "UPDATE titles SET tmdb_status = 'ok', tmdb_id = ?2, tmdb_title = ?3, overview = ?4,
                     tagline = ?5, rating = ?6, genres = ?7, country = ?8, credits = ?9,
                     tmdb_poster = ?10, backdrop = ?11, runtime = ?12, language = ?13,
                     network = ?14, tmdb_ongoing = ?15
                 WHERE id = ?1",
                params![
                    title_id, m.tmdb_id, m.title, m.overview, m.tagline, m.rating, genres,
                    m.country, m.credits, m.poster, m.backdrop, m.runtime, m.language,
                    m.network, m.ongoing
                ],
            )?;
            conn.execute(
                "UPDATE titles_fts SET alt = ?2 WHERE rowid = ?1",
                params![title_id, m.title],
            )?;
        }
        None => {
            conn.execute(
                "UPDATE titles SET tmdb_status = 'none' WHERE id = ?1",
                [title_id],
            )?;
        }
    }
    Ok(())
}

pub fn store_season(conn: &mut Connection, title_id: i64, season: u32, eps: &[EpisodeMeta]) -> Result<()> {
    let tx = conn.transaction()?;
    for e in eps {
        tx.execute(
            "INSERT OR REPLACE INTO episode_meta
                 (title_id, season, episode, name, overview, air_date, runtime, still)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![title_id, season, e.episode, e.name, e.overview, e.air_date, e.runtime, e.still],
        )?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO season_fetched (title_id, season) VALUES (?1, ?2)",
        params![title_id, season],
    )?;
    tx.commit()?;
    Ok(())
}

/// Seasons of a show that have files but no fetched episode info yet.
pub fn missing_seasons(conn: &Connection, title_id: i64) -> Result<Vec<u32>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT season FROM files WHERE title_id = ?1 AND season IS NOT NULL
         AND season NOT IN (SELECT season FROM season_fetched WHERE title_id = ?1)",
    )?;
    let rows = stmt.query_map([title_id], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Titles still waiting for a lookup: (id, kind, name, year).
pub fn pending(conn: &Connection, retry_misses: bool) -> Result<Vec<(i64, String, String, Option<u32>)>> {
    let sql = if retry_misses {
        "SELECT id, kind, name, year FROM titles WHERE tmdb_status IS NULL OR tmdb_status != 'ok' ORDER BY added_at DESC"
    } else {
        "SELECT id, kind, name, year FROM titles WHERE tmdb_status IS NULL ORDER BY added_at DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// How a lookup run reports progress and learns it should stop.
pub trait LookupProgress {
    fn progress(&self, done: usize, total: usize);
    fn cancelled(&self) -> bool;
    fn log(&self, msg: &str);
}

const BATCH: usize = 24;

/// Looks up every pending title. Returns (matched, not found, failed).
pub fn lookup_all(
    cfg: &config::Tmdb,
    index: &mut Index,
    retry_misses: bool,
    progress: &dyn LookupProgress,
) -> Result<(usize, usize, usize)> {
    let tmdb = Tmdb::new(cfg)?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(tmdb.check()).context("checking the TMDB key")?;

    let todo = pending(index.conn(), retry_misses)?;
    let total = todo.len();
    let (mut ok, mut none, mut failed) = (0, 0, 0);
    progress.progress(0, total);

    for (n, chunk) in todo.chunks(BATCH).enumerate() {
        if progress.cancelled() {
            break;
        }
        let results: Vec<(i64, Result<Option<Meta>>)> = rt.block_on(
            stream::iter(chunk.iter().cloned())
                .map(|(id, kind, name, year)| {
                    let tmdb = tmdb.clone();
                    async move { (id, tmdb.lookup(&kind, &name, year).await) }
                })
                .buffer_unordered(6)
                .collect(),
        );
        let tx = index.conn_mut().transaction()?;
        for (id, r) in &results {
            match r {
                Ok(Some(m)) => {
                    store(&tx, *id, Some(m))?;
                    ok += 1;
                }
                Ok(None) => {
                    store(&tx, *id, None)?;
                    none += 1;
                }
                Err(e) => {
                    failed += 1;
                    if failed <= 3 {
                        progress.log(&format!("lookup failed: {e:#}"));
                    }
                    if e.to_string().contains("API key") {
                        tx.commit()?;
                        return Err(anyhow!("{e:#}"));
                    }
                }
            }
        }
        tx.commit()?;
        progress.progress(((n + 1) * BATCH).min(total), total);
    }
    Ok((ok, none, failed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prefers_exact_title_and_year() {
        let results = vec![
            json!({"id": 1, "title": "Arrival of the Dead", "release_date": "2016-01-01"}),
            json!({"id": 2, "title": "Arrival", "release_date": "1996-05-31"}),
            json!({"id": 3, "title": "Arrival", "release_date": "2016-11-10"}),
        ];
        assert_eq!(best_result(&results, "movie", "Arrival", Some(2016)), Some(3));
        assert_eq!(best_result(&results, "movie", "Arrival", None), Some(2));
        assert_eq!(best_result(&results, "movie", "Something Else", None), Some(1));
        assert_eq!(best_result(&[], "movie", "x", None), None);
    }
}

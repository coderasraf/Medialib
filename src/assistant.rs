//! The watch assistant: Google Gemini, constrained to the library.
//!
//! A language model happily recommends films that don't exist on the
//! user's server. So the model never gets the last word on *what* to show:
//!
//! 1. "Intent" pass: the model turns the request into concrete title
//!    guesses and filters (genres, language, years, movie/show).
//! 2. The app gathers candidates that really are in the library: titles
//!    matching the guesses, plus filter matches ranked by rating.
//! 3. "Choose" pass: the model picks from those candidates by id and
//!    writes the reply. Ids not in the candidate list are dropped.

use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, params_from_iter, Connection};
use serde::Serialize;
use serde_json::{json, Value};

use crate::config;
use crate::index::{fts_query, unix_now};
use crate::library::{self, Card};
use crate::naming;

const API: &str = "https://generativelanguage.googleapis.com/v1beta";

fn client() -> Result<reqwest::Client> {
    // 25 s: plenty for a normal answer, short enough that an overloaded
    // model falls through to the backup instead of hanging the chat.
    Ok(reqwest::Client::builder().timeout(Duration::from_secs(25)).build()?)
}

/// Models that can generate text, for the settings dropdown.
pub async fn list_models(key: &str) -> Result<Vec<String>> {
    if key.is_empty() {
        bail!("add a Gemini API key first");
    }
    let resp = client()?
        .get(format!("{API}/models"))
        .header("x-goog-api-key", key)
        .query(&[("pageSize", "200")])
        .send()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        bail!("{}", api_error(&v, status));
    }
    let mut names: Vec<String> = v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| {
            m["supportedGenerationMethods"]
                .as_array()
                .is_some_and(|ms| ms.iter().any(|x| x == "generateContent"))
        })
        .filter_map(|m| m["name"].as_str())
        .map(|n| n.trim_start_matches("models/").to_string())
        .filter(|n| n.starts_with("gemini") && is_chat_model(n))
        .collect();
    names.sort_by(|a, b| b.cmp(a));
    Ok(names)
}

/// Whether a model can write text replies. Google lists speech (`-tts`),
/// image, audio and live models under `generateContent` too, but they only
/// answer in other formats, so they can't run the assistant.
pub fn is_chat_model(name: &str) -> bool {
    const NOT_CHAT: &[&str] = &[
        "tts", "image", "audio", "live", "embedding", "aqa", "robotics", "computer-use", "veo",
        "imagen", "lyria", "banana", "transcribe",
    ];
    let n = name.to_ascii_lowercase();
    !NOT_CHAT.iter().any(|w| n.contains(w))
}

fn api_error(v: &Value, status: reqwest::StatusCode) -> String {
    let msg = v["error"]["message"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("Gemini returned {status}"));
    if msg.contains("response modalities") {
        return "This model can't write text replies (it's a speech or image model). \
                Choose a chat model such as gemini-flash-latest in Settings → AI assistant."
            .into();
    }
    msg
}

/// One JSON-mode call. `turns` are (role, text) with role "user"/"model".
/// Backup models tried, in order, when the chosen one is overloaded. The
/// `-latest` alias follows Google's current Flash-Lite, so it doesn't go
/// stale the way a pinned version name does.
const FALLBACK_MODELS: &[&str] = &["gemini-flash-lite-latest", "gemini-3.5-flash-lite"];

/// One JSON-mode call. `turns` are (role, text) with role "user"/"model".
///
/// Popular Gemini models are often briefly overloaded ("high demand", HTTP
/// 503/429) or slow to answer. Rather than surface that, each model gets a
/// second try after a short pause, then the call moves on to a lighter
/// fallback model. Real errors (bad key, blocked prompt) return at once.
async fn generate(cfg: &config::Assistant, system: &str, turns: &[(String, String)]) -> Result<Value> {
    let mut models = vec![cfg.model.as_str()];
    models.extend(FALLBACK_MODELS.iter().filter(|m| **m != cfg.model));
    let mut last = anyhow!("Gemini didn't answer");
    for model in models {
        for attempt in 0..2 {
            match generate_once(cfg, model, system, turns).await {
                Ok(v) => return Ok(v),
                Err(Busy(e)) => {
                    // A quick "high demand" reply is worth one more try; a
                    // timeout already cost the full wait, so move on.
                    let timed_out = e.chain().any(|c| c.to_string().contains("timed out"));
                    last = e;
                    if attempt == 0 && !timed_out {
                        tokio::time::sleep(Duration::from_millis(1500)).await;
                    } else {
                        break;
                    }
                }
                Err(Fatal(e)) => return Err(e),
            }
        }
    }
    Err(last.context("Gemini is overloaded right now; try again in a minute"))
}

enum GenError {
    /// Overloaded, rate-limited or timed out: worth retrying elsewhere.
    Busy(anyhow::Error),
    Fatal(anyhow::Error),
}
use GenError::{Busy, Fatal};

async fn generate_once(
    cfg: &config::Assistant,
    model: &str,
    system: &str,
    turns: &[(String, String)],
) -> std::result::Result<Value, GenError> {
    let contents: Vec<Value> = turns
        .iter()
        .map(|(role, text)| json!({ "role": role, "parts": [{ "text": text }] }))
        .collect();
    let body = json!({
        "systemInstruction": { "parts": [{ "text": system }] },
        "contents": contents,
        "generationConfig": { "responseMimeType": "application/json", "temperature": 0.7 },
    });
    let resp = client()
        .map_err(Fatal)?
        .post(format!("{API}/models/{model}:generateContent"))
        .header("x-goog-api-key", &cfg.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            let busy = e.is_timeout() || e.is_connect();
            let e = anyhow!(e).context("contacting Gemini");
            if busy { Busy(e) } else { Fatal(e) }
        })?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let e = anyhow!("{}", api_error(&v, status));
        let overloaded = matches!(status.as_u16(), 429 | 500 | 503 | 504)
            || v["error"]["status"] == "UNAVAILABLE"
            || v["error"]["message"].as_str().is_some_and(|m| m.contains("high demand") || m.contains("overloaded"))
            // A retired model ("no longer available") can still be
            // replaced by a fallback.
            || v["error"]["message"].as_str().is_some_and(|m| m.contains("no longer available"));
        return Err(if overloaded { Busy(e) } else { Fatal(e) });
    }
    let text: String = v["candidates"][0]["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["text"].as_str())
        .collect();
    if text.is_empty() {
        return Err(Fatal(anyhow!("Gemini returned no answer (it may have been blocked)")));
    }
    serde_json::from_str(strip_fences(&text))
        .context("Gemini's answer wasn't valid JSON")
        .map_err(Busy)
}

/// Models sometimes wrap JSON in ``` fences even in JSON mode.
fn strip_fences(s: &str) -> &str {
    let s = s.trim();
    let s = s.strip_prefix("```json").or_else(|| s.strip_prefix("```")).unwrap_or(s);
    s.strip_suffix("```").unwrap_or(s).trim()
}

/// What the UI shows for one message.
#[derive(Serialize, Clone)]
pub struct Message {
    pub id: i64,
    pub role: String,
    pub text: String,
    pub picks: Vec<Card>,
    pub at: i64,
}

pub fn history(conn: &Connection, limit: usize) -> Result<Vec<Message>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, text, picks, at FROM (SELECT * FROM chat ORDER BY id DESC LIMIT ?1) ORDER BY id",
    )?;
    let rows: Vec<(i64, String, String, Option<String>, i64)> = stmt
        .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.into_iter()
        .map(|(id, role, text, picks, at)| {
            let ids: Vec<i64> = picks
                .as_deref()
                .and_then(|p| serde_json::from_str(p).ok())
                .unwrap_or_default();
            Ok(Message {
                id,
                role,
                text,
                picks: library::cards_by_ids(conn, &ids)?,
                at,
            })
        })
        .collect()
}

pub fn clear(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM chat", [])?;
    Ok(())
}

fn save(conn: &Connection, role: &str, text: &str, picks: &[i64]) -> Result<()> {
    conn.execute(
        "INSERT INTO chat (role, text, picks, at) VALUES (?1, ?2, ?3, ?4)",
        params![role, text, serde_json::to_string(picks)?, unix_now()],
    )?;
    Ok(())
}

/// Facts about the library for the intent pass.
pub fn library_summary(conn: &Connection) -> Result<String> {
    let c = library::counts(conn)?;
    let mut stmt = conn.prepare(
        "SELECT language, COUNT(*) FROM titles WHERE language IS NOT NULL
         GROUP BY language ORDER BY COUNT(*) DESC LIMIT 12",
    )?;
    let langs: Vec<String> = stmt
        .query_map([], |r| Ok(format!("{}:{}", r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let facets = library::facets(conn, &library::Query::default())?;
    let genres: Vec<String> = facets.genres.iter().map(|g| g.value.clone()).collect();
    Ok(format!(
        "{} movies and {} TV shows. Genres: {}. Original languages (ISO 639-1:count): {}.",
        c.movies,
        c.shows,
        if genres.is_empty() { "unknown".into() } else { genres.join(", ") },
        if langs.is_empty() { "unknown".into() } else { langs.join(", ") },
    ))
}

const INTENT_PROMPT: &str = r#"You help someone choose what to watch from their own media library.
Turn their latest request into a search plan. Reply with JSON only:
{
  "titles": [{"name": "exact well-known title", "year": 2010, "kind": "movie" or "show"}],
  "genres": ["TMDB genre names, e.g. Action, Comedy, Drama, Romance, Thriller, Horror, Animation, Crime, Science Fiction, Family, Mystery, Adventure, Fantasy, Documentary, Action & Adventure, Sci-Fi & Fantasy"],
  "language": "ISO 639-1 code of the original language, or null",
  "kind": "movie" | "show" | null,
  "year_from": number or null,
  "year_to": number or null,
  "keywords": ["words likely to appear in matching titles"]
}
Put 10 to 25 specific, well-known titles that fit the request in "titles" (for "something like X", list titles similar to X, plus X itself).
"Hindi dubbed" means Indian or foreign films; language can stay null. Korean means "ko", Hindi "hi", Tamil "ta", Telugu "te", Malayalam "ml", Bengali "bn", Japanese "ja", English "en".
"Last few years" means year_from = current year minus 3."#;

const CHOOSE_PROMPT: &str = r#"You are a friendly watch assistant for someone's personal media library.
Recommend titles ONLY from the candidate list provided (each has an id). Never mention titles that are not in the list.
Reply with JSON only: {"reply": "short, warm answer (2-4 sentences) in the same language the user wrote in", "picks": [candidate ids, best first, at most 8]}
If no candidate fits, say so honestly in "reply", suggest how to rephrase, and return an empty "picks"."#;

#[derive(Serialize)]
pub struct Answer {
    pub reply: String,
    pub picks: Vec<Card>,
}

/// Handles one user message end to end. Database access happens between
/// the two model calls; the caller passes a function that runs a closure
/// against the connection, so no lock is held across an `.await`.
pub async fn ask<F>(cfg: &config::Assistant, message: &str, with_db: F) -> Result<Answer>
where
    F: Fn(&mut dyn FnMut(&Connection) -> Result<()>) -> Result<()>,
{
    if cfg.api_key.is_empty() {
        bail!("add a Gemini API key in Settings > AI assistant");
    }
    let mut summary = String::new();
    let mut past: Vec<(String, String)> = Vec::new();
    with_db(&mut |conn| {
        summary = library_summary(conn)?;
        past = history(conn, 10)?
            .into_iter()
            .map(|m| {
                let role = if m.role == "user" { "user" } else { "model" };
                let picked: Vec<String> = m.picks.iter().map(|c| c.name.clone()).collect();
                let text = if picked.is_empty() {
                    m.text
                } else {
                    format!("{} [Recommended: {}]", m.text, picked.join(", "))
                };
                (role.to_string(), text)
            })
            .collect();
        Ok(())
    })?;

    // Pass 1: intent.
    let year = 1970 + unix_now() / 31_557_600;
    let mut turns = past.clone();
    turns.push((
        "user".into(),
        format!("Current year: {year}. Library: {summary}\n\nRequest: {message}"),
    ));
    let plan = generate(cfg, INTENT_PROMPT, &turns).await?;

    // Candidates from the library.
    let mut candidates: Vec<Candidate> = Vec::new();
    with_db(&mut |conn| {
        candidates = gather_candidates(conn, &plan)?;
        Ok(())
    })?;

    // Pass 2: choose.
    let list: Vec<Value> = candidates
        .iter()
        .map(|c| {
            json!({ "id": c.id, "title": c.name, "year": c.year, "kind": c.kind,
                    "genres": c.genres, "rating": c.rating, "language": c.language, "audio": c.audio })
        })
        .collect();
    let mut turns = past;
    turns.push((
        "user".into(),
        format!(
            "Request: {message}\n\nCandidates from the library ({} found):\n{}",
            list.len(),
            serde_json::to_string(&list)?
        ),
    ));
    let choice = generate(cfg, CHOOSE_PROMPT, &turns).await?;
    let allowed: BTreeSet<i64> = candidates.iter().map(|c| c.id).collect();
    let mut picks: Vec<i64> = Vec::new();
    for id in choice["picks"].as_array().into_iter().flatten().filter_map(Value::as_i64) {
        if allowed.contains(&id) && !picks.contains(&id) {
            picks.push(id);
        }
    }
    let reply = choice["reply"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("Gemini's answer had no reply"))?;

    let mut cards = Vec::new();
    with_db(&mut |conn| {
        save(conn, "user", message, &[])?;
        save(conn, "assistant", &reply, &picks)?;
        cards = library::cards_by_ids(conn, &picks)?;
        Ok(())
    })?;
    Ok(Answer { reply, picks: cards })
}

struct Candidate {
    id: i64,
    name: String,
    year: Option<u32>,
    kind: String,
    genres: Option<String>,
    rating: Option<f64>,
    language: Option<String>,
    audio: Option<String>,
}

const CANDIDATE_COLUMNS: &str = "id, name, year, kind, TRIM(genres, ','), rating, language, audio";

fn candidate(r: &rusqlite::Row) -> rusqlite::Result<Candidate> {
    Ok(Candidate {
        id: r.get(0)?,
        name: r.get(1)?,
        year: r.get(2)?,
        kind: r.get(3)?,
        genres: r.get(4)?,
        rating: r.get(5)?,
        language: r.get(6)?,
        audio: r.get(7)?,
    })
}

const MAX_CANDIDATES: usize = 120;

fn gather_candidates(conn: &Connection, plan: &Value) -> Result<Vec<Candidate>> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut add = |c: Candidate, out: &mut Vec<Candidate>| {
        if out.len() < MAX_CANDIDATES && seen.insert(c.id) {
            out.push(c);
        }
    };

    // 1. The model's concrete guesses, matched by title.
    let mut by_name = conn.prepare(&format!(
        "SELECT {CANDIDATE_COLUMNS} FROM titles
         WHERE id IN (SELECT rowid FROM titles_fts WHERE titles_fts MATCH ?1) LIMIT 10"
    ))?;
    for t in plan["titles"].as_array().into_iter().flatten() {
        let Some(name) = t["name"].as_str() else { continue };
        let Some(q) = fts_query(name) else { continue };
        let want = naming::title_key(name);
        let year = t["year"].as_u64().map(|y| y as u32);
        let rows: Vec<Candidate> = by_name
            .query_map([q], candidate)?
            .collect::<rusqlite::Result<_>>()?;
        // Exact title matches only, so "Up" doesn't drag in "Upgrade".
        for c in rows {
            let same_name = naming::title_key(&c.name) == want;
            let year_ok = match (year, c.year) {
                (Some(a), Some(b)) => a.abs_diff(b) <= 1,
                _ => true,
            };
            if same_name && year_ok {
                add(c, &mut out);
            }
        }
    }

    // 2. Filter matches, best rated first.
    let mut sql = format!("SELECT {CANDIDATE_COLUMNS} FROM titles WHERE 1=1");
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(k) = plan["kind"].as_str().filter(|k| matches!(*k, "movie" | "show")) {
        sql.push_str(" AND kind = ?");
        args.push(k.to_string().into());
    }
    if let Some(l) = plan["language"].as_str().filter(|l| l.len() == 2) {
        sql.push_str(" AND language = ?");
        args.push(l.to_string().into());
    }
    if let Some(y) = plan["year_from"].as_i64() {
        sql.push_str(" AND year >= ?");
        args.push(y.into());
    }
    if let Some(y) = plan["year_to"].as_i64() {
        sql.push_str(" AND year <= ?");
        args.push(y.into());
    }
    let genres: Vec<String> = plan["genres"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| g.as_str().map(String::from))
        .collect();
    if !genres.is_empty() {
        let ors: Vec<&str> = genres.iter().map(|_| "genres LIKE ?").collect();
        sql.push_str(&format!(" AND ({})", ors.join(" OR ")));
        for g in &genres {
            args.push(format!("%,{g},%").into());
        }
    }
    let filtered = !args.is_empty();
    sql.push_str(" ORDER BY rating IS NULL, rating DESC, added_at DESC LIMIT 100");
    if filtered {
        let mut stmt = conn.prepare(&sql)?;
        for c in stmt.query_map(params_from_iter(args.iter()), candidate)? {
            add(c?, &mut out);
        }
    }

    // 3. Keywords, for libraries without looked-up genres.
    let keywords: Vec<&str> = plan["keywords"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if out.len() < 30 && !keywords.is_empty() {
        let mut stmt = conn.prepare(&format!(
            "SELECT {CANDIDATE_COLUMNS} FROM titles
             WHERE id IN (SELECT rowid FROM titles_fts WHERE titles_fts MATCH ?1) LIMIT 40"
        ))?;
        let q: Vec<String> = keywords.iter().filter_map(|k| fts_query(k)).collect();
        if !q.is_empty() {
            for c in stmt.query_map([q.join(" OR ")], candidate)? {
                add(c?, &mut out);
            }
        }
    }

    // 4. Still thin: popular picks of the requested kind, so the model has
    // something real to choose from.
    if out.len() < 15 {
        let kind = plan["kind"].as_str().unwrap_or("");
        let mut stmt = conn.prepare(&format!(
            "SELECT {CANDIDATE_COLUMNS} FROM titles WHERE (?1 = '' OR kind = ?1)
             ORDER BY rating IS NULL, rating DESC, added_at DESC LIMIT 60"
        ))?;
        for c in stmt.query_map([kind], candidate)? {
            add(c?, &mut out);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_models_only() {
        assert!(is_chat_model("gemini-2.5-flash"));
        assert!(is_chat_model("gemini-2.5-pro"));
        assert!(!is_chat_model("gemini-2.5-flash-preview-tts"));
        assert!(!is_chat_model("gemini-2.5-flash-image-preview"));
        assert!(!is_chat_model("gemini-2.5-flash-native-audio-preview"));
        assert!(!is_chat_model("gemini-live-2.5-flash"));
        assert!(!is_chat_model("gemini-embedding-001"));
        assert!(!is_chat_model("gemini-nano-banana-2.1"));
        assert!(!is_chat_model("gemini-3.5-transcribe"));
    }

    #[test]
    fn fences() {
        assert_eq!(strip_fences("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_fences("{\"a\":1}"), "{\"a\":1}");
    }
}

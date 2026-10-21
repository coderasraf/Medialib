//! TOML configuration: where to crawl, what counts as media, how to play it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

// `deny_unknown_fields` on every table, not just for tidiness. In TOML a bare
// key belongs to the most recent `[table]` / `[[array]]` header above it, so a
// `skip_dirs = [...]` written *below* `[[roots]]` silently becomes a field of
// that root. With serde's default (ignore unknown fields) the top-level
// `skip_dirs` then falls back to empty and nothing reports a problem. Denying
// unknown fields turns that into a parse error pointing at the line.
//
// Field order matters for saving: TOML needs plain values before tables, so
// `player` and `roots` come last.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Relative paths are resolved against the config file's directory.
    #[serde(default = "default_db_path")]
    pub db_path: String,
    /// Directory pages fetched in parallel.
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// Pause between depth levels, to be polite to the server.
    #[serde(default = "default_delay_ms")]
    pub delay_ms: u64,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Guards against symlink loops, which produce ever-longer *distinct*
    /// URLs and so are never caught by the visited set.
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    #[serde(default = "default_media_extensions")]
    pub media_extensions: Vec<String>,
    /// Case-insensitive substring match against directory names.
    #[serde(default)]
    pub skip_dirs: Vec<String>,
    #[serde(default)]
    pub player: Player,
    #[serde(default)]
    pub tmdb: Tmdb,
    #[serde(default)]
    pub assistant: Assistant,
    #[serde(default)]
    pub roots: Vec<Root>,

    /// Directory the config was loaded from; not part of the file.
    #[serde(skip)]
    pub base_dir: PathBuf,
}

/// A place to index: either an HTTP directory listing (`url`) or a folder
/// on this machine (`path`). Exactly one of the two must be set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Root {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// What the folder holds; decides how files are grouped into titles.
    #[serde(default)]
    pub kind: SourceKind,
}

/// Movies: every movie in its own folder (or loose files).
/// Shows: show folders, optionally with season folders, holding episodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    #[default]
    Movies,
    Shows,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Movies => "movies",
            SourceKind::Shows => "shows",
        }
    }
}

pub enum RootKind {
    Http(Url),
    Local(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Player {
    pub command: String,
    /// `{url}` in any argument is replaced with the media URL or file path.
    #[serde(default = "default_player_args")]
    pub args: Vec<String>,
    /// Play browser-compatible files (mp4, webm) inside the app instead.
    #[serde(default)]
    pub prefer_builtin: bool,
}

impl Default for Player {
    fn default() -> Self {
        Player {
            command: default_player_command(),
            args: default_player_args(),
            prefer_builtin: false,
        }
    }
}

/// The Movie Database: posters, backdrops, genres, ratings, episode info.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tmdb {
    /// A v3 API key or a v4 read access token; both work.
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_tmdb_language")]
    pub language: String,
    /// Look up new titles automatically after each sync.
    #[serde(default = "default_true")]
    pub auto_lookup: bool,
}

impl Default for Tmdb {
    fn default() -> Self {
        Tmdb {
            api_key: String::new(),
            language: default_tmdb_language(),
            auto_lookup: true,
        }
    }
}

/// The watch assistant (Google Gemini).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assistant {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_assistant_model")]
    pub model: String,
}

impl Default for Assistant {
    fn default() -> Self {
        Assistant {
            api_key: String::new(),
            model: default_assistant_model(),
        }
    }
}

fn default_tmdb_language() -> String {
    "en-US".into()
}
fn default_assistant_model() -> String {
    "gemini-2.5-flash".into()
}
fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            db_path: default_db_path(),
            concurrency: default_concurrency(),
            delay_ms: default_delay_ms(),
            timeout_secs: default_timeout_secs(),
            max_depth: default_max_depth(),
            media_extensions: default_media_extensions(),
            skip_dirs: ["sample", "featurettes", "extras"].map(String::from).to_vec(),
            player: Player::default(),
            tmdb: Tmdb::default(),
            assistant: Assistant::default(),
            roots: Vec::new(),
            base_dir: PathBuf::from("."),
        }
    }
}

fn default_db_path() -> String {
    "medialib.db".into()
}
fn default_concurrency() -> usize {
    6
}
fn default_delay_ms() -> u64 {
    250
}
fn default_timeout_secs() -> u64 {
    20
}
fn default_max_depth() -> usize {
    12
}
fn default_player_args() -> Vec<String> {
    vec!["{url}".into()]
}
fn default_media_extensions() -> Vec<String> {
    [
        "mkv", "mp4", "m4v", "avi", "mov", "webm", "wmv", "flv", "ts", "m2ts", "mpg", "mpeg",
        "ogv",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// VLC's installer doesn't put `vlc` on PATH on Windows, so prefer the
/// standard install location when it exists.
fn default_player_command() -> String {
    if cfg!(windows) {
        for base in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Ok(dir) = std::env::var(base) {
                let exe = Path::new(&dir).join("VideoLAN").join("VLC").join("vlc.exe");
                if exe.is_file() {
                    return exe.to_string_lossy().into_owned();
                }
            }
        }
    }
    "vlc".into()
}

const FILE_HEADER: &str = "\
# medialib configuration (saved by the app; hand edits are fine too).
#
# Every bare key must come BEFORE the first [player] or [[roots]] header:
# in TOML a bare key belongs to the most recent header above it, so a
# `skip_dirs` written below [[roots]] would become a field of that root.

";

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path).with_context(|| {
            format!(
                "reading config {} (copy config.example.toml to get started)",
                path.display()
            )
        })?;
        let mut cfg =
            Self::from_toml_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        cfg.base_dir = base_dir_of(path);
        Ok(cfg)
    }

    /// Loads the config, or writes and returns defaults if there is none.
    pub fn load_or_create(path: &Path) -> Result<Config> {
        if path.exists() {
            return Self::load(path);
        }
        let mut cfg = Config::default();
        cfg.base_dir = base_dir_of(path);
        cfg.save(path)?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self).context("serialising config")?;
        // Write-then-rename so a crash mid-write can't leave a truncated file.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, format!("{FILE_HEADER}{body}"))
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    pub fn from_toml_str(text: &str) -> Result<Config> {
        let mut cfg: Config = toml::from_str(text).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("unknown field") {
                anyhow!(
                    "{msg}\nhint: bare keys (skip_dirs, media_extensions, ...) must come before \
                     the first [[roots]] or [player] header; below it they belong to that table"
                )
            } else {
                anyhow!(msg)
            }
        })?;
        cfg.normalise()?;
        Ok(cfg)
    }

    /// Validates and canonicalises the config in place. Also used by the app
    /// after every edit, so the file on disk is always loadable.
    pub fn normalise(&mut self) -> Result<()> {
        self.concurrency = self.concurrency.clamp(1, 64);
        self.max_depth = self.max_depth.max(1);
        self.timeout_secs = self.timeout_secs.max(1);
        self.media_extensions = dedup(
            self.media_extensions
                .iter()
                .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase()),
        );
        self.skip_dirs = dedup(self.skip_dirs.iter().map(|s| s.trim().to_lowercase()));
        self.tmdb.api_key = self.tmdb.api_key.trim().to_string();
        if self.tmdb.language.trim().is_empty() {
            self.tmdb.language = default_tmdb_language();
        }
        self.assistant.api_key = self.assistant.api_key.trim().to_string();
        self.assistant.model = self.assistant.model.trim().trim_start_matches("models/").to_string();
        if self.assistant.model.is_empty() {
            self.assistant.model = default_assistant_model();
        }

        let mut names = HashSet::new();
        for root in &mut self.roots {
            root.name = root.name.trim().to_string();
            if root.name.is_empty() {
                bail!("every root needs a name");
            }
            if !names.insert(root.name.to_lowercase()) {
                bail!("duplicate root name {:?}", root.name);
            }
            match (&mut root.url, &root.path) {
                (Some(url), None) => {
                    // The trailing slash matters twice: `Url::join` resolves
                    // relative hrefs against the last `/`, and the crawler's
                    // "stay inside the root" check is a prefix match.
                    *url = url.trim().to_string();
                    if !url.ends_with('/') {
                        url.push('/');
                    }
                    let parsed = Url::parse(url)
                        .with_context(|| format!("root {:?}: bad url {url:?}", root.name))?;
                    if !matches!(parsed.scheme(), "http" | "https") {
                        bail!("root {:?}: only http(s) URLs are supported", root.name);
                    }
                }
                (None, Some(_)) => {}
                _ => bail!("root {:?}: set exactly one of `url` or `path`", root.name),
            }
        }
        Ok(())
    }

    /// The SQLite file, with relative paths taken from the config's folder.
    pub fn db_file(&self) -> PathBuf {
        let p = Path::new(&self.db_path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.base_dir.join(p)
        }
    }

    pub fn is_media(&self, name: &str) -> bool {
        name.rsplit_once('.').is_some_and(|(_, ext)| {
            let ext = ext.to_ascii_lowercase();
            self.media_extensions.iter().any(|m| *m == ext)
        })
    }

    pub fn should_skip_dir(&self, name: &str) -> bool {
        let name = name.trim_end_matches(['/', '\\']).to_lowercase();
        self.skip_dirs.iter().any(|s| name.contains(s.as_str()))
    }

    pub fn root(&self, name: &str) -> Option<&Root> {
        self.roots.iter().find(|r| r.name == name)
    }
}

impl Root {
    pub fn kind(&self) -> Result<RootKind> {
        match (&self.url, &self.path) {
            (Some(url), _) => Ok(RootKind::Http(Url::parse(url).with_context(|| {
                format!("root {:?}: bad url {url:?}", self.name)
            })?)),
            (None, Some(path)) => Ok(RootKind::Local(PathBuf::from(path))),
            (None, None) => bail!("root {:?} has neither url nor path", self.name),
        }
    }

    /// Human-readable location, for listings.
    pub fn location(&self) -> &str {
        self.url.as_deref().or(self.path.as_deref()).unwrap_or("")
    }
}

fn base_dir_of(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn dedup(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
skip_dirs = ["Sample", "extras"]
media_extensions = [".MKV", "mp4"]

[[roots]]
name = "nas"
url = "http://media.local/share"

[[roots]]
name = "disk"
path = 'D:\Videos'
"#;

    #[test]
    fn parses_and_normalises() {
        let cfg = Config::from_toml_str(GOOD).unwrap();
        assert_eq!(cfg.roots[0].url.as_deref(), Some("http://media.local/share/"));
        assert!(matches!(cfg.roots[1].kind().unwrap(), RootKind::Local(_)));
        assert!(cfg.is_media("Movie.mkv"));
        assert!(cfg.is_media("movie.MP4"));
        assert!(!cfg.is_media("readme.txt"));
        assert!(cfg.should_skip_dir("Movie.SAMPLE/"));
        assert!(!cfg.should_skip_dir("Season 01"));
        assert_eq!(cfg.concurrency, 6);
        assert_eq!(cfg.roots[0].kind, SourceKind::Movies);
    }

    #[test]
    fn key_below_roots_is_an_error_not_silently_ignored() {
        let text = r#"
[[roots]]
name = "nas"
url = "http://media.local/"
skip_dirs = ["sample"]
"#;
        let err = format!("{:#}", Config::from_toml_str(text).unwrap_err());
        assert!(err.contains("skip_dirs"), "{err}");
    }

    #[test]
    fn root_needs_exactly_one_location() {
        let both = "[[roots]]\nname = \"x\"\nurl = \"http://a/\"\npath = \"C:/\"\n";
        assert!(Config::from_toml_str(both).is_err());
        let neither = "[[roots]]\nname = \"x\"\n";
        assert!(Config::from_toml_str(neither).is_err());
    }

    #[test]
    fn save_round_trips() {
        let mut cfg = Config::from_toml_str(GOOD).unwrap();
        cfg.player.command = r"C:\Program Files\VideoLAN\VLC\vlc.exe".into();
        let text = format!("{FILE_HEADER}{}", toml::to_string_pretty(&cfg).unwrap());
        let back = Config::from_toml_str(&text).unwrap();
        assert_eq!(back.roots.len(), 2);
        assert_eq!(back.skip_dirs, cfg.skip_dirs);
        assert_eq!(back.player.command, cfg.player.command);
    }
}

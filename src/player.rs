//! Hands a media URL to an external player.

use std::io::ErrorKind;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use url::Url;

use crate::config::Player;

/// Accepts only http(s) URLs with a host. The point is not shell injection
/// (see `play`) but not pointing a player at `file:///etc/passwd`, a
/// `javascript:` URL, or whatever other scheme handlers the player supports.
pub fn validate_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw.trim()).with_context(|| format!("not a valid URL: {raw:?}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => bail!("refusing to play a {other}: URL; only http and https are allowed"),
    }
    if url.host_str().map_or(true, str::is_empty) {
        bail!("URL has no host: {raw:?}");
    }
    Ok(url)
}

pub fn play(cfg: &Player, raw_url: &str) -> Result<()> {
    let url = validate_url(raw_url)?;
    spawn(cfg, url.as_str())
}

/// Plays a URL taken from the index. Remote entries go through `play` and
/// its http(s)-only check. Local entries are stored as `file:` URLs, and
/// only these are passed to the player, as a plain path. Accepting `file:`
/// here is safe only because callers look the URL up in the index first.
pub fn play_indexed(cfg: &Player, indexed_url: &str) -> Result<()> {
    let url = Url::parse(indexed_url)?;
    if url.scheme() != "file" {
        return play(cfg, indexed_url);
    }
    let path = url
        .to_file_path()
        .map_err(|_| anyhow!("not a local file path: {indexed_url}"))?;
    if !path.is_file() {
        bail!("file no longer exists: {}", path.display());
    }
    spawn(cfg, &path.to_string_lossy())
}

/// Opens a file the app wrote itself (a playlist).
pub fn play_file(cfg: &Player, path: &std::path::Path) -> Result<()> {
    spawn(cfg, &path.to_string_lossy())
}

fn spawn(cfg: &Player, target: &str) -> Result<()> {
    let mut args: Vec<String> = cfg.args.iter().map(|a| a.replace("{url}", target)).collect();
    if !cfg.args.iter().any(|a| a.contains("{url}")) {
        args.push(target.to_string());
    }

    // `Command` execs the program directly with an argv array; no shell is
    // involved, so a filename full of `;`, `$()` or quotes is just bytes in
    // one argument. There is no injection surface to escape for.
    let child = Command::new(&cfg.command)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => anyhow!(
                "player {:?} not found; install it or set player.command in the config \
                 to its full path",
                cfg.command
            ),
            _ => anyhow!(e).context(format!("starting player {:?}", cfg.command)),
        })?;

    // Reap the child when it exits so long-running `serve` sessions don't
    // accumulate zombie processes on Unix. The thread just dies with us if
    // the CLI exits first; the player keeps running either way.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bogus_player() -> Player {
        Player {
            command: "medialib-test-no-such-player".into(),
            args: vec!["{url}".into()],
            prefer_builtin: false,
        }
    }

    #[test]
    fn rejects_non_http_schemes() {
        for url in ["file:///etc/passwd", "javascript:alert(1)", "ftp://host/x.mkv"] {
            let err = play(&bogus_player(), url).unwrap_err().to_string();
            assert!(err.contains("only http and https"), "{url}: {err}");
        }
    }

    #[test]
    fn valid_url_reaches_spawn() {
        let err = play(&bogus_player(), "http://media.local/a.mkv").unwrap_err().to_string();
        assert!(err.contains("player.command"), "{err}");
    }
}

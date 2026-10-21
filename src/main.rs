//! medialib command-line interface. The desktop app is src/desktop.rs.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use medialib::config::Config;
use medialib::crawler::{self, CrawlMode, StderrProgress};
use medialib::index::{Hit, Index};
use medialib::{jobs, library, player, web};

#[derive(Parser)]
#[command(name = "medialib", version, about)]
struct Cli {
    /// Path to the TOML config file.
    #[arg(short, long, global = true, default_value = "config.toml")]
    config: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Walk the configured roots and build the index.
    Crawl {
        /// Only crawl the root with this name.
        #[arg(long)]
        root: Option<String>,
        /// Ignore checkpoints and re-walk everything.
        #[arg(long)]
        fresh: bool,
    },
    /// Search the local index (never touches the network).
    Search {
        #[arg(required = true)]
        terms: Vec<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Search, then open one result in the configured player.
    Play {
        #[arg(required = true)]
        terms: Vec<String>,
        /// Which result to play, 1-based.
        #[arg(long, default_value_t = 1)]
        pick: usize,
    },
    /// Run the local web UI on 127.0.0.1.
    Serve {
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
    /// Show index size.
    Stats,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(&cli.config)?;

    match cli.cmd {
        Cmd::Crawl { root, fresh } => crawl(&cfg, root.as_deref(), fresh),
        Cmd::Search { terms, limit } => {
            let index = Index::open(cfg.db_file())?;
            let hits = index.search(&terms.join(" "), limit)?;
            if hits.is_empty() {
                println!("no matches");
                hint_if_empty(&index)?;
            }
            for (i, hit) in hits.iter().enumerate() {
                print_hit(i + 1, hit);
            }
            Ok(())
        }
        Cmd::Play { terms, pick } => {
            if pick == 0 {
                bail!("--pick is 1-based");
            }
            let index = Index::open(cfg.db_file())?;
            let hits = index.search(&terms.join(" "), pick)?;
            let Some(hit) = hits.get(pick - 1) else {
                hint_if_empty(&index)?;
                bail!("only {} result(s) for that search", hits.len());
            };
            print_hit(pick, hit);
            player::play_indexed(&cfg.player, &hit.url)?;
            println!("-> {} {}", cfg.player.command, hit.url);
            Ok(())
        }
        Cmd::Serve { port } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(async {
                let server = web::Server::bind(cfg, cli.config.clone(), port).await?;
                println!("medialib UI on http://{}/  (Ctrl+C to stop)", server.addr()?);
                server.run().await
            })
        }
        Cmd::Stats => {
            let s = Index::open(cfg.db_file())?.stats()?;
            println!("files:        {}", s.files);
            println!("total size:   {} (where the server reports sizes)", human_size(s.bytes));
            for (root, n) in &s.per_root {
                println!("  {root}: {n}");
            }
            println!("dirs crawled: {}", s.visited);
            println!("dirs pending: {}", s.pending);
            Ok(())
        }
    }
}

fn crawl(cfg: &Config, only: Option<&str>, fresh: bool) -> Result<()> {
    let roots: Vec<_> = match only {
        Some(name) => vec![cfg
            .roots
            .iter()
            .find(|r| r.name == name)
            .with_context(|| format!("no root named {name:?} in the config"))?],
        None => cfg.roots.iter().collect(),
    };
    if roots.is_empty() {
        bail!("no [[roots]] configured");
    }
    let mode = if fresh { CrawlMode::Fresh } else { CrawlMode::Resume };
    let mut index = Index::open(cfg.db_file())?;

    for root in roots {
        let s = crawler::crawl_root(cfg, &mut index, root, mode, &StderrProgress)?;
        // Group files into movies / shows so the app sees what the CLI found.
        let titles = library::rebuild_titles(&mut index, root)?;
        let status = if s.errors > 0 { "error" } else { "ok" };
        let error = (s.errors > 0).then(|| format!("{} folders could not be read", s.errors));
        jobs::set_source_state(&index, &root.name, status, error.as_deref())?;
        println!(
            "{}: {} new directories, {} media files indexed, {} titles{}",
            root.name,
            s.dirs,
            s.files,
            titles,
            if s.resumed { " (resumed)" } else { "" }
        );
        if s.skipped_dirs > 0 {
            println!("  {} directories skipped by skip_dirs", s.skipped_dirs);
        }
        if s.too_deep > 0 {
            println!("  {} directories beyond max_depth = {}", s.too_deep, cfg.max_depth);
        }
        if s.pruned > 0 {
            println!("  {} files no longer on the server were removed", s.pruned);
        }
        if s.errors > 0 {
            println!("  {} directories failed; the next `crawl` retries them", s.errors);
        } else if s.dirs == 0 && !fresh {
            println!("  nothing pending; use --fresh to re-walk and pick up new files");
        }
    }
    Ok(())
}

fn hint_if_empty(index: &Index) -> Result<()> {
    if index.stats()?.files == 0 {
        eprintln!("the index is empty; run `medialib crawl` first");
    }
    Ok(())
}

fn print_hit(n: usize, hit: &Hit) {
    let mut label = hit.title.clone();
    match (hit.season, hit.episode, hit.year) {
        (Some(s), Some(e), _) => label.push_str(&format!("  S{s:02}E{e:02}")),
        (_, _, Some(y)) => label.push_str(&format!("  ({y})")),
        _ => {}
    }
    if let Some(size) = hit.size {
        label.push_str(&format!("  [{}]", human_size(size)));
    }
    println!("{n:>3}. {label}");
    println!("     {}", hit.name);
    println!("     {}", hit.dir);
}

fn human_size(bytes: i64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", units[i])
    }
}

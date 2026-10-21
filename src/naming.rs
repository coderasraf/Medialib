//! Title / season / episode / year extraction from release-style filenames.
//!
//! The approach: normalise separators, then find the earliest "cut point" —
//! an episode marker, a year, or the first release-metadata token — and treat
//! everything before it as the title. Release names put the title first and
//! the noise after it, so cutting is far more robust than trying to recognise
//! the title itself.

use std::sync::OnceLock;

use regex::Regex;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub title: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub year: Option<u32>,
}

// Each regex is compiled once and shared. `Regex::new` is expensive relative
// to matching, and `parse` runs once per indexed file, so compiling per call
// would dominate a crawl of a few hundred thousand files.

fn season_episode() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // S01E02 / s01 e02 (dots are already spaces), or 1x02.
        // No trailing \b on the SxxEyy form so multi-episode S01E02E03 matches.
        Regex::new(r"(?i)\bS(\d{1,2}) ?E(\d{1,3})|\b(\d{1,2})x(\d{2,3})\b").unwrap()
    })
}

fn year() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b(19\d{2}|20\d{2})\b").unwrap())
}

fn metadata() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Deliberately excludes short words that also occur in real titles
        // ("web", "dv", "uncut"): a false cut truncates the title.
        Regex::new(
            r"(?ix)\b(?:
                2160p|1440p|1080[pi]|720p|576p|480p|4k|uhd|
                hdr10|hdr|dolby\ ?vision|sdr|
                blu-?ray|bdrip|brrip|bdremux|web-?dl|web-?rip|hdtv|dvdrip|dvdscr|hdrip|hdcam|camrip|
                x26[45]|h\ ?26[45]|hevc|avc|xvid|divx|10\ ?bit|
                aac\d?|e?ac3|dts(?:-hd)?|truehd|atmos|ddp?\d|flac|
                remux|proper|repack|extended|unrated|imax|remastered|
                dual[\ -]audio|multi[\ -]audio|esubs?|cd\ ?\d|disc\ ?\d
            )\b",
        )
        .unwrap()
    })
}

fn leading_year() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[(\[]\s*((?:19|20)\d{2})\s*[)\]]\s*").unwrap())
}

fn date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // dd-mm-yyyy / dd mm yyyy (dots are spaces by now), optionally bracketed.
    RE.get_or_init(|| Regex::new(r"\(?\b\d{1,2}[-/ ]\d{1,2}[-/ ]((?:19|20)\d{2})\b\)?").unwrap())
}

fn leading_group() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // "[Group] Title - 01 [1080p]" style prefixes.
    RE.get_or_init(|| Regex::new(r"^\[[^\]]*\]\s*").unwrap())
}

pub fn parse(filename: &str) -> Parsed {
    let stem = strip_extension(filename);
    let normalised = stem
        .replace(['.', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut text = leading_group().replace(&normalised, "").into_owned();

    let mut parsed = Parsed::default();
    // "(1972) Pratidwandi": a bracketed year in front is the year, not the
    // title. A bare leading year stays ("2001 A Space Odyssey").
    if let Some(c) = leading_year().captures(&text) {
        parsed.year = c[1].parse().ok();
        text = text[c.get(0).unwrap().end()..].to_string();
    }
    let mut cut = text.len();

    if let Some(c) = season_episode().captures(&text) {
        let (s, e) = match (c.get(1), c.get(2)) {
            (Some(s), Some(e)) => (s, e),
            _ => (c.get(3).unwrap(), c.get(4).unwrap()),
        };
        parsed.season = s.as_str().parse().ok();
        parsed.episode = e.as_str().parse().ok();
        cut = cut.min(c.get(0).unwrap().start());
    }
    if let Some(m) = metadata().find(&text) {
        cut = cut.min(m.start());
    }
    // "(TV Documentary 2016)", "(TV Special 2018)": the tag carries the year
    // and ends the title.
    if let Some(c) = tv_series_tag().captures(&text).filter(|c| c.get(0).unwrap().start() > 0) {
        if c.get(0).unwrap().start() < cut {
            cut = c.get(0).unwrap().start();
            parsed.year = c[1].parse().ok();
        }
    }
    // Event dates, "WWE Battleground (19-07-2015)": the year, and the end of
    // the title.
    if let Some(c) = date_re().captures(&text).filter(|c| c.get(0).unwrap().start() > 0) {
        if c.get(0).unwrap().start() < cut {
            cut = c.get(0).unwrap().start();
            parsed.year = c[1].parse().ok();
        }
    }
    // Of the years before the other cut points, take the *last* one: in
    // "Blade Runner 2049 2017 1080p" the first is part of the title. A year at
    // position 0 is always the title ("1917", "2001 A Space Odyssey").
    if parsed.year.is_some() {
        // Already found in a leading year, tag or date.
    } else if let Some(m) = year()
        .find_iter(&text)
        .filter(|m| m.start() > 0 && m.start() < cut)
        .last()
    {
        parsed.year = m.as_str().parse().ok();
        cut = m.start();
    }

    let title = text[..cut]
        .trim_end_matches(|c: char| c.is_whitespace() || "-([{.,:_".contains(c))
        .trim();
    parsed.title = if title.is_empty() {
        text.trim().to_string()
    } else {
        title.to_string()
    };
    parsed
}

/// A show folder name such as "A Bloody Lucky Day (TV Series 2023– ) 720p".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShowFolder {
    pub title: String,
    pub year: Option<u32>,
    pub year_end: Option<u32>,
    /// An open year range ("2023– ") means the show is still airing.
    pub ongoing: bool,
}

fn tv_series_tag() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\(\s*(?:tv\s+)?(?:mini[\s-]+)?(?:series|shows?|special|documentary|movie|short|video)\s+(\d{4})\s*(?:([–—-])\s*(\d{4})?)?\s*\)",
        )
        .unwrap()
    })
}

pub fn parse_show_folder(name: &str) -> ShowFolder {
    if let Some(c) = tv_series_tag().captures(name) {
        let title = name[..c.get(0).unwrap().start()]
            .trim_end_matches(|c: char| c.is_whitespace() || "-([{.,:_".contains(c))
            .trim();
        if !title.is_empty() {
            let end = c.get(3).and_then(|m| m.as_str().parse().ok());
            return ShowFolder {
                title: title.replace(['.', '_'], " "),
                year: c[1].parse().ok(),
                year_end: end,
                ongoing: c.get(2).is_some() && end.is_none(),
            };
        }
    }
    let p = parse(name);
    ShowFolder {
        title: p.title,
        year: p.year,
        year_end: None,
        ongoing: false,
    }
}

fn season_dir() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^\s*(?:season|series|s)\s*[-._]?\s*(\d{1,2})\b").unwrap())
}

/// "Season 1", "S02", "Season 03 [Dual Audio]" -> season number;
/// "Specials" -> 0.
pub fn season_folder(name: &str) -> Option<u32> {
    if let Some(c) = season_dir().captures(name) {
        return c[1].parse().ok();
    }
    let lower = name.trim().to_lowercase();
    (lower == "specials" || lower == "special").then_some(0)
}

fn loose_episode_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(?:episode|ep|e)\s*[-._]?\s*(\d{1,3})\b").unwrap())
}

/// Episode number from names without a season marker: "E05", "Ep 5",
/// "Episode 05".
pub fn loose_episode(name: &str) -> Option<u32> {
    loose_episode_re()
        .captures(&strip_extension(name).replace(['.', '_'], " "))
        .and_then(|c| c[1].parse().ok())
}

/// Episode number as a bare trailing number: "Arang and Magistrate 01.mkv".
/// Only used inside show folders, where such a number can't be a sequel.
pub fn trailing_number(name: &str) -> Option<u32> {
    let stem = strip_extension(name).replace(['.', '_', '-'], " ");
    let last = stem.split_whitespace().last()?;
    (last.len() <= 3 && last.chars().all(|c| c.is_ascii_digit()))
        .then(|| last.parse().ok())
        .flatten()
}

fn quality_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(2160p|4k|uhd|1080p|1080i|720p|576p|480p|360p)\b").unwrap())
}

/// Highest resolution mentioned anywhere in the text.
pub fn quality(text: &str) -> Option<&'static str> {
    quality_re()
        .find_iter(text)
        .map(|m| match m.as_str().to_ascii_lowercase().as_str() {
            "2160p" | "4k" | "uhd" => (4, "2160p"),
            "1080p" | "1080i" => (3, "1080p"),
            "720p" => (2, "720p"),
            "576p" | "480p" => (1, "480p"),
            _ => (0, "360p"),
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, q)| q)
}

fn audio_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(multi|dual)[\s._-]*audio\b|\[([^\]]*\+[^\]]*)\]").unwrap()
    })
}

/// "multi" for multi-audio releases, "dual" for dual-audio ones. Besides
/// the explicit tags, a bracket like "[Hindi + Tamil + Telugu]" counts its
/// languages.
pub fn audio(text: &str) -> Option<&'static str> {
    let mut best = None;
    for c in audio_re().captures_iter(text) {
        let found = if let Some(tag) = c.get(1) {
            if tag.as_str().eq_ignore_ascii_case("multi") {
                "multi"
            } else {
                "dual"
            }
        } else {
            let langs = c[2]
                .split('+')
                .filter(|p| p.chars().any(|ch| ch.is_alphabetic()))
                .count();
            match langs {
                0 | 1 => continue,
                2 => "dual",
                _ => "multi",
            }
        };
        if found == "multi" {
            return Some("multi");
        }
        best = Some(found);
    }
    best
}

/// Streaming-service tags in release names, e.g. "...720p.NF.WEB-DL...".
const PLATFORMS: &[(&str, &str)] = &[
    ("NF", "Netflix"),
    ("NETFLIX", "Netflix"),
    ("AMZN", "Prime Video"),
    ("DSNP", "Disney+"),
    ("HMAX", "HBO Max"),
    ("ATVP", "Apple TV+"),
    ("HULU", "Hulu"),
    ("PCOK", "Peacock"),
    ("PMTP", "Paramount+"),
    ("HS", "Hotstar"),
    ("HOTSTAR", "Hotstar"),
    ("JC", "JioCinema"),
    ("ZEE5", "ZEE5"),
    ("SONYLIV", "SonyLIV"),
    ("BMS", "BookMyShow"),
    ("AHA", "aha"),
    ("SUNNXT", "Sun NXT"),
    ("VIKI", "Viki"),
    ("IQIYI", "iQIYI"),
    ("TVING", "TVING"),
    ("WAVVE", "Wavve"),
    ("CPNG", "Coupang Play"),
    ("KCW", "Kocowa"),
    ("CRAV", "Crave"),
];

/// The streaming platform a release came from, if its name says so. Only
/// tokens after the title are considered, so a film called "Stan" or "Aha"
/// isn't mistaken for a platform tag.
pub fn platform(text: &str) -> Option<&'static str> {
    let normalised = text.replace(['.', '_'], " ");
    let start = [metadata().find(&normalised), quality_re().find(&normalised), year().find(&normalised)]
        .into_iter()
        .flatten()
        .map(|m| m.start())
        .filter(|&s| s > 0)
        .min()?;
    normalised[start..]
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .find_map(|token| {
            let upper = token.to_ascii_uppercase();
            // Two-letter tags must be written in capitals to count.
            if upper.len() <= 2 && token != upper {
                return None;
            }
            PLATFORMS.iter().find(|(tag, _)| *tag == upper).map(|(_, name)| *name)
        })
}

/// Grouping key for titles: lowercase letters and digits only, so
/// "Spider-Man: No Way Home" and "Spider Man No Way Home" collide.
pub fn title_key(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Removes a short alphanumeric extension; leaves "Mr. Robot" alone.
fn strip_extension(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            stem
        }
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(title: &str, season: Option<u32>, episode: Option<u32>, year: Option<u32>) -> Parsed {
        Parsed {
            title: title.into(),
            season,
            episode,
            year,
        }
    }

    #[test]
    fn scene_episode() {
        assert_eq!(
            parse("Breaking.Bad.S03E07.1080p.BluRay.x265.mkv"),
            p("Breaking Bad", Some(3), Some(7), None)
        );
    }

    #[test]
    fn nxnn_episode() {
        assert_eq!(
            parse("The Wire - 2x05 - Undertow.mkv"),
            p("The Wire", Some(2), Some(5), None)
        );
    }

    #[test]
    fn movie_with_year() {
        assert_eq!(
            parse("Arrival (2016) 1080p WEB-DL x264.mp4"),
            p("Arrival", None, None, Some(2016))
        );
    }

    #[test]
    fn plain_title() {
        assert_eq!(parse("Some Documentary.mkv"), p("Some Documentary", None, None, None));
    }

    #[test]
    fn year_shaped_titles() {
        assert_eq!(parse("1917.2019.1080p.mkv"), p("1917", None, None, Some(2019)));
        assert_eq!(
            parse("Blade.Runner.2049.2017.2160p.mkv"),
            p("Blade Runner 2049", None, None, Some(2017))
        );
    }

    #[test]
    fn tagged_and_dated_names() {
        assert_eq!(parse("13th (TV Documentary 2016) 720p WEBRip"), p("13th", None, None, Some(2016)));
        assert_eq!(
            parse("14 Peaks-Nothing Is Impossible (TV Documentary 2021) 720p NF [Dual Audio]"),
            p("14 Peaks-Nothing Is Impossible", None, None, Some(2021))
        );
        assert_eq!(parse("Adam Sandler-100% Fresh (TV Special 2018) 720p"), p("Adam Sandler-100% Fresh", None, None, Some(2018)));
        assert_eq!(parse("WWE Battleground (19-07-2015) HDTV"), p("WWE Battleground", None, None, Some(2015)));
        assert_eq!(parse("(1972) Pratidwandi-CD2"), p("Pratidwandi", None, None, Some(1972)));
        assert_eq!(parse("Pratidwandi (1972)-CD2.avi"), p("Pratidwandi", None, None, Some(1972)));
        assert_eq!(parse("2001 A Space Odyssey (1968) 1080p.mkv"), p("2001 A Space Odyssey", None, None, Some(1968)));
        assert_eq!(parse("WWE.Backlash.2016.Kickoff.WEB.h264-HEEL.mp4").year, Some(2016));
        let f = parse_show_folder("Aapka Apna Zakir (TV Shows 2024– ) 720p");
        assert_eq!((f.title.as_str(), f.year, f.ongoing), ("Aapka Apna Zakir", Some(2024), true));
    }

    #[test]
    fn show_folders() {
        let f = parse_show_folder("A Bloody Lucky Day (TV Series 2023– ) 720p [Dual Audio]");
        assert_eq!(f.title, "A Bloody Lucky Day");
        assert_eq!((f.year, f.year_end, f.ongoing), (Some(2023), None, true));
        let f = parse_show_folder("2 Weeks (TV Mini Series 2013) 720p");
        assert_eq!((f.title.as_str(), f.year, f.ongoing), ("2 Weeks", Some(2013), false));
        let f = parse_show_folder("Tale of the Nine Tailed (TV Series 2020–2023) 720p");
        assert_eq!((f.year, f.year_end), (Some(2020), Some(2023)));
        let f = parse_show_folder("Some.Show.2019.720p");
        assert_eq!((f.title.as_str(), f.year), ("Some Show", Some(2019)));
        assert_eq!(season_folder("Season 1"), Some(1));
        assert_eq!(season_folder("S02 [Dual Audio]"), Some(2));
        assert_eq!(season_folder("Specials"), Some(0));
        assert_eq!(season_folder("Seasonal Stories"), None);
        assert_eq!(loose_episode("Some Show E05 720p.mkv"), Some(5));
        assert_eq!(loose_episode("Episode 12.mp4"), Some(12));
        assert_eq!(trailing_number("Arang and Magistrate 01.mkv"), Some(1));
        assert_eq!(trailing_number("Some Show 720p.mkv"), None);
        assert_eq!(trailing_number("Movie 2019.mkv"), None);
    }

    #[test]
    fn release_tags() {
        let s = "A Bloody Lucky Day S01E01  720p WEBRip x265 HEVC ESub [Dual Audio][Korean 5.1+English 5.1] -MsMod.mkv";
        assert_eq!(quality(s), Some("720p"));
        assert_eq!(audio(s), Some("dual"));
        assert_eq!(audio("Movie (2024) [Hindi + Tamil + Telugu]"), Some("multi"));
        assert_eq!(audio("Movie (2024) [Multi Audio]"), Some("multi"));
        assert_eq!(audio("Movie (2024) 720p"), None);
        assert_eq!(quality("Movie 2160p and 720p"), Some("2160p"));
        assert_eq!(platform("An.Almost.Christmas.Story.2024.720p.DSNP.WEBRip.x264.mkv"), Some("Disney+"));
        assert_eq!(platform("Teach.You.a.Lesson.S01E01.720p.NF.WEB-DL.mkv"), Some("Netflix"));
        assert_eq!(platform("Appu (2024) 720p BMS [Dual Audio]"), Some("BookMyShow"));
        assert_eq!(platform("Stan & Ollie (2018) 1080p BluRay"), None);
        assert_eq!(title_key("Spider-Man: No Way Home"), "spidermannowayhome");
    }

    #[test]
    fn dotted_episode_and_group_prefix() {
        assert_eq!(parse("show_name.s01.e02.mkv"), p("show name", Some(1), Some(2), None));
        assert_eq!(
            parse("[Group] Some Show S02E10 [1080p].mkv"),
            p("Some Show", Some(2), Some(10), None)
        );
    }
}

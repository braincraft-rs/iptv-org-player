//! Download the iptv-org JSON API files and join them into a [`Catalog`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::db::Db;
use crate::model::{Catalog, Channel, Country, Stream};

const API_BASE: &str = "https://iptv-org.github.io/api";

#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub countries: usize,
    pub channels: usize,
    pub streams: usize,
}

#[derive(Debug, Clone)]
pub enum SyncMsg {
    Progress(String),
    Done(SyncSummary),
    Failed(String),
}

// ---- raw API shapes (only the fields we use) -------------------------------

#[derive(Deserialize)]
struct RawCountry {
    code: String,
    name: String,
    #[serde(default)]
    flag: Option<String>,
}

#[derive(Deserialize)]
struct RawChannel {
    id: String,
    name: String,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    is_nsfw: bool,
    #[serde(default)]
    closed: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RawStream {
    #[serde(default)]
    channel: Option<String>,
    url: String,
    #[serde(default)]
    quality: Option<String>,
    #[serde(default)]
    referrer: Option<String>,
    #[serde(default)]
    user_agent: Option<String>,
}

#[derive(Deserialize)]
struct RawLogo {
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    in_use: Option<bool>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    url: String,
}

// ---- join ------------------------------------------------------------------

/// Parse the four API payloads and join them.
///
/// Rules (see docs/DATA-SOURCES.md):
/// - drop closed channels
/// - drop streams without a known channel or with a non-http(s) URL
/// - drop duplicate (channel, url) pairs
/// - drop channels left without any stream
/// - NSFW channels are kept but flagged; filtering happens at query time
/// - logo = largest `in_use` logo of the channel
pub fn join(countries: &[u8], channels: &[u8], streams: &[u8], logos: &[u8]) -> Result<Catalog> {
    let raw_countries: Vec<RawCountry> =
        serde_json::from_slice(countries).context("parsing countries.json")?;
    let raw_channels: Vec<RawChannel> =
        serde_json::from_slice(channels).context("parsing channels.json")?;
    let raw_streams: Vec<RawStream> =
        serde_json::from_slice(streams).context("parsing streams.json")?;
    let raw_logos: Vec<RawLogo> = serde_json::from_slice(logos).context("parsing logos.json")?;

    // Best logo per channel: in use, largest area (unknown size counts as 0).
    let mut best_logo: HashMap<&str, (u64, &str)> = HashMap::new();
    for l in &raw_logos {
        let Some(ch) = l.channel.as_deref() else {
            continue;
        };
        if l.in_use == Some(false) {
            continue;
        }
        let area = u64::from(l.width.unwrap_or(0)) * u64::from(l.height.unwrap_or(0));
        let entry = best_logo.entry(ch).or_insert((area, l.url.as_str()));
        if area > entry.0 {
            *entry = (area, l.url.as_str());
        }
    }

    let open_channels: HashMap<&str, &RawChannel> = raw_channels
        .iter()
        .filter(|c| c.closed.as_ref().is_none_or(|v| v.is_null()))
        .map(|c| (c.id.as_str(), c))
        .collect();

    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    let mut streams_out = Vec::new();
    let mut channels_with_streams: HashSet<&str> = HashSet::new();
    for s in &raw_streams {
        let Some(ch) = s.channel.as_deref() else {
            continue;
        };
        if !open_channels.contains_key(ch) {
            continue;
        }
        if !(s.url.starts_with("http://") || s.url.starts_with("https://")) {
            continue;
        }
        if !seen.insert((ch, s.url.as_str())) {
            continue;
        }
        channels_with_streams.insert(ch);
        streams_out.push(Stream {
            channel_id: ch.to_string(),
            url: s.url.clone(),
            quality: s.quality.clone(),
            referrer: s.referrer.clone(),
            user_agent: s.user_agent.clone(),
        });
    }

    let mut channels_out: Vec<Channel> = raw_channels
        .iter()
        .filter(|c| channels_with_streams.contains(c.id.as_str()))
        .map(|c| Channel {
            id: c.id.clone(),
            name: c.name.clone(),
            country_code: c.country.clone().unwrap_or_default(),
            categories: c.categories.clone(),
            is_nsfw: c.is_nsfw,
            logo_url: best_logo.get(c.id.as_str()).map(|(_, u)| (*u).to_string()),
        })
        .collect();
    channels_out.sort_by_key(|c| c.name.to_lowercase());

    let countries_out = raw_countries
        .into_iter()
        .map(|c| Country {
            code: c.code,
            name: c.name,
            flag: c.flag.unwrap_or_default(),
        })
        .collect();

    Ok(Catalog {
        countries: countries_out,
        channels: channels_out,
        streams: streams_out,
    })
}

// ---- network ---------------------------------------------------------------

async fn fetch(client: &reqwest::Client, file: &str) -> Result<Vec<u8>> {
    let url = format!("{API_BASE}/{file}");
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?
        .error_for_status()
        .with_context(|| format!("bad status from {url}"))?;
    Ok(resp
        .bytes()
        .await
        .with_context(|| format!("reading {url}"))?
        .to_vec())
}

async fn run(db_path: &Path, tx: &Sender<SyncMsg>) -> Result<SyncSummary> {
    let send = |m: &str| {
        let _ = tx.send(SyncMsg::Progress(m.to_string()));
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .user_agent(concat!("iptv-org-player/", env!("CARGO_PKG_VERSION")))
        .build()?;

    send("Downloading channel data...");
    let (countries, channels, streams, logos) = tokio::try_join!(
        fetch(&client, "countries.json"),
        fetch(&client, "channels.json"),
        fetch(&client, "streams.json"),
        fetch(&client, "logos.json"),
    )?;

    send("Processing...");
    let catalog =
        tokio::task::spawn_blocking(move || join(&countries, &channels, &streams, &logos))
            .await??;

    send("Saving...");
    let path = db_path.to_path_buf();
    let summary = tokio::task::spawn_blocking(move || -> Result<SyncSummary> {
        let mut db = Db::open(&path)?;
        db.replace_catalog(&catalog)?;
        Ok(SyncSummary {
            countries: catalog.countries.len(),
            channels: catalog.channels.len(),
            streams: catalog.streams.len(),
        })
    })
    .await??;
    Ok(summary)
}

/// Run a sync on a background thread. Progress and the result are sent over
/// `tx`; `on_msg` is called after each message so the UI can repaint.
pub fn spawn(db_path: PathBuf, tx: Sender<SyncMsg>, on_msg: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let (ptx, prx) = std::sync::mpsc::channel::<SyncMsg>();
        let forward = {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for m in prx {
                    let _ = tx.send(m);
                    on_msg();
                }
            })
        };
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|rt| rt.block_on(run(&db_path, &ptx)));
        let _ = ptx.send(match result {
            Ok(s) => SyncMsg::Done(s),
            Err(e) => SyncMsg::Failed(format!("{e:#}")),
        });
        drop(ptx);
        let _ = forward.join();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_catalog() -> Catalog {
        join(
            include_bytes!("../../tests/fixtures/countries.json"),
            include_bytes!("../../tests/fixtures/channels.json"),
            include_bytes!("../../tests/fixtures/streams.json"),
            include_bytes!("../../tests/fixtures/logos.json"),
        )
        .unwrap()
    }

    #[test]
    fn drops_closed_orphan_and_streamless() {
        let c = fixture_catalog();
        let ids: Vec<_> = c.channels.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["HRT1.hr", "Spicy.af"]); // Old (closed) and NoStreams gone
    }

    #[test]
    fn dedups_and_filters_streams() {
        let c = fixture_catalog();
        let urls: Vec<_> = c
            .streams
            .iter()
            .filter(|s| s.channel_id == "HRT1.hr")
            .map(|s| s.url.as_str())
            .collect();
        assert_eq!(
            urls,
            [
                "https://example.com/hrt1/hd.m3u8",
                "http://example.com/hrt1/sd.m3u8"
            ]
        );
        assert_eq!(c.streams.len(), 3); // + Spicy
    }

    #[test]
    fn keeps_headers() {
        let c = fixture_catalog();
        let s = c
            .streams
            .iter()
            .find(|s| s.url.ends_with("sd.m3u8"))
            .unwrap();
        assert_eq!(s.referrer.as_deref(), Some("https://hrt.hr/"));
        assert_eq!(s.user_agent.as_deref(), Some("TestUA"));
    }

    #[test]
    fn flags_nsfw_and_picks_largest_used_logo() {
        let c = fixture_catalog();
        let hrt = c.channels.iter().find(|c| c.id == "HRT1.hr").unwrap();
        assert!(!hrt.is_nsfw);
        assert_eq!(
            hrt.logo_url.as_deref(),
            Some("https://example.com/large.png")
        );
        let spicy = c.channels.iter().find(|c| c.id == "Spicy.af").unwrap();
        assert!(spicy.is_nsfw);
        assert_eq!(
            spicy.logo_url.as_deref(),
            Some("https://example.com/nosize.png")
        );
    }
}

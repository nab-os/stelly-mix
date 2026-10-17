//! Stelly's server as stelly-mix's: it holds the mix analysis of every track
//! it has been asked to, and hands out signed URLs for their audio.
//!
//! `sync` mirrors its mix library into the local one, as `qobuz:<id>`
//! entries, the same that a Qobuz source would make, so what is already here
//! is not fetched twice; and as a source named `qobuz` it fetches a track's
//! audio when it is to be heard. Everything else, next, sets, renders, finds,
//! then runs here on the mirror, offline included.
//!
//! It wants `STELLY_SERVER` and `STELLY_TOKEN`, from the environment or
//! `~/.config/stelly-mix/stelly.env`.

use crate::api::{EntriesRequest, Entry, Status, TrackSummary};
use crate::library::Library;
use crate::source::{Item, Source};
use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Entries asked for at once while syncing.
const CHUNK: usize = 200;
/// Qobuz's formats: FLAC 16/44.1, and MP3 320 for analysing.
const FLAC_CD: u32 = 6;
const MP3_320: u32 = 5;

pub struct Stelly {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl Stelly {
    /// The server named by `STELLY_SERVER` and `STELLY_TOKEN`, if both are
    /// set, in the environment or in stelly-mix's config.
    pub fn from_env() -> Option<Self> {
        let files: Vec<PathBuf> = config_dir().map(|d| vec![d.join("stelly.env"), d.join("qobuz.env")]).unwrap_or_default();
        let read = |key: &str| {
            std::env::var(key).ok().filter(|v| !v.is_empty()).or_else(|| {
                files.iter().find_map(|f| {
                    std::fs::read_to_string(f).ok()?.lines().find_map(|l| {
                        let (k, v) = l.trim().strip_prefix("export ").unwrap_or(l.trim()).split_once('=')?;
                        (k.trim() == key).then(|| v.trim().trim_matches(['"', '\'']).to_string()).filter(|v| !v.is_empty())
                    })
                })
            })
        };
        let (base, token) = (read("STELLY_SERVER")?, read("STELLY_TOKEN")?);
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .build()
            .new_agent();
        Some(Self { base: base.trim_end_matches('/').to_string(), token, agent })
    }

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let mut request = self.agent.get(&format!("{}{path}", self.base)).header("Authorization", &format!("Bearer {}", self.token));
        for (key, value) in query {
            request = request.query(*key, value);
        }
        answer(path, request.call().with_context(|| format!("reaching Stelly's server at {}", self.base))?)
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> {
        let response = self
            .agent
            .post(&format!("{}{path}", self.base))
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send(serde_json::to_string(body)?)
            .with_context(|| format!("reaching Stelly's server at {}", self.base))?;
        answer(path, response)
    }

    pub fn status(&self) -> Result<Status> {
        self.get("/api/mix/status", &[])
    }

    pub fn library(&self) -> Result<Vec<TrackSummary>> {
        self.get("/api/mix/library", &[])
    }

    /// Bring every entry of the server's mix library that this one lacks
    /// into it, telling `report` how far along it is; how many came.
    pub fn sync(&self, library: &Library, report: &dyn Fn(usize, usize)) -> Result<usize> {
        let missing: Vec<i64> = self
            .library()?
            .into_iter()
            .map(|s| s.id)
            .filter(|id| library.by_origin(&format!("qobuz:{id}")).is_none())
            .collect();
        let mut done = 0;
        report(0, missing.len());
        for chunk in missing.chunks(CHUNK) {
            let entries: Vec<Entry> = self.post("/api/mix/tracks", &EntriesRequest { ids: chunk.to_vec() })?;
            for entry in entries {
                if entry.track.origin.is_none() {
                    continue;
                }
                library.put(&entry.track)?;
                done += 1;
            }
            report(done, missing.len());
        }
        Ok(done)
    }

    /// The track's audio in `format_id` into `cache`, unless some is there.
    fn download(&self, item: &Item, cache: &Path, format_id: u32) -> Result<PathBuf> {
        if let Some(file) = self.cached(item, cache) {
            return Ok(file);
        }
        std::fs::create_dir_all(cache).with_context(|| format!("creating {}", cache.display()))?;
        let url: Value = self.get(&format!("/api/tracks/{}/url", item.id), &[("format", format_id.to_string())])?;
        let url = url["url"].as_str().context("Stelly's server gave no URL")?;
        let extension = if format_id == MP3_320 { "mp3" } else { "flac" };
        let target = cache.join(format!("{}.{extension}", item.file_stem()));
        let mut response = ureq::get(url).call().with_context(|| format!("downloading {}", item.title))?;
        let mut body = response.body_mut().with_config().limit(1 << 31).reader();
        let partial = target.with_extension(format!("{}.part", std::process::id()));
        let fetched = (|| -> Result<()> {
            let mut file = std::fs::File::create(&partial)?;
            std::io::copy(&mut body, &mut file)?;
            file.flush()?;
            Ok(std::fs::rename(&partial, &target)?)
        })();
        if fetched.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        fetched.with_context(|| format!("downloading {}", item.title))?;
        Ok(target)
    }
}

/// A reply's JSON, or why there is none.
fn answer<T: DeserializeOwned>(path: &str, mut response: ureq::http::Response<ureq::Body>) -> Result<T> {
    let status = response.status();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    if status.as_u16() == 401 {
        bail!("Stelly's server refused the token in STELLY_TOKEN");
    }
    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["message"].as_str().map(str::to_string))
            .unwrap_or_else(|| body[..body.floor_char_boundary(200)].to_string());
        bail!("{path}: {status}: {message}");
    }
    serde_json::from_str(&body).with_context(|| format!("parsing {path}"))
}

fn config_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("stelly-mix"))
}

/// One of Stelly's tracks as an item.
fn item(track: &Value) -> Option<Item> {
    Some(Item {
        id: track["track_id"].as_i64()?.to_string(),
        artist: track["artist"].as_str().unwrap_or("Unknown").to_string(),
        title: track["title"].as_str()?.to_string(),
        seconds: None,
    })
}

/// Qobuz's tracks by way of Stelly: `qobuz:<id>` is a track of its space.
impl Source for Stelly {
    fn name(&self) -> &str {
        "qobuz"
    }

    fn resolve(&self, reference: &str) -> Result<Vec<Item>> {
        let id: i64 = reference.parse().with_context(|| format!("qobuz:{reference}: expected a track id"))?;
        let tracks: Vec<Value> = self.post("/api/space/tracks", &serde_json::json!({ "ids": [id] }))?;
        let items: Vec<Item> = tracks.iter().filter_map(item).collect();
        if items.is_empty() {
            bail!("qobuz:{reference}: not in Stelly's space");
        }
        Ok(items)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<Item>> {
        let found: Value = self.get("/api/space/search", &[("q", query.to_string()), ("limit", limit.to_string())])?;
        Ok(found["tracks"].as_array().into_iter().flatten().filter_map(item).take(limit).collect())
    }

    fn cached(&self, item: &Item, cache: &Path) -> Option<PathBuf> {
        let stem = item.file_stem();
        ["flac", "mp3"].iter().map(|e| cache.join(format!("{stem}.{e}"))).find(|p| p.is_file())
    }

    fn fetch(&self, item: &Item, cache: &Path) -> Result<PathBuf> {
        self.download(item, cache, FLAC_CD)
    }

    fn fetch_for_analysis(&self, item: &Item, cache: &Path) -> Result<PathBuf> {
        self.download(item, cache, MP3_320)
    }
}

/// The sources plain stelly-mix has: Stelly's server, when one is set.
pub fn sources() -> Vec<Box<dyn Source>> {
    Stelly::from_env().map(|s| Box::new(s) as Box<dyn Source>).into_iter().collect()
}

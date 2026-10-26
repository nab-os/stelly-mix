//! Stelly's server as stelly-mix's: it holds the mix analysis of every track
//! it has been asked to, and hands out signed URLs for their audio.
//!
//! `sync` mirrors its mix library into the local one, as `stelly:<id>`
//! entries, by the server's own ids, so what is already here is not fetched
//! twice; and as the `stelly` source it fetches a track's audio, from
//! whichever of the server's providers has it, when it is to be heard.
//! Everything else, next, sets, renders, finds, then runs here on the
//! mirror, offline included.
//!
//! It wants `STELLY_SERVER` and `STELLY_TOKEN`, from the environment or
//! `~/.config/stelly-mix/stelly.env`.

use crate::api::{EntriesRequest, Entry, JobStatus, Selection, Stats, Status, TrackSummary};
use crate::library::Library;
use crate::source::{Item, Source};
use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use stelly_provider::Quality;

/// Entries asked for at once while syncing.
const CHUNK: usize = 200;
/// The largest reply read, bytes: a chunk of entries, at worst.
const REPLY_LIMIT: u64 = 1 << 30;
/// What Stelly's tracks are called here: `stelly:<id>`, by the server's own
/// ids, whichever of its providers has the audio.
pub const SOURCE: &str = "stelly";
/// Extensions a fetched track may have been kept under.
const KEPT: [&str; 8] = ["flac", "mp3", "ogg", "opus", "m4a", "wav", "aiff", "audio"];
/// What entries mirrored before 0.4.0 are filed under: whichever of Stelly's
/// providers had the audio, though their ids were already the server's own.
const LEGACY: [&str; 2] = ["qobuz", "ytmusic"];

pub struct Stelly {
    base: String,
    token: String,
    agent: ureq::Agent,
}

impl Stelly {
    /// A server at `base` with `token`, whatever the environment says.
    pub fn new(base: &str, token: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .build()
            .new_agent();
        Self { base: base.trim().trim_end_matches('/').to_string(), token: token.trim().to_string(), agent }
    }

    /// Where it is pointed, for a window that shows it.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The server named by `STELLY_SERVER` and `STELLY_TOKEN`, if both are
    /// set, in the environment or in stelly-mix's config.
    pub fn from_env() -> Option<Self> {
        let files: Vec<PathBuf> = config_dir().map(|d| vec![d.join("stelly.env")]).unwrap_or_default();
        let read = |key: &str| {
            std::env::var(key)
                .ok()
                .filter(|v| !v.is_empty())
                .or_else(|| files.iter().find_map(|file| from_file(file, key)))
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
    /// into it, and those the server has found drops in that this one has
    /// none of, or older ones, telling `report` how far along it is; how
    /// many came.
    pub fn sync(&self, library: &Library, report: &dyn Fn(usize, usize)) -> Result<usize> {
        // Entries of another analysis are read by neither side: mirroring
        // them would store what `by_origin` then drops as stale, and every
        // sync would fetch the whole library again saying nothing.
        let version = self.status()?.version;
        if version != crate::library::VERSION {
            let behind = if version > crate::library::VERSION { "stelly-mix" } else { "Stelly's server" };
            bail!(
                "the server's mix analysis is version {version}, this stelly-mix reads {}: upgrade {behind} so the two agree",
                crate::library::VERSION
            );
        }
        library.adopt_sources(&LEGACY, SOURCE)?;
        let missing: Vec<i64> = self
            .library()?
            .into_iter()
            .filter(|summary| match library.by_origin(&format!("{SOURCE}:{}", summary.id)) {
                None => true,
                Some(here) => summary.drops == Some(crate::drops::VERSION) && !here.analysis.drops.is_some_and(|d| d.current()),
            })
            .map(|summary| summary.id)
            .collect();
        self.bring(library, &missing, report)
    }

    /// The server's entries for `ids` into the library, as `sync` brings
    /// them; those it has none for are passed over. How many came.
    pub fn bring(&self, library: &Library, ids: &[i64], report: &dyn Fn(usize, usize)) -> Result<usize> {
        let mut done = 0;
        report(0, ids.len());
        for chunk in ids.chunks(CHUNK) {
            let entries: Vec<Entry> = self.post("/api/mix/tracks", &EntriesRequest { ids: chunk.to_vec() })?;
            for mut entry in entries {
                if !adopt(&mut entry) {
                    continue;
                }
                // What was heard here stays: the server does not listen for
                // voices, and a track brought again for its drops has the
                // same phrases the voices were heard in.
                if entry.track.vocals.is_none() {
                    let here = library.by_origin(&format!("{SOURCE}:{}", entry.id));
                    entry.track.vocals = here.and_then(|here| here.vocals.filter(|_| here.segments.len() == entry.track.segments.len()));
                }
                library.put(&entry.track)?;
                done += 1;
            }
            report(done, ids.len());
        }
        Ok(done)
    }

    /// Have the server analyse track `id` for mixing now, ahead of what it
    /// has queued, and wait for it: a download and seconds of analysis,
    /// longer when its workers are busy. None from a server from before it
    /// could, or that has no such track: the caller analyses it here then.
    pub fn analyse_now(&self, id: &str) -> Result<Option<crate::library::Track>> {
        let path = format!("/api/mix/tracks/{id}/analyse");
        let response = self
            .agent
            .post(&format!("{}{path}", self.base))
            .header("Authorization", &format!("Bearer {}", self.token))
            .config()
            // The server waits up to ten minutes before saying it is still queued.
            .timeout_global(Some(Duration::from_secs(11 * 60)))
            .build()
            .send_empty()
            .with_context(|| format!("reaching Stelly's server at {}", self.base))?;
        if response.status().as_u16() == 404 {
            return Ok(None);
        }
        answer(&path, response).map(Some)
    }

    /// What the family liked on Stelly after `since`, Unix seconds, newest
    /// first: liked tracks and the tracks of liked albums. The server queues
    /// each for its mix analysis as it is liked, so most are analysed by the
    /// time anyone sits down to mix them.
    pub fn liked(&self, since: i64) -> Result<Vec<Liked>> {
        self.get("/api/mix/liked", &[("since", since.to_string())])
    }

    /// Have the server analyse `selection` for mixing, in the background.
    /// Wants a device with `admin` scope: it is hours of downloading and CPU.
    pub fn analyse(&self, selection: &Selection) -> Result<JobStatus> {
        self.post("/api/mix/analyse", selection).map_err(|err| {
            if format!("{err:#}").contains("403") {
                err.context("asking a server to analyse wants a device with admin scope: stelly-server pair --scope admin")
            } else {
                err
            }
        })
    }

    pub fn analysis(&self) -> Result<JobStatus> {
        self.get("/api/mix/analyse", &[])
    }

    /// The cover art of each of `ids`, where the server's catalogue has one.
    ///
    /// The mirrored library holds none: an entry is the analysis, and
    /// artwork is the catalogue's. So it is asked for when it is about to be
    /// shown, and whoever asks is expected to remember the answer.
    pub fn covers(&self, ids: &[i64]) -> Result<Vec<(i64, String)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let tracks: Vec<Value> = self.post("/api/space/tracks", &serde_json::json!({ "ids": ids }))?;
        Ok(tracks
            .iter()
            .filter_map(|track| {
                let id = track["track_id"].as_i64()?;
                let image = track["image"].as_str().filter(|url| !url.is_empty())?;
                Some((id, image.to_string()))
            })
            .collect())
    }

    /// Stop the analysis the server is running, as it stands when it hears.
    /// Wants `admin`, as starting one does.
    pub fn stop(&self) -> Result<JobStatus> {
        self.post("/api/mix/analyse/stop", &Value::Null)
    }

    /// `analyse`, then follow it to the end, telling `report` each new line.
    pub fn analyse_and_wait(&self, selection: &Selection, report: &dyn Fn(&str)) -> Result<Stats> {
        self.analyse(selection)?;
        let mut last = None;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let status = self.analysis()?;
            if status.last != last {
                if let Some(line) = &status.last {
                    report(line);
                }
                last = status.last.clone();
            }
            if !status.running {
                return status.stats.with_context(|| status.last.unwrap_or_else(|| "the analysis stopped".into()));
            }
        }
    }

    /// The track's audio in up to `quality` into `cache`, unless some is
    /// there. The server answers with the best it has up to that, which for
    /// a track with no lossless master is less.
    fn download(&self, item: &Item, cache: &Path, quality: Quality) -> Result<PathBuf> {
        if let Some(file) = self.cached(item, cache) {
            return Ok(file);
        }
        std::fs::create_dir_all(cache).with_context(|| format!("creating {}", cache.display()))?;
        let url: Value = self.get(&format!("/api/tracks/{}/url", item.id), &[("quality", quality.as_str().to_string())])?;
        let url = url["url"].as_str().context("Stelly's server gave no URL")?;
        // Bounded, so a transfer that stalls fails rather than holding up
        // the analysis or the render that wanted it for good: the answer
        // must start in half a minute, and the file come in twenty, which is
        // a hi-res album track on a slow line.
        let mut response = self
            .agent
            .get(url)
            .config()
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(20 * 60)))
            .build()
            .call()
            .with_context(|| format!("downloading {}", item.title))?;
        // This agent hands back errors as answers; one is not audio.
        if !response.status().is_success() {
            bail!("downloading {}: the file's host answered {}", item.title, response.status());
        }
        // What came, which is not always what was asked for.
        let mime = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or_default().to_ascii_lowercase();
        let extension = match () {
            _ if mime.contains("mpeg") || mime.contains("mp3") => "mp3",
            _ if mime.contains("flac") => "flac",
            _ if mime.contains("opus") => "opus",
            _ if mime.contains("ogg") => "ogg",
            _ if mime.contains("mp4") || mime.contains("m4a") || mime.contains("aac") => "m4a",
            _ if mime.contains("wav") => "wav",
            _ if mime.contains("aiff") => "aiff",
            _ if quality == Quality::Lossy => "mp3",
            _ => "flac",
        };
        let target = cache.join(format!("{}.{extension}", item.file_stem()));
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
    // Past ureq's own 10MB: two hundred long tracks' entries come to more.
    let read = response.body_mut().with_config().limit(REPLY_LIMIT).read_to_string();
    let body = match read {
        Ok(body) => body,
        Err(err) if status.is_success() => return Err(err).with_context(|| format!("reading {path}")),
        // A failure's own text is only a nicety.
        Err(_) => String::new(),
    };
    if status.as_u16() == 401 {
        bail!("Stelly's server refused the token in STELLY_TOKEN");
    }
    // Since 2.0 a server whose database is busy says so rather than calling
    // the token revoked; it is worth trying again, which 401 is not.
    if status.as_u16() == 503 {
        bail!("Stelly's server could not answer just now, try again: {}", body.trim());
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

/// One setting out of a `KEY=value` file, as `from_env` reads them:
/// `export ` is allowed in front, and quotes around the value come off.
fn from_file(file: &Path, key: &str) -> Option<String> {
    std::fs::read_to_string(file).ok()?.lines().find_map(|line| {
        let (k, v) = line.trim().strip_prefix("export ").unwrap_or(line.trim()).split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches(['"', '\'']).to_string()).filter(|v| !v.is_empty())
    })
}

/// Where the server and token are kept when they are not in the
/// environment: `~/.config/stelly-mix/stelly.env`.
pub fn config_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("stelly.env"))
}

/// Whether the environment is naming a server, in which case what is
/// written to the config file will not be what the next run uses.
pub fn env_overrides() -> bool {
    ["STELLY_SERVER", "STELLY_TOKEN"].iter().any(|key| std::env::var(key).is_ok_and(|value| !value.is_empty()))
}

/// Keep `base` and `token` for every later run. The file is written whole,
/// so anything else in it is not preserved: it holds these two settings.
pub fn save(base: &str, token: &str) -> Result<PathBuf> {
    let file = config_file().context("there is no home directory to keep settings in")?;
    save_to(&file, base, token)?;
    Ok(file)
}

fn save_to(file: &Path, base: &str, token: &str) -> Result<()> {
    let dir = file.parent().context("a config file has a directory")?;
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    let body = format!(
        "# Written by stelly-mix. The environment wins over this file.\nSTELLY_SERVER={}\nSTELLY_TOKEN={}\n",
        base.trim().trim_end_matches('/'),
        token.trim()
    );
    let partial = file.with_extension(format!("{}.part", std::process::id()));
    std::fs::write(&partial, body).with_context(|| format!("writing {}", partial.display()))?;
    std::fs::rename(&partial, file).with_context(|| format!("moving {} into place", file.display()))?;
    Ok(())
}

fn config_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(config.join("stelly-mix"))
}

/// A track liked on Stelly, as `/api/mix/liked` lists it.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Liked {
    pub track_id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    /// Unix seconds, the latest when several liked it.
    pub liked_at: i64,
    /// Who liked it, by name.
    #[serde(default)]
    pub by: Vec<String>,
    /// Whether the server has its mix analysis yet.
    #[serde(default)]
    pub analysed: bool,
}

impl Liked {
    /// Its key in the library.
    pub fn key(&self) -> String {
        format!("{SOURCE}:{}", self.track_id)
    }
}

/// File an entry as the server's own track, under the id it was asked for:
/// one analysed before 2.0 comes back naming whichever provider had the
/// audio, though its id was already the server's. Whether it can be filed at
/// all: an entry with no origin is no source's.
fn adopt(entry: &mut Entry) -> bool {
    let Some(origin) = entry.track.origin.as_mut() else { return false };
    origin.source = SOURCE.to_string();
    origin.item.id = entry.id.to_string();
    true
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

/// The server's tracks: `stelly:<id>` is a track of its space.
impl Source for Stelly {
    fn name(&self) -> &str {
        SOURCE
    }

    fn resolve(&self, reference: &str) -> Result<Vec<Item>> {
        let id: i64 = reference.parse().with_context(|| format!("{SOURCE}:{reference}: expected a track id"))?;
        let tracks: Vec<Value> = self.post("/api/space/tracks", &serde_json::json!({ "ids": [id] }))?;
        let items: Vec<Item> = tracks.iter().filter_map(item).collect();
        if items.is_empty() {
            bail!("{SOURCE}:{reference}: not in Stelly's space");
        }
        Ok(items)
    }

    fn analysed(&self, item: &Item) -> Result<Option<crate::library::Track>> {
        self.analyse_now(&item.id)
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<Item>> {
        let found: Value = self.get("/api/space/search", &[("q", query.to_string()), ("limit", limit.to_string())])?;
        Ok(found["tracks"].as_array().into_iter().flatten().filter_map(item).take(limit).collect())
    }

    fn cached(&self, item: &Item, cache: &Path) -> Option<PathBuf> {
        let stem = item.file_stem();
        KEPT.iter().map(|e| cache.join(format!("{stem}.{e}"))).find(|p| p.is_file())
    }

    fn fetch(&self, item: &Item, cache: &Path) -> Result<PathBuf> {
        self.download(item, cache, Quality::Lossless)
    }

    fn fetch_for_analysis(&self, item: &Item, cache: &Path) -> Result<PathBuf> {
        self.download(item, cache, Quality::Lossy)
    }
}

/// The sources plain stelly-mix has: Stelly's server, when one is set.
pub fn sources() -> Vec<Box<dyn Source>> {
    Stelly::from_env().map(|s| Box::new(s) as Box<dyn Source>).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// A server that answers each of `replies` in turn, one connection each,
    /// whatever was asked; `{base}` in a reply is its own address.
    fn serve(replies: Vec<(u16, &'static str)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let own = base.clone();
        std::thread::spawn(move || {
            for (status, body) in replies {
                let Ok((stream, _)) = listener.accept() else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                    line.clear();
                }
                let body = body.replace("{base}", &own);
                let mut stream = stream;
                let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        base
    }

    #[test]
    fn a_file_host_s_error_is_no_audio() {
        let base = serve(vec![(200, r#"{"url": "{base}/file"}"#), (404, "gone")]);
        let cache = std::env::temp_dir().join(format!("stelly-mix-download-{}", std::process::id()));
        let item = Item { id: "7".into(), artist: "A".into(), title: "T".into(), seconds: None };
        let error = Stelly::new(&base, "token").download(&item, &cache, Quality::Lossy).unwrap_err();
        assert!(format!("{error:#}").contains("404"), "{error:#}");
        let left: Vec<_> = std::fs::read_dir(&cache).map(|d| d.flatten().collect()).unwrap_or_default();
        assert!(left.is_empty(), "nothing kept as audio: {left:?}");
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn what_is_saved_is_what_a_later_run_reads() {
        let dir = std::env::temp_dir().join(format!("stelly-mix-env-{}", std::process::id()));
        let file = dir.join("stelly.env");
        save_to(&file, "https://music.example.org/", "  abc123  ").unwrap();

        assert_eq!(from_file(&file, "STELLY_SERVER").as_deref(), Some("https://music.example.org"), "the trailing slash comes off");
        assert_eq!(from_file(&file, "STELLY_TOKEN").as_deref(), Some("abc123"), "and the padding with it");
        assert_eq!(from_file(&file, "STELLY_NOTHING"), None);

        // Written whole each time, so the second save is not the first plus
        // the second.
        save_to(&file, "http://nas:7700", "zzz").unwrap();
        assert_eq!(from_file(&file, "STELLY_SERVER").as_deref(), Some("http://nas:7700"));
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().filter(|l| l.starts_with("STELLY_SERVER")).count(), 1);

        // The forms a hand-written file might use are still read.
        std::fs::write(&file, "export STELLY_TOKEN=\"quoted\"\n# a comment\nSTELLY_SERVER='single'\n").unwrap();
        assert_eq!(from_file(&file, "STELLY_TOKEN").as_deref(), Some("quoted"));
        assert_eq!(from_file(&file, "STELLY_SERVER").as_deref(), Some("single"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_like_reads_as_the_server_lists_it() {
        let listed = r#"[{"track_id":16629,"title":"Weekend Wars","artist":"MGMT","liked_at":1792868215,"by":["owner"],"analysed":true},
                         {"track_id":7,"liked_at":5}]"#;
        let liked: Vec<Liked> = serde_json::from_str(listed).unwrap();
        assert_eq!((liked[0].key().as_str(), liked[0].title.as_str(), liked[0].analysed), ("stelly:16629", "Weekend Wars", true));
        // A server that says less still reads.
        assert_eq!((liked[1].by.len(), liked[1].analysed), (0, false));
    }

    /// An entry as the server hands it over, from `source`.
    fn entry(id: i64, source: &str, track_id: &str) -> Entry {
        let track = serde_json::json!({
            "path": format!("MGMT - Time To Pretend [{track_id}]"),
            "size": 0, "modified": 0, "version": crate::library::VERSION,
            "origin": { "source": source, "item": { "id": track_id, "artist": "MGMT", "title": "Time To Pretend" } },
            "analysis": {
                "seconds": 261.0, "grid": null, "key": null, "loudness": null,
                "beats": [], "downbeats": [], "downbeat_confidence": 0.0, "phrases": [], "sections": []
            },
            "segments": []
        });
        Entry { id, track: serde_json::from_value(track).expect("a track") }
    }

    #[test]
    fn an_entry_from_before_2_0_is_filed_under_stelly() {
        // What ~/stelly-prod-data holds: the provider's name, the server's id.
        let mut old = entry(16522, "qobuz", "16522");
        assert!(adopt(&mut old));
        assert_eq!(old.track.origin.unwrap().key(), "stelly:16522");
    }

    #[test]
    fn adopting_an_entry_the_server_wrote_leaves_it_alone() {
        let mut now = entry(16522, SOURCE, "16522");
        let before = now.track.origin.clone();
        assert!(adopt(&mut now));
        assert_eq!(now.track.origin, before);
    }

    /// The id asked for wins: it is what the library looks the entry up by.
    #[test]
    fn an_entry_filed_under_a_provider_id_takes_the_one_it_came_for() {
        let mut other = entry(16522, "qobuz", "99887766");
        assert!(adopt(&mut other));
        assert_eq!(other.track.origin.unwrap().key(), "stelly:16522");
    }

    #[test]
    fn an_entry_with_no_origin_is_not_filed() {
        let mut none = entry(16522, "qobuz", "16522");
        none.track.origin = None;
        assert!(!adopt(&mut none));
    }
}

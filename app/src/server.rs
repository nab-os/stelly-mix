//! Audio to the webview: tracks and renders served over HTTP on 127.0.0.1,
//! with ranges, so its `<audio>` element can seek in them.
//!
//! A custom URL scheme would do for small things, but the webview's media
//! player fetches through its own loader and is happiest with plain HTTP.
//! Only files the app registers are served, under a path that holds a token
//! made at start, so nothing else on the machine reads files through it.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

struct Server {
    port: u16,
    token: String,
    files: Mutex<HashMap<String, PathBuf>>,
}

static SERVER: OnceLock<Server> = OnceLock::new();

/// Start serving; once, at launch.
pub fn start() -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let _ = SERVER.set(Server {
        port,
        token: token(),
        files: Mutex::new(HashMap::new()),
    });
    std::thread::Builder::new().name("audio-server".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let _ = answer(stream);
            });
        }
    })?;
    Ok(())
}

/// A URL the webview can play `path` from.
pub fn url(path: &Path) -> String {
    let server = SERVER.get().expect("the audio server starts with the app");
    let id = format!("{:016x}", hash(path.as_os_str().as_encoded_bytes()));
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("bin");
    server.files.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), path.to_path_buf());
    format!("http://127.0.0.1:{}/{}/{id}.{extension}", server.port, server.token)
}

/// 128 bits from the standard library's own random keys.
fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let half = || {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
        hasher.finish()
    };
    format!("{:016x}{:016x}", half(), half())
}

fn hash(bytes: &[u8]) -> u64 {
    // FNV-1a: stable across runs, which a URL wants.
    bytes.iter().fold(0xcbf29ce484222325, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

fn answer(stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut range = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_string());
            }
        }
    }
    let mut out = stream;
    let Some(path) = lookup(target) else {
        return write!(out, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let Ok(mut file) = std::fs::File::open(&path) else {
        return write!(out, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let size = file.metadata()?.len();
    let kind = content_type(&path);
    let (status, from, to) = match range.as_deref().map(|r| parse_range(r, size)) {
        None => ("200 OK", 0, size.saturating_sub(1)),
        Some(Some((from, to))) => ("206 Partial Content", from, to),
        Some(None) => {
            return write!(
                out,
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{size}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        }
    };
    let length = if size == 0 { 0 } else { to - from + 1 };
    write!(
        out,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nAccept-Ranges: bytes\r\nContent-Length: {length}\r\n"
    )?;
    if status.starts_with("206") {
        write!(out, "Content-Range: bytes {from}-{to}/{size}\r\n")?;
    }
    write!(out, "Connection: close\r\n\r\n")?;
    if method == "HEAD" {
        return Ok(());
    }
    file.seek(SeekFrom::Start(from))?;
    std::io::copy(&mut file.take(length), &mut out)?;
    out.flush()
}

/// The registered file a request's target names, if its token is right.
fn lookup(target: &str) -> Option<PathBuf> {
    let server = SERVER.get()?;
    let rest = target.strip_prefix('/')?.strip_prefix(server.token.as_str())?.strip_prefix('/')?;
    let id = rest.split('.').next()?;
    server.files.lock().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` as an inclusive range within `size`.
fn parse_range(header: &str, size: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (from, to) = spec.split_once('-')?;
    let last = size.checked_sub(1)?;
    let range = match (from.trim(), to.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (size.saturating_sub(n), last)
        }
        (a, "") => (a.parse().ok()?, last),
        (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.min(last)),
    };
    (range.0 <= range.1 && range.0 <= last).then_some(range)
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        Some("ogg") | Some("opus") => "audio/ogg",
        Some("m4a") => "audio/mp4",
        Some("aif") | Some("aiff") => "audio/aiff",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_as_browsers_ask_for_them() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-200", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-2", 100), None);
        assert_eq!(parse_range("lines=0-1", 100), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
    }

    #[test]
    fn a_registered_file_is_served_whole_and_in_part_and_nothing_else_is() {
        start().unwrap();
        let dir = std::env::temp_dir().join(format!("stelly-mix-app-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.wav");
        std::fs::write(&path, b"0123456789").unwrap();
        let url = url(&path);
        let get = |target: &str, range: Option<&str>| {
            let address = url.trim_start_matches("http://").split('/').next().unwrap().to_string();
            let mut stream = TcpStream::connect(address).unwrap();
            let range = range.map_or(String::new(), |r| format!("Range: {r}\r\n"));
            write!(stream, "GET {target} HTTP/1.1\r\nHost: x\r\n{range}\r\n").unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).unwrap();
            reply
        };
        let target = &url[url.find('/').unwrap() + 2..];
        let target = &target[target.find('/').unwrap()..];
        let whole = get(target, None);
        assert!(whole.starts_with("HTTP/1.1 200") && whole.ends_with("0123456789"), "{whole}");
        assert!(whole.contains("Content-Type: audio/wav"));
        let part = get(target, Some("bytes=2-4"));
        assert!(part.starts_with("HTTP/1.1 206") && part.ends_with("234") && part.contains("bytes 2-4/10"), "{part}");
        // A wrong token, or a file never registered, is nothing.
        let forged = target.replacen(&SERVER.get().unwrap().token, "0000", 1);
        assert!(get(&forged, None).starts_with("HTTP/1.1 404"));
        assert!(get("/etc/passwd", None).starts_with("HTTP/1.1 404"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

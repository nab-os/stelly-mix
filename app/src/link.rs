//! `stelly-mix://track/<id>`: Stelly's "Open in stelly-mix", one of its
//! tracks by the server's own id, shown here as `stelly:<id>`.
//!
//! The link comes in as the first argument. A window already open takes it
//! over a socket on 127.0.0.1, so a second launch hands it on and leaves
//! rather than opening a second window; the port and a token made at start
//! are in a file only this user reads, so nothing else on the machine has a
//! window open tracks through it.
//!
//! The window says it handles the scheme each time it starts, to the desktop
//! on Linux and to the registry on Windows, so whichever build was run last
//! is the one a link opens.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub const SCHEME: &str = "stelly-mix";

/// What a link asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Link {
    /// One of Stelly's tracks, by the server's id.
    Track(i64),
}

impl Link {
    pub fn parse(text: &str) -> Option<Self> {
        let rest = text.trim().strip_prefix(SCHEME)?.strip_prefix("://")?;
        let mut parts = rest.trim_end_matches('/').split('/');
        let link = match (parts.next()?, parts.next()?) {
            ("track", id) => Link::Track(id.parse().ok()?),
            _ => return None,
        };
        parts.next().is_none().then_some(link)
    }

    fn text(&self) -> String {
        match self {
            Link::Track(id) => format!("{SCHEME}://track/{id}"),
        }
    }
}

/// The link this launch was given, if any.
pub fn from_args() -> Option<Link> {
    std::env::args().skip(1).find_map(|arg| Link::parse(&arg))
}

/// Links for the window, from this launch and every later one.
static LINKS: Mutex<Option<UnboundedReceiver<Link>>> = Mutex::new(None);

/// What the window reads links from; once.
pub fn take() -> Option<UnboundedReceiver<Link>> {
    LINKS.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Give `link` to a window already open; whether one took it.
pub fn hand_over(link: &Link) -> bool {
    let Some((port, token)) = std::fs::read_to_string(window_file())
        .ok()
        .and_then(|text| text.split_once(' ').and_then(|(port, token)| Some((port.parse::<u16>().ok()?, token.trim().to_string()))))
    else {
        return false;
    };
    let ask = || -> std::io::Result<bool> {
        let mut stream = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(1))?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        writeln!(stream, "{token} {}", link.text())?;
        let mut answer = String::new();
        BufReader::new(stream).read_line(&mut answer)?;
        Ok(answer.trim() == "ok")
    };
    ask().unwrap_or(false)
}

/// Listen for links from later launches, `first` from this one ahead of
/// them. Without a socket the window still opens, it just opens again for
/// the next link.
pub fn listen(first: Option<Link>) {
    let (tell, heard) = unbounded_channel();
    *LINKS.lock().unwrap_or_else(|e| e.into_inner()) = Some(heard);
    if let Some(link) = first {
        let _ = tell.send(link);
    }
    if let Err(err) = serve(tell) {
        eprintln!("could not listen for links from Stelly: {err}");
    }
}

fn serve(tell: UnboundedSender<Link>) -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let token = crate::server::token();
    write_private(&window_file(), &format!("{port} {token}"))?;
    std::thread::Builder::new().name("links".into()).spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let link = line.trim().split_once(' ').filter(|(given, _)| *given == token).and_then(|(_, link)| Link::parse(link));
            let taken = link.is_some_and(|link| tell.send(link).is_ok());
            let _ = writeln!(stream, "{}", if taken { "ok" } else { "no" });
        }
    })?;
    Ok(())
}

/// Where the open window says how to reach it.
fn window_file() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("stelly-mix.window"),
        None => stelly_mix::source::cache_dir("window"),
    }
}

fn write_private(file: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(file)?.write_all(text.as_bytes())
}

/// Say this build opens `stelly-mix://` links: unless a package already
/// does, on Linux; and always on Windows, per user, where the installer
/// does not.
pub fn register() {
    #[cfg(target_os = "linux")]
    if let Err(err) = register_linux() {
        eprintln!("could not register for {SCHEME}:// links: {err}");
    }
    #[cfg(windows)]
    register_windows();
}

/// The `.desktop` entry this writes, beside the package's own.
#[cfg(target_os = "linux")]
const ENTRY: &str = "stelly-mix-link.desktop";

#[cfg(target_os = "linux")]
fn register_linux() -> std::io::Result<()> {
    use std::process::Command;
    let mime = format!("x-scheme-handler/{SCHEME}");
    let current = Command::new("xdg-mime").args(["query", "default", &mime]).output()?;
    let current = String::from_utf8_lossy(&current.stdout).trim().to_string();
    // A package's entry stays; one of ours is written again, for this build.
    if !current.is_empty() && current != ENTRY {
        return Ok(());
    }
    // An AppImage's own path is where it was mounted, gone once it closes.
    let exe = match std::env::var_os("APPIMAGE") {
        Some(image) => PathBuf::from(image),
        None => std::env::current_exe()?,
    };
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| std::io::Error::other("no HOME"))?;
    let dir = data.join("applications");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join(ENTRY),
        format!(
            "[Desktop Entry]\nType=Application\nName=stelly-mix\nExec={} %u\nMimeType={mime};\nNoDisplay=true\nTerminal=false\n",
            desktop_quote(&exe.to_string_lossy())
        ),
    )?;
    if current.is_empty() {
        Command::new("xdg-mime").args(["default", ENTRY, &mime]).status()?;
    }
    Ok(())
}

/// An `Exec` argument, quoted as the desktop entry spec has it.
#[cfg(target_os = "linux")]
fn desktop_quote(text: &str) -> String {
    let mut quoted = String::from("\"");
    for c in text.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

#[cfg(windows)]
fn register_windows() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(exe) = std::env::current_exe() else { return };
    let key = format!(r"HKCU\Software\Classes\{SCHEME}");
    let command = format!("\"{}\" \"%1\"", exe.display());
    let entries: [(String, &str, String); 3] = [
        (key.clone(), "/ve", format!("URL:{SCHEME}")),
        (key.clone(), "URL Protocol", String::new()),
        (format!(r"{key}\shell\open\command"), "/ve", command),
    ];
    for (key, value, data) in entries {
        let mut reg = std::process::Command::new("reg");
        reg.args(["add", &key]);
        if value == "/ve" {
            reg.arg("/ve");
        } else {
            reg.args(["/v", value]);
        }
        let _ = reg.args(["/d", &data, "/f"]).creation_flags(CREATE_NO_WINDOW).output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_link_parses_and_writes_back() {
        assert_eq!(Link::parse("stelly-mix://track/16522"), Some(Link::Track(16522)));
        assert_eq!(Link::parse("stelly-mix://track/16522/"), Some(Link::Track(16522)));
        assert_eq!(Link::parse(&Link::Track(7).text()), Some(Link::Track(7)));
    }

    #[test]
    fn anything_else_is_not_a_link() {
        for text in ["", "stelly-mix://", "stelly-mix://track/", "stelly-mix://track/x", "stelly-mix://album/3", "stelly-mix://track/1/2", "other://track/1", "/home/me/a.flac"] {
            assert_eq!(Link::parse(text), None, "{text}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_exec_path_is_quoted() {
        assert_eq!(desktop_quote("/opt/my app/stelly-mix"), "\"/opt/my app/stelly-mix\"");
        assert_eq!(desktop_quote("/a$b\"c"), "\"/a\\$b\\\"c\"");
    }
}

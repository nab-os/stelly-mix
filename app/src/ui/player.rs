//! The player along the bottom, and the channel to player.js, which owns the
//! `<audio>` element: Rust says what to play, the script says where it is.

use super::State;
use dioxus::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;

/// What the player is doing, as player.js last said.
#[derive(Clone, Copy)]
pub struct Player {
    pub url: Signal<String>,
    pub position: Signal<f64>,
    pub duration: Signal<f64>,
    pub playing: Signal<bool>,
}

impl Player {
    pub fn new() -> Self {
        Self {
            url: Signal::new(String::new()),
            position: Signal::new(0.0),
            duration: Signal::new(0.0),
            playing: Signal::new(false),
        }
    }
}

/// What each URL the player may play is called, for the bar to say.
static TITLES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub fn title(url: &str, title: &str) {
    TITLES.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(url.to_string(), title.to_string());
}

fn title_of(url: &str) -> Option<String> {
    TITLES.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(url).cloned()
}

/// Play `url` from `start` seconds.
pub fn play(url: &str, start: f64) {
    let url = serde_json::to_string(url).unwrap_or_default();
    document::eval(&format!("window.mixPlay && window.mixPlay({url}, {start});"));
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Message {
    Time { url: String, position: f64, duration: f64, playing: bool },
    Error { url: String, message: String },
}

/// Install player.js and fold what it says into the player.
pub fn use_transport(state: State) {
    use_future(move || async move {
        let mut player = state.player;
        let mut handle = document::eval(include_str!("../../assets/player.js"));
        while let Ok(message) = handle.recv::<Message>().await {
            match message {
                Message::Time { url, position, duration, playing } => {
                    if *player.url.peek() != url {
                        player.url.set(url);
                    }
                    player.position.set(position);
                    player.duration.set(duration);
                    if *player.playing.peek() != playing {
                        player.playing.set(playing);
                    }
                }
                Message::Error { url, message } => {
                    let name = title_of(&url).unwrap_or(url);
                    state.fail(anyhow::anyhow!("could not play {name}: {message}"));
                }
            }
        }
    });
}

#[component]
pub fn PlayerBar() -> Element {
    let state = use_context::<State>();
    let player = state.player;
    let url = (player.url)();
    let (position, duration) = ((player.position)(), (player.duration)());
    let name = if url.is_empty() { None } else { Some(title_of(&url).unwrap_or_else(|| "something".into())) };
    rsx! {
        div { class: "player",
            audio { id: "player", preload: "auto" }
            button {
                class: "play",
                disabled: url.is_empty(),
                title: "Play or pause (space)",
                onclick: move |_| {
                    document::eval("window.mixToggle && window.mixToggle();");
                },
                if (player.playing)() { "❚❚" } else { "▶" }
            }
            div { class: "ellipsis",
                match name {
                    Some(name) => rsx! { span { "{name}" } },
                    None => rsx! { span { class: "muted", "Nothing playing. Click a waveform to play from there." } },
                }
            }
            input {
                class: "seek",
                r#type: "range",
                min: "0",
                max: "{duration.max(0.001)}",
                step: "0.1",
                value: "{position}",
                disabled: url.is_empty(),
                oninput: move |event| {
                    if let Ok(seconds) = event.value().parse::<f64>() {
                        document::eval(&format!("window.mixSeek && window.mixSeek({seconds});"));
                    }
                },
            }
            span { class: "time", "{crate::work::clock(position)} / {crate::work::clock(duration)}" }
        }
    }
}

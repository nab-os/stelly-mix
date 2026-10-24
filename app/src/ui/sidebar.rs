//! The library down the left: indexing more into it, filtering and sorting
//! it, and what is under way.

use super::State;
use crate::work;
use dioxus::prelude::*;
use stelly_mix::indexing::{self, Event};

/// Rows drawn at once; a filter narrows past it.
const SHOWN: usize = 400;

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Name,
    Tempo,
    Key,
}

#[component]
pub fn Sidebar() -> Element {
    let state = use_context::<State>();
    let mut filter = use_signal(String::new);
    let mut sort = use_signal(|| Sort::Name);
    let mut adding = use_signal(String::new);

    let library = (state.library)();
    let words: Vec<String> = filter().to_lowercase().split_whitespace().map(str::to_string).collect();
    let rows: Vec<(String, String, String, bool, std::path::PathBuf)> = match &library {
        None => Vec::new(),
        Some(lib) => {
            let mut tracks: Vec<&stelly_mix::library::Track> = lib
                .0
                .tracks
                .iter()
                .filter(|t| {
                    let name = t.name().to_lowercase();
                    words.iter().all(|w| name.contains(w.as_str()))
                })
                .collect();
            match sort() {
                Sort::Name => tracks.sort_by_key(|t| t.name().to_lowercase()),
                Sort::Tempo => tracks.sort_by(|a, b| a.bpm().unwrap_or(0.0).total_cmp(&b.bpm().unwrap_or(0.0))),
                Sort::Key => tracks.sort_by_key(|t| t.analysis.key.as_ref().map(|k| camelot_order(&k.camelot))),
            }
            tracks
                .into_iter()
                .map(|t| {
                    let bpm = t.bpm().map_or("–".into(), |b| format!("{b:.1}"));
                    let key = t.analysis.key.as_ref().map_or("–".into(), |k| k.camelot.clone());
                    let sings = t.vocals.as_ref().is_some_and(|heard| heard.speaks());
                    (t.name(), bpm, key, sings, t.path.clone())
                })
                .collect()
        }
    };
    let total = rows.len();
    let selected = (state.selected)();

    let mut index = move || {
        let argument = adding().trim().to_string();
        if argument.is_empty() || (state.busy)().is_some() {
            return;
        }
        adding.set(String::new());
        start_indexing(state, argument);
    };

    rsx! {
        div { class: "sidebar",
            header {
                div { class: "brand", "stelly", span { "-mix" } }
                div { class: "row-flex",
                    input {
                        class: "spacer",
                        placeholder: "Folder, file, or stelly:album/<id>, mood/<words>…",
                        value: "{adding}",
                        oninput: move |e| adding.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                index();
                            }
                        },
                    }
                    button {
                        disabled: adding().trim().is_empty() || (state.busy)().is_some(),
                        onclick: move |_| index(),
                        "Index"
                    }
                }
                div { class: "row-flex",
                    input {
                        class: "spacer",
                        placeholder: "Filter the library",
                        value: "{filter}",
                        oninput: move |e| filter.set(e.value()),
                    }
                    select {
                        onchange: move |e| sort.set(match e.value().as_str() {
                            "tempo" => Sort::Tempo,
                            "key" => Sort::Key,
                            _ => Sort::Name,
                        }),
                        option { value: "name", "Name" }
                        option { value: "tempo", "BPM" }
                        option { value: "key", "Key" }
                    }
                }
                div { class: "muted small",
                    match &library {
                        None => rsx! { "Reading the library…" },
                        Some(lib) if total == lib.0.tracks.len() => rsx! { "{total} tracks" },
                        Some(lib) => rsx! { "{total} of {lib.0.tracks.len()} tracks" },
                    }
                }
            }
            div { class: "tracks",
                for (name, bpm, key, sings, path) in rows.into_iter().take(SHOWN) {
                    div {
                        key: "{path.display()}",
                        class: if selected.as_ref() == Some(&path) { "track-row selected" } else { "track-row" },
                        title: "{path.display()}",
                        onclick: move |_| state.open(path.clone()),
                        span { class: "ellipsis", "{name}" }
                        if sings {
                            // The word, not the icon: at this size a glyph
                            // for a voice is a glyph for anything.
                            span {
                                class: "badge warm vocals",
                                title: "there is singing in this, as CLAP hears it: a guess, not a tag",
                                "vocals"
                            }
                        }
                        span { class: "bpm", "{bpm}" }
                        span { class: "key", "{key}" }
                    }
                }
                if total > SHOWN {
                    div { class: "muted small", style: "padding: 6px 8px", "and {total - SHOWN} more: filter to find them" }
                }
                if let Some(lib) = &library {
                    if lib.0.tracks.is_empty() {
                        div { class: "muted small", style: "padding: 6px 8px",
                            "Nothing indexed yet. Give a folder of music above, or `stelly-mix index <folder>`."
                        }
                    }
                }
            }
            Status {}
        }
    }
}

#[component]
fn Status() -> Element {
    let state = use_context::<State>();
    let mut error = state.error;
    rsx! {
        if let Some(busy) = (state.busy)() {
            div { class: "status", "{busy}" }
        }
        if let Some(message) = (state.load_error)() {
            div { class: "status error", "The library could not be read: {message}" }
        }
        if let Some(message) = error() {
            div { class: "status",
                div { class: "row-flex",
                    span { class: "error spacer", "{message}" }
                    button { onclick: move |_| error.set(None), "×" }
                }
            }
        }
    }
}

/// Index `argument` on the blocking pool, saying how it goes, then read the
/// library again: a `stelly:` selection on Stelly's server, then synced in;
/// anything else here.
fn start_indexing(state: State, argument: String) {
    if argument.starts_with("stelly:") {
        if let Some(stelly) = work::stelly() {
            return start_server_analysis(state, stelly, argument);
        }
    }
    let mut busy = state.busy;
    busy.set(Some(format!("Indexing {argument}…")));
    let (tell, mut heard) = tokio::sync::mpsc::unbounded_channel::<String>();
    spawn(async move {
        while let Some(line) = heard.recv().await {
            busy.set(Some(line));
        }
    });
    spawn(async move {
        let shown = argument.clone();
        let done = work::blocking(move || {
            let library = work::library()?;
            let files = indexing::gather(&[argument], work::sources())?;
            let report = |event: Event| {
                let line = match event {
                    Event::Starting { total } => format!("Indexing {total} tracks…"),
                    Event::Indexed { done, total, track } => format!("Indexed {done} of {total}: {}", track.name()),
                    Event::Failed { name, error } => format!("{name}: {error}"),
                };
                let _ = tell.send(line);
            };
            indexing::run(files, &library, &stelly_mix::Options::default(), false, &report)
        })
        .await;
        busy.set(None);
        match done {
            Ok(0) => state.fail(anyhow::anyhow!("nothing new to index in {shown}")),
            Ok(_) => state.reload(),
            Err(err) => state.fail(err),
        }
    });
}

/// Have Stelly's server analyse `argument`, following it in the status line,
/// then bring what it made into the library.
fn start_server_analysis(state: State, stelly: &'static stelly_mix::stelly::Stelly, argument: String) {
    let mut busy = state.busy;
    let selection: stelly_mix::api::Selection = match argument.parse() {
        Ok(selection) => selection,
        Err(err) => return state.fail(anyhow::anyhow!(err)),
    };
    busy.set(Some(format!("Asking Stelly's server for {selection}…")));
    let (tell, mut heard) = tokio::sync::mpsc::unbounded_channel::<String>();
    spawn(async move {
        while let Some(line) = heard.recv().await {
            busy.set(Some(line));
        }
    });
    spawn(async move {
        let done = work::blocking(move || {
            let stats = stelly.analyse_and_wait(&selection, &|line| {
                let _ = tell.send(format!("Server: {line}"));
            })?;
            let _ = tell.send(format!("Server analysed {} ({} failed); bringing them in…", stats.done, stats.failed));
            stelly.sync(&work::library()?, &|_, _| {})
        })
        .await;
        busy.set(None);
        match done {
            Ok(_) => state.reload(),
            Err(err) => state.fail(err),
        }
    });
}

/// Camelot keys in wheel order: 1A, 1B, 2A … 12B.
fn camelot_order(camelot: &str) -> (u32, char) {
    let (number, letter) = camelot.split_at(camelot.len().saturating_sub(1));
    (number.parse().unwrap_or(99), letter.chars().next().unwrap_or('Z'))
}

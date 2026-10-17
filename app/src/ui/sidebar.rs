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
    let rows: Vec<(String, String, String, std::path::PathBuf)> = match &library {
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
                    (t.name(), bpm, key, t.path.clone())
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
                        placeholder: "File, folder or source:reference",
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
                for (name, bpm, key, path) in rows.into_iter().take(SHOWN) {
                    div {
                        key: "{path.display()}",
                        class: if selected.as_ref() == Some(&path) { "track-row selected" } else { "track-row" },
                        title: "{path.display()}",
                        onclick: move |_| state.open(path.clone()),
                        span { class: "ellipsis", "{name}" }
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
/// library again.
fn start_indexing(state: State, argument: String) {
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

/// Camelot keys in wheel order: 1A, 1B, 2A … 12B.
fn camelot_order(camelot: &str) -> (u32, char) {
    let (number, letter) = camelot.split_at(camelot.len().saturating_sub(1));
    (number.parse().unwrap_or(99), letter.chars().next().unwrap_or('Z'))
}

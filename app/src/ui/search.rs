//! The search bar across the top, and what it finds.
//!
//! One search for the whole window, rather than a filter in the sidebar and
//! another inside every gap. Results are cards — cover, name, artist, and
//! the few facts that decide whether a track belongs in a mix — because a
//! line of text is not enough to choose from and a sidebar row was all there
//! was before.
//!
//! A card goes into a set either by being dragged onto the line between two
//! tracks, or by its own Add, which puts it at the end.

use super::{icons, Lib, State};
use crate::work;
use dioxus::prelude::*;

/// How many cards at once. Enough to choose from, few enough that the
/// covers behind them are a handful of requests rather than a thousand.
const SHOWN: usize = 24;

/// What the window is searching for, and what is being dragged out of it.
#[derive(Clone, Copy)]
pub struct Search {
    pub query: Signal<String>,
    /// The origin key under the pointer, while a card is being dragged.
    pub dragging: Signal<Option<String>>,
    /// Asked to be put into the set being edited, by a card's own button.
    pub wanted: Signal<Option<String>>,
}

impl Search {
    pub fn provide() -> Self {
        use_context_provider(|| Search { query: Signal::new(String::new()), dragging: Signal::new(None), wanted: Signal::new(None) })
    }
}

#[component]
pub fn SearchBar() -> Element {
    let mut search = use_context::<Search>();
    rsx! {
        div { class: "searchbar",
            {icons::find()}
            input {
                placeholder: "search the library — drag a result between two tracks, or add it to the set",
                value: "{search.query}",
                oninput: move |event| search.query.set(event.value()),
                onkeydown: move |event: Event<KeyboardData>| {
                    if event.key() == Key::Escape {
                        search.query.set(String::new());
                    }
                },
            }
            if !search.query.read().trim().is_empty() {
                button { class: "icon-btn", title: "clear the search", onclick: move |_| search.query.set(String::new()),
                    {icons::close()}
                }
            }
        }
    }
}

/// The cards, under the bar, pushing the view down rather than covering it:
/// a result has to be draggable onto a tracklist that is still visible.
#[component]
pub fn Results() -> Element {
    let state = use_context::<State>();
    let search = use_context::<Search>();
    let words = search.query.read().trim().to_string();
    if words.is_empty() {
        return rsx! {};
    }
    let Some(library) = state.library.read().clone() else {
        return rsx! { div { class: "results", p { class: "muted small", "loading the library…" } } };
    };

    let found = work::search_library(&library.0, &words, SHOWN);
    let keys: Vec<String> = found.iter().filter_map(|&i| library.0.tracks[i].origin.as_ref().map(|o| o.key())).collect();

    // Ask for the covers of what is on screen, once per set of results.
    let mut art: Signal<std::collections::HashMap<String, String>> = use_signal(Default::default);
    use_effect({
        let keys = keys.clone();
        move || {
            let keys = keys.clone();
            art.set(work::known_covers(&keys));
            spawn(async move {
                let asking = keys.clone();
                let _ = work::blocking(move || work::fetch_covers(&asking)).await;
                art.set(work::known_covers(&keys));
            });
        }
    });

    rsx! {
        div { class: "results",
            div { class: "results-head",
                span { class: "eyebrow", "{found.len()} of the library" }
                span { class: "muted small", if found.len() == SHOWN { "showing the first {SHOWN} — narrow it down" } else { "" } }
            }
            if found.is_empty() {
                p { class: "muted small", "Nothing matches. Every word has to appear in the name." }
            }
            div { class: "cards",
                for &i in found.iter() {
                    Card { key: "{i}", index: i, library: library.clone(), art: art }
                }
            }
        }
    }
}

/// One result: what it is, and the two ways of getting it into a set.
#[component]
fn Card(index: usize, library: Lib, art: Signal<std::collections::HashMap<String, String>>) -> Element {
    let state = use_context::<State>();
    let mut search = use_context::<Search>();
    let track = &library.0.tracks[index];
    let Some(origin) = track.origin.as_ref() else { return rsx! {} };
    let key = origin.key();
    let cover = art.read().get(&key).cloned();
    let steady = track.analysis.grid.as_ref().is_some_and(|g| g.steady);
    let path = track.path.clone();

    rsx! {
        div {
            class: "card",
            draggable: true,
            title: "{track.name()}",
            ondragstart: {
                let key = key.clone();
                move |_| search.dragging.set(Some(key.clone()))
            },
            ondragend: move |_| search.dragging.set(None),
            div { class: "card-art",
                if let Some(url) = cover {
                    img { src: "{url}", alt: "" }
                }
            }
            div { class: "card-body",
                div { class: "card-title ellipsis", "{origin.item.title}" }
                div { class: "card-artist ellipsis", "{origin.item.artist}" }
                div { class: "card-facts",
                    span { class: "num", "{track.bpm().unwrap_or(0.0):.0}" }
                    if let Some(camelot) = track.analysis.key.as_ref().map(|k| k.camelot.clone()) {
                        span { "{camelot}" }
                    }
                    if !steady { span { class: "badge warm", title: "its tempo drifts, so a mix on it will not hold", "drifts" } }
                    {super::vocals_badge(track)}
                }
            }
            div { class: "card-actions",
                button {
                    class: "ghost",
                    title: "open this track's details",
                    onclick: move |_| state.open(path.clone()),
                    "Details"
                }
                button {
                    class: "secondary",
                    title: "put it at the end of the set being edited",
                    onclick: {
                        let key = key.clone();
                        move |_| search.wanted.set(Some(key.clone()))
                    },
                    {icons::plus()}
                    "Add"
                }
            }
        }
    }
}

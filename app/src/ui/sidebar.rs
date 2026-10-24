//! The source, down the left: what to put in the set next.
//!
//! This used to be the library — six thousand names in a column with a
//! filter over them, which is not how anyone finds music. Nobody browses a
//! library that size; they ask it a question. So the pane answers one, four
//! ways: what was liked on Stelly lately, what fits the gap you have picked,
//! what matches a name, and what sounds like a description.
//!
//! It is one half of the workbench. The set is the other, in the main area,
//! and the two share a `Bench` — which is what lets a row here know what it
//! would be mixing into, and lets its ⊕ put the track exactly where the
//! insert point is rather than nowhere.

use super::sets::Bench;
use super::{icons, score_class, Lib, State};
use crate::work;
use dioxus::prelude::*;

/// Rows at once. A column this narrow shows about this many without
/// scrolling, and the covers behind them are one request.
const SHOWN: usize = 40;

/// Which question the pane is answering.
#[derive(Clone, Copy, PartialEq)]
enum Asking {
    /// Liked on Stelly, newest first: what was found, ready to be mixed.
    Liked,
    Fits,
    Name,
    Sounds,
}

#[component]
pub fn Sidebar() -> Element {
    let state = use_context::<State>();
    let bench = use_context::<Bench>();
    // Opens on what was found, when there is somewhere it was found.
    let mut asking = use_signal(|| if work::stelly().is_some() { Asking::Liked } else { Asking::Fits });
    // What the liked list could not show: how many wait, or why it failed.
    let mut note: Signal<Option<String>> = use_signal(|| None);
    let mut words = use_signal(String::new);
    let mut found: Signal<Vec<(String, Option<f64>)>> = use_signal(Vec::new);
    let mut looking = use_signal(|| false);
    let mut art: Signal<std::collections::HashMap<String, String>> = use_signal(Default::default);

    let library = (state.library)();
    let set = bench.set();
    let at = bench.insert_at.read().unwrap_or(usize::MAX);

    // Re-asked whenever the question, the words, the set or the insert point
    // changes: the answer depends on all four.
    use_effect(move || {
        let (which, text) = (asking(), words().trim().to_string());
        let Some(lib) = (state.library)() else { return };
        let set = bench.set();
        let gap = bench.insert_at.read().unwrap_or(usize::MAX);
        looking.set(true);
        note.set(None);
        spawn(async move {
            if which == Asking::Liked {
                let answer = work::blocking({
                    let lib = lib.clone();
                    move || work::liked(&lib.0, SHOWN)
                })
                .await;
                looking.set(false);
                let keys = match answer {
                    Ok(now) => {
                        note.set(Some(match now.waiting {
                            0 => format!("{} liked lately", now.keys.len()),
                            n => format!("{} liked lately, {n} more still being analysed", now.keys.len()),
                        }));
                        if now.brought > 0 {
                            // Read again, so their rows have something to show;
                            // this then runs again and brings nothing.
                            state.reload();
                        }
                        now.keys
                    }
                    Err(err) => {
                        note.set(Some(format!("{err:#}")));
                        Vec::new()
                    }
                };
                found.set(keys.iter().map(|key| (key.clone(), None)).collect());
                art.set(work::known_covers(&keys));
                let _ = work::blocking({
                    let keys = keys.clone();
                    move || work::fetch_covers(&keys)
                })
                .await;
                art.set(work::known_covers(&keys));
                return;
            }
            let hits = work::blocking(move || {
                Ok(match which {
                    Asking::Liked => Vec::new(),
                    Asking::Fits => match &set {
                        Some(set) if !set.tracks.is_empty() => {
                            work::fits_between(&lib.0, set, gap.min(set.tracks.len()), SHOWN)
                                .into_iter()
                                .map(|fit| (fit.key, Some(fit.mean)))
                                .collect()
                        }
                        _ => Vec::new(),
                    },
                    Asking::Name => work::search_library(&lib.0, &text, SHOWN)
                        .into_iter()
                        .filter_map(|i| Some((lib.0.tracks[i].origin.as_ref()?.key(), None)))
                        .collect(),
                    Asking::Sounds => work::sounds_like(&lib.0, &text, SHOWN)?
                        .into_iter()
                        .map(|(key, rank)| (key, Some(rank)))
                        .collect(),
                })
            })
            .await
            .unwrap_or_default();
            let keys: Vec<String> = hits.iter().map(|(key, _)| key.clone()).collect();
            found.set(hits);
            looking.set(false);
            art.set(work::known_covers(&keys));
            let _ = work::blocking({
                let keys = keys.clone();
                move || work::fetch_covers(&keys)
            })
            .await;
            art.set(work::known_covers(&keys));
        });
    });

    let tab = move |which: Asking, label: &'static str, hint: &'static str| {
        rsx! {
            button {
                class: if asking() == which { "active" } else { "" },
                title: "{hint}",
                onclick: move |_| asking.set(which),
                "{label}"
            }
        }
    };

    rsx! {
        aside { class: "sidebar",
            header {
                div { class: "brand", "stelly", span { "-mix" } }
                div { class: "segmented wide",
                    {tab(Asking::Liked, "liked", "what was liked on Stelly lately, newest first")}
                    {tab(Asking::Fits, "fits here", "what mixes best into the gap you have picked")}
                    {tab(Asking::Name, "by name", "search the library by artist or title")}
                    {tab(Asking::Sounds, "by sound", "describe it and the library is ranked against the words")}
                }
                if asking() == Asking::Name || asking() == Asking::Sounds {
                    input {
                        placeholder: if asking() == Asking::Name { "artist or title" } else { "e.g. dark rolling techno" },
                        value: "{words}",
                        oninput: move |event| words.set(event.value()),
                    }
                }
                div { class: "muted small row-flex",
                    if looking() {
                        span { class: "working-dot" }
                    }
                    match (asking(), &set) {
                        (Asking::Liked, _) => rsx! { span { {note().unwrap_or_default()} } },
                        (Asking::Fits, None) => rsx! { span { "open a set to see what fits it" } },
                        (Asking::Fits, Some(set)) if set.tracks.is_empty() => rsx! { span { "plan or add a track first" } },
                        (Asking::Fits, Some(set)) => rsx! {
                            span { "into {gap_label(at, set.tracks.len())}" }
                        },
                        _ => rsx! { span { "{found.read().len()} of the library" } },
                    }
                }
            }

            div { class: "sources",
                if let Some(library) = library {
                    for (key, score) in found() {
                        SourceRow { key: "{key}", origin: key.clone(), score, library: library.clone(), art }
                    }
                } else {
                    p { class: "muted small", "reading the library…" }
                }
            }
        }
    }
}

/// Where the insert point is, said in a way that names the tracks it falls
/// between rather than a number nobody is counting.
fn gap_label(at: usize, len: usize) -> String {
    match at {
        0 => "the front".into(),
        at if at >= len => "the end".into(),
        at => format!("after {at}"),
    }
}

/// One candidate: what it is, how well it would sit there, and a way in.
#[component]
fn SourceRow(
    origin: String,
    score: Option<f64>,
    library: Lib,
    art: Signal<std::collections::HashMap<String, String>>,
) -> Element {
    let state = use_context::<State>();
    let mut bench = use_context::<Bench>();
    let Some(track) = library.0.tracks.iter().find(|t| t.origin.as_ref().is_some_and(|o| o.key() == origin)) else {
        return rsx! {};
    };
    let item = track.origin.as_ref().expect("found by origin");
    let cover = art.read().get(&origin).cloned();
    let steady = track.analysis.grid.as_ref().is_some_and(|g| g.steady);
    let sings = track.vocals.as_ref().is_some_and(|heard| heard.speaks());
    let path = track.path.clone();

    rsx! {
        div { class: "source-row", title: "{track.name()}",
            div { class: "source-art",
                if let Some(url) = cover {
                    img { src: "{url}", alt: "" }
                }
            }
            div { class: "source-of",
                div { class: "source-title ellipsis", "{item.item.title}" }
                div { class: "source-artist ellipsis", "{item.item.artist}" }
                div { class: "source-facts",
                    span { class: "num", "{track.bpm().unwrap_or(0.0):.0}" }
                    if let Some(camelot) = track.analysis.key.as_ref().map(|k| k.camelot.clone()) {
                        span { "{camelot}" }
                    }
                    if sings { span { class: "dot warm", title: "there is singing in it: a guess, not a tag", "•" } }
                    if !steady { span { class: "dot bad", title: "its tempo drifts", "•" } }
                }
            }
            div { class: "source-end",
                if let Some(score) = score {
                    span { class: "{score_class(score)}", "{score:.2}" }
                }
                div { class: "source-actions",
                    button { class: "icon-btn", title: "open this track's details", onclick: move |_| state.open(path.clone()),
                        {icons::track()}
                    }
                    button {
                        class: "icon-btn add",
                        title: "put it in the set at the insert point",
                        disabled: bench.editing.read().is_none(),
                        onclick: {
                            let origin = origin.clone();
                            move |_| bench.wanted.set(Some(origin.clone()))
                        },
                        {icons::plus()}
                    }
                }
            }
        }
    }
}

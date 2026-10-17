//! The library searched by description, through CLAP's text tower.

use super::{score_class, Lib, State};
use crate::work;
use dioxus::prelude::*;

/// Matches listed.
const LISTED: usize = 60;

#[component]
pub fn FindView() -> Element {
    let state = use_context::<State>();
    let Some(lib) = (state.library)() else {
        return rsx! { div { class: "view", div { class: "empty", "Reading the library…" } } };
    };
    // Afresh for each library load: the matches are places in the one searched.
    let key = format!("{:p}", std::sync::Arc::as_ptr(&lib.0));
    rsx! {
        for key in [key] {
            FindPanel { key: "{key}", lib: lib.clone() }
        }
    }
}

#[component]
fn FindPanel(lib: Lib) -> Element {
    let state = use_context::<State>();
    let mut query = use_signal(String::new);
    let mut found = use_signal(|| None::<Vec<(usize, f64)>>);
    let mut busy = use_signal(|| false);

    let search = use_callback({
        let lib = lib.clone();
        move |_: ()| {
            let description = query().trim().to_string();
            if description.is_empty() || busy() {
                return;
            }
            busy.set(true);
            let lib = lib.clone();
            spawn(async move {
                let ranked = work::blocking(move || {
                    let like = work::like(&lib.0, &description)?;
                    let mut order: Vec<(usize, f64)> = like.into_iter().enumerate().collect();
                    order.sort_by(|a, b| b.1.total_cmp(&a.1));
                    order.truncate(LISTED);
                    Ok(order)
                })
                .await;
                busy.set(false);
                match ranked {
                    Ok(ranked) => found.set(Some(ranked)),
                    Err(err) => state.fail(err),
                }
            });
        }
    });

    rsx! {
        div { class: "view",
            div { class: "title", "Find by description" }
            div { class: "panel",
                div { class: "row-flex",
                    input {
                        class: "spacer",
                        placeholder: "e.g. chill downtempo with soft pads",
                        value: "{query}",
                        oninput: move |e| query.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                search.call(());
                            }
                        },
                    }
                    button {
                        class: "primary",
                        disabled: busy() || query().trim().is_empty(),
                        onclick: move |_| search.call(()),
                        if busy() { "Listening…" } else { "Find" }
                    }
                }
                div { class: "muted small",
                    "Each track is scored by how much nearer the description its phrases are than plain \"music\", then ranked: 1 is the best match in the library."
                }
            }
            match found() {
                None => rsx! {},
                Some(ranked) => rsx! {
                    div { class: "panel",
                        div { class: "list",
                            for (i, score) in ranked {
                                Match { key: "{i}", lib: lib.clone(), track: i, score }
                            }
                        }
                    }
                },
            }
        }
    }
}

#[component]
fn Match(lib: Lib, track: usize, score: f64) -> Element {
    let state = use_context::<State>();
    let t = &lib.0.tracks[track];
    let bpm = t.bpm().map_or("no tempo".into(), |b| format!("{b:.1} BPM"));
    let key = t.analysis.key.as_ref().map_or("?".into(), |k| k.camelot.clone());
    let path = t.path.clone();
    rsx! {
        div { class: "item",
            span { class: score_class(score), "{score:.2}" }
            div {
                div { class: "name ellipsis", "{t.name()}" }
                div { class: "detail", "{bpm} · {key} · {work::clock(t.analysis.seconds)}" }
            }
            div { class: "actions",
                button { onclick: move |_| state.open(path.clone()), "Open" }
            }
        }
    }
}

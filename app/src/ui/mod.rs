//! The window: the library down the left, a track, a set or a search in the
//! middle, the player along the bottom.

mod find;
pub mod icons;
mod player;
mod set;
mod sets;
mod settings;
mod sidebar;
mod track;
mod waveform;

use crate::work::{self, Loaded};
use dioxus::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

pub use player::Player;

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Track,
    Set,
    Sets,
    Find,
    Settings,
}

/// The library, compared by which load it is rather than by what it holds,
/// so a component handed it re-renders when the library is reloaded.
#[derive(Clone)]
pub struct Lib(pub Arc<Loaded>);

impl PartialEq for Lib {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What every part of the window shares.
#[derive(Clone, Copy)]
pub struct State {
    pub library: Signal<Option<Lib>>,
    /// Why the library could not be loaded, if it could not.
    pub load_error: Signal<Option<String>>,
    /// The track shown, by path, which survives reloading the library.
    pub selected: Signal<Option<PathBuf>>,
    pub view: Signal<View>,
    /// What is under way, and what last went wrong.
    pub busy: Signal<Option<String>>,
    pub error: Signal<Option<String>>,
    pub player: Player,
}

impl State {
    /// Show `path` as the track.
    pub fn open(mut self, path: PathBuf) {
        self.selected.set(Some(path));
        self.view.set(View::Track);
    }

    /// Bring what Stelly's server has analysed since into the library, if a
    /// server is set, then read the library; reading it right away first, so
    /// the window has something to show while that runs.
    pub fn sync(self) {
        self.reload();
        let Some(stelly) = work::stelly() else { return };
        let mut busy = self.busy;
        busy.set(Some("Asking Stelly's server what it has analysed…".into()));
        let (tell, mut heard) = tokio::sync::mpsc::unbounded_channel::<String>();
        spawn(async move {
            while let Some(line) = heard.recv().await {
                busy.set(Some(line));
            }
        });
        spawn(async move {
            let brought = work::blocking(move || {
                let library = work::library()?;
                stelly.sync(&library, &|done, total| {
                    if total > 0 {
                        let _ = tell.send(format!("Bringing Stelly's analyses: {done} of {total}"));
                    }
                })
            })
            .await;
            busy.set(None);
            match brought {
                Ok(0) => {}
                Ok(_) => self.reload(),
                Err(err) => self.fail(err.context("syncing from Stelly's server")),
            }
        });
    }

    /// Read the library again, after indexing.
    pub fn reload(self) {
        let (mut library, mut load_error) = (self.library, self.load_error);
        spawn(async move {
            match work::blocking(work::load).await {
                Ok(loaded) => {
                    library.set(Some(Lib(Arc::new(loaded))));
                    load_error.set(None);
                }
                Err(err) => load_error.set(Some(format!("{err:#}"))),
            }
        });
    }

    pub fn fail(mut self, err: anyhow::Error) {
        self.error.set(Some(format!("{err:#}")));
    }
}

#[component]
pub fn App() -> Element {
    let state = use_context_provider(|| State {
        library: Signal::new(None),
        load_error: Signal::new(None),
        selected: Signal::new(None),
        view: Signal::new(View::Track),
        busy: Signal::new(None),
        error: Signal::new(None),
        player: Player::new(),
    });
    use_hook(move || state.sync());
    player::use_transport(state);

    rsx! {
        style { {include_str!("../../assets/style.css")} }
        div { class: "frame",
            sidebar::Sidebar {}
            div { class: "main",
                Tabs {}
                match (state.view)() {
                    View::Track => rsx! { track::TrackView {} },
                    View::Set => rsx! { set::SetView {} },
                    View::Sets => rsx! { sets::SetsView {} },
                    View::Find => rsx! { find::FindView {} },
                    View::Settings => rsx! { settings::SettingsView {} },
                }
            }
            player::PlayerBar {}
        }
    }
}

#[component]
fn Tabs() -> Element {
    let mut view = use_context::<State>().view;
    let tab = move |which: View, label: &'static str, icon: Element| {
        rsx! {
            button {
                class: if view() == which { "tab active" } else { "tab" },
                onclick: move |_| view.set(which),
                {icon}
                "{label}"
            }
        }
    };
    rsx! {
        div { class: "tabs",
            {tab(View::Track, "Track", icons::track())}
            {tab(View::Set, "Set", icons::set())}
            {tab(View::Sets, "Sets", icons::sets())}
            {tab(View::Find, "Find", icons::find())}
            span { class: "spacer" }
            {tab(View::Settings, "Settings", icons::settings())}
        }
    }
}

/// The "vocals" badge for a track, when something has been heard in it.
///
/// Only shown where there is singing: a track with none says nothing, and a
/// track nobody has listened to says nothing either, which are different
/// things and neither is worth a badge. The title says it is a guess,
/// because it is one.
pub fn vocals_badge(track: &stelly_mix::library::Track) -> Element {
    let Some(heard) = track.vocals.as_ref().filter(|heard| heard.speaks()) else {
        return rsx! {};
    };
    rsx! {
        span {
            class: "badge warm vocals",
            title: "there is singing in this, as CLAP hears it: a guess, not a tag ({heard.track:+.2})",
            "vocals"
        }
    }
}

/// A transition or a match's score as a colour: good, middling, poor. Sound
/// is scored against each track's closest matches, so 0.6 is already among
/// the best a library offers.
pub fn score_class(score: f64) -> &'static str {
    if score >= 0.6 {
        "score high"
    } else if score >= 0.4 {
        "score mid"
    } else {
        "score low"
    }
}

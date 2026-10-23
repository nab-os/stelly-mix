//! Settings: the server this window talks to, and the analysis it asks for.
//!
//! Both are things that otherwise only exist on the command line. Pointing
//! at a server was an environment variable or a file edited by hand, which
//! is how a window ends up with a library it cannot fetch audio for; and
//! asking the server to analyse more was one line of status in the sidebar,
//! with no way to see what it was doing or to call it off.
//!
//! Paired devices are not here: they are Stelly's to hand out.

use super::{icons, State};
use crate::work;
use dioxus::prelude::*;
use stelly_mix::api::Selection;

#[component]
pub fn SettingsView() -> Element {
    rsx! {
        div { class: "view",
            Server {}
            Analysis {}
        }
    }
}

/// Where the music comes from.
#[component]
fn Server() -> Element {
    let state = use_context::<State>();
    let current = work::stelly();
    let mut base = use_signal(|| current.map(|server| server.base().to_string()).unwrap_or_default());
    let mut token = use_signal(String::new);
    let trying = use_signal(|| false);
    let said: Signal<Option<Result<String, String>>> = use_signal(|| None);

    // Local copies of the signals, so the closure stays Copy and both the
    // button and the Enter key can call it.
    let connect = move |_: ()| {
        let (mut trying, mut said, mut token) = (trying, said, token);
        if trying() {
            return;
        }
        let (where_to, secret) = (base().trim().to_string(), token().trim().to_string());
        if where_to.is_empty() || secret.is_empty() {
            said.set(Some(Err("Both an address and a token, please.".into())));
            return;
        }
        trying.set(true);
        said.set(None);
        spawn(async move {
            let tried = work::blocking(move || work::set_server(&where_to, &secret)).await;
            trying.set(false);
            match tried {
                Ok(status) => {
                    token.set(String::new());
                    said.set(Some(Ok(format!(
                        "Connected. {} tracks analysed for mixing, at analysis version {}.",
                        status.tracks, status.version
                    ))));
                    state.sync();
                }
                Err(err) => said.set(Some(Err(format!("{err:#}")))),
            }
        });
    };

    rsx! {
        section { class: "panel",
            h3 {
                {icons::server()}
                "Stelly's server"
                span { class: "spacer" }
                if work::stelly().is_some() {
                    span { class: "badge good", "connected" }
                } else {
                    span { class: "badge bad", "not set" }
                }
            }
            p { class: "muted small",
                "Where the mix library is mirrored from, and where a track's audio is fetched when it is played, mixed or exported."
            }
            div { class: "form",
                div { class: "field wide",
                    label { "Address" }
                    input {
                        placeholder: "https://music.example.org",
                        value: "{base}",
                        oninput: move |event| base.set(event.value()),
                    }
                }
                div { class: "field wide",
                    label { "Token" }
                    input {
                        r#type: "password",
                        placeholder: if work::stelly().is_some() { "kept; type a new one to change it" } else { "a device's token" },
                        value: "{token}",
                        oninput: move |event| token.set(event.value()),
                        onkeydown: move |event: Event<KeyboardData>| {
                            if event.key() == Key::Enter { connect(()); }
                        },
                    }
                }
                button { class: "primary", disabled: trying(), onclick: move |_| connect(()),
                    if trying() { "trying…" } else { "Connect" }
                }
            }
            // Tried before it is kept: a token that does not answer should
            // not replace one that does.
            match said() {
                Some(Ok(good)) => rsx! { p { class: "told", "{good}" } },
                Some(Err(bad)) => rsx! { p { class: "error small selectable", "{bad}" } },
                None => rsx! {},
            }
            if let Some(file) = stelly_mix::stelly::config_file() {
                p { class: "muted small selectable", "Kept in {file.display()}, so every later run finds it." }
            }
            if stelly_mix::stelly::env_overrides() {
                p { class: "warning",
                    "STELLY_SERVER or STELLY_TOKEN is set in this window's environment, and wins over what is saved here."
                }
            }
        }
    }
}

/// Asking the server to analyse more tracks for mixing.
#[component]
fn Analysis() -> Element {
    let state = use_context::<State>();
    let mut asking = use_signal(String::new);
    let running = use_signal(|| false);
    let lines: Signal<Vec<String>> = use_signal(Vec::new);
    let failed: Signal<Option<String>> = use_signal(|| None);

    let Some(server) = work::stelly() else {
        return rsx! {
            section { class: "panel",
                h3 { {icons::analyse()} "Analysing for mixing" }
                p { class: "muted small", "Connect to a server above, and what it has analysed can be added to from here." }
            }
        };
    };

    let start = move |_: ()| {
        let (mut running, mut lines, mut failed) = (running, lines, failed);
        if running() {
            return;
        }
        let selection: Selection = match asking().trim().parse() {
            Ok(selection) => selection,
            Err(err) => return failed.set(Some(err)),
        };
        running.set(true);
        failed.set(None);
        lines.set(vec![format!("asking for {selection}")]);

        // The job is the server's: it keeps going whether or not this window
        // is still watching, so the lines come back over a channel rather
        // than the call being awaited.
        let (tell, mut heard) = tokio::sync::mpsc::unbounded_channel::<String>();
        spawn(async move {
            while let Some(line) = heard.recv().await {
                lines.write().push(line);
            }
        });
        spawn(async move {
            let done = work::blocking(move || {
                let stats = server.analyse_and_wait(&selection, &|line| {
                    let _ = tell.send(line.to_string());
                })?;
                let _ = tell.send(format!("{} analysed, {} failed, of {}; bringing them in", stats.done, stats.failed, stats.total));
                server.sync(&work::library()?, &|_, _| {})
            })
            .await;
            running.set(false);
            match done {
                Ok(brought) => {
                    lines.write().push(format!("{brought} entries are in the library"));
                    state.reload();
                }
                Err(err) => failed.set(Some(format!("{err:#}"))),
            }
        });
    };

    let stop = move |_| {
        let (mut lines, mut failed) = (lines, failed);
        spawn(async move {
            match work::blocking(move || server.stop()).await {
                Ok(status) => lines.write().push(match status.running {
                    true => format!("stopping {}", status.selection.unwrap_or_else(|| "the analysis".into())),
                    false => "nothing was running".into(),
                }),
                Err(err) => failed.set(Some(format!("{err:#}"))),
            }
        });
    };

    rsx! {
        section { class: "panel",
            h3 {
                {icons::analyse()}
                "Analysing for mixing"
                span { class: "spacer" }
                if running() {
                    span { class: "row-flex", span { class: "working-dot" } span { class: "muted small hint", "running" } }
                }
            }
            p { class: "muted small",
                "A whole track is ten times an excerpt's download and CPU, so the server analyses what it is asked to. It keeps going if this window closes."
            }
            div { class: "form",
                div { class: "field wide",
                    label { "Analyse" }
                    input {
                        placeholder: "album/<id>, artist-name/Lun?n=30, mood/dark rolling techno?n=200, favourites, genre/techno",
                        value: "{asking}",
                        oninput: move |event| asking.set(event.value()),
                        onkeydown: move |event: Event<KeyboardData>| {
                            if event.key() == Key::Enter { start(()); }
                        },
                    }
                }
                button { class: "primary", disabled: running() || asking().trim().is_empty(), onclick: move |_| start(()), "Analyse" }
                button { class: "danger", disabled: !running(), onclick: stop, "Stop" }
            }
            p { class: "muted small",
                "Wants a device paired with admin scope. Hours of downloading and CPU for a large selection."
            }
            if let Some(why) = failed() {
                p { class: "error small selectable", "{why}" }
            }
            if !lines.read().is_empty() {
                div { class: "log selectable",
                    for line in lines().iter().rev().take(14) {
                        div { class: "log-line", "{line}" }
                    }
                }
            }
        }
    }
}

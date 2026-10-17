//! stelly-mix in a window: the library, a track's beats, bars and phrases
//! over its waveform, where to go next and how it sounds, sets planned and
//! played, and the library searched by description.
//!
//! A library, like stelly-mix's command line, so that a build with more
//! sources than local files is a `main` calling `run` with them.

pub mod server;
mod ui;
pub mod work;

use stelly_mix::source::Source;

/// Open the window, with `sources` besides local files.
pub fn run(sources: Vec<Box<dyn Source>>) {
    work::set_sources(sources);
    if let Err(err) = server::start() {
        eprintln!("could not start serving audio to the window: {err}");
        return;
    }
    let window = dioxus::desktop::WindowBuilder::new()
        .with_title("stelly-mix")
        .with_inner_size(dioxus::desktop::LogicalSize::new(1440.0, 920.0));
    let config = dioxus::desktop::Config::new().with_window(window).with_menu(None);
    dioxus::LaunchBuilder::desktop().with_cfg(config).launch(ui::App);
}

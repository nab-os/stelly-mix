//! stelly-mix with local files, and Stelly's server when `STELLY_SERVER` and
//! `STELLY_TOKEN` say where it is. A build with more sources is this with
//! them passed to `run`; see `stelly_mix::source`.

fn main() -> anyhow::Result<()> {
    stelly_mix::cli::run(stelly_mix::stelly::sources())
}

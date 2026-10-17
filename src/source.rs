//! Where tracks come from. Local files are built in. Anything else, a
//! streaming service or a server, is a `Source` that a build of stelly-mix
//! registers with `cli::run`, and which turns its own references into local
//! files: everything downstream, analysis, the library, renders, only ever
//! sees files.
//!
//! A reference is `name:rest`, `name` a registered source's: what `rest`
//! means, a track, a playlist, the user's favourites, is the source's
//! business. stelly-mix itself registers none, so it carries no service's
//! client, credentials or terms; a build that wants one adds it from its own
//! crate:
//!
//! ```ignore
//! fn main() -> anyhow::Result<()> {
//!     stelly_mix::cli::run(vec![Box::new(MyService::from_env()?)])
//! }
//! ```

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A track a source can fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    /// The source's own id for it; `name:id` refers to it.
    pub id: String,
    pub artist: String,
    pub title: String,
    pub seconds: Option<f64>,
}

impl Item {
    /// What its fetched file should be called, without an extension: the
    /// library names tracks after their files. Path separators are taken out.
    pub fn file_stem(&self) -> String {
        format!("{} - {} [{}]", self.artist, self.title, self.id).replace(['/', '\\', '\0'], "-")
    }
}

pub trait Source: Send + Sync {
    /// The prefix of its references.
    fn name(&self) -> &str;

    /// What a reference names: one item for a track, many for a collection.
    fn resolve(&self, reference: &str) -> Result<Vec<Item>>;

    /// Items matching a query, best first.
    fn search(&self, query: &str, limit: usize) -> Result<Vec<Item>>;

    /// The item's audio as a file in `cache`, fetched if it is not there
    /// already. Called from several threads at once while indexing.
    fn fetch(&self, item: &Item, cache: &Path) -> Result<PathBuf>;

    /// The item's file in `cache` if it was fetched already, without asking
    /// the service anything: what lets `index` pass over what it has.
    fn cached(&self, _item: &Item, _cache: &Path) -> Option<PathBuf> {
        None
    }
}

/// A file an argument comes to, not yet fetched.
pub enum Pending<'a> {
    File(PathBuf),
    Item(&'a dyn Source, Item),
}

impl Pending<'_> {
    /// The file, fetched into its source's cache if need be.
    pub fn fetch(&self) -> Result<PathBuf> {
        match self {
            Pending::File(path) => Ok(path.clone()),
            Pending::Item(source, item) => source.fetch(item, &cache_dir(source.name())),
        }
    }

    /// The file if it is there without fetching.
    pub fn cached(&self) -> Option<PathBuf> {
        match self {
            Pending::File(path) => Some(path.clone()),
            Pending::Item(source, item) => source.cached(item, &cache_dir(source.name())),
        }
    }

    /// What to call it in a message.
    pub fn name(&self) -> String {
        match self {
            Pending::File(path) => path.display().to_string(),
            Pending::Item(source, item) => format!("{}:{} ({} - {})", source.name(), item.id, item.artist, item.title),
        }
    }
}

/// The registered sources, and the files their references come to.
pub struct Sources(Vec<Box<dyn Source>>);

impl Sources {
    pub fn new(sources: Vec<Box<dyn Source>>) -> Self {
        Self(sources)
    }

    pub fn get(&self, name: &str) -> Option<&dyn Source> {
        self.0.iter().find(|s| s.name() == name).map(|s| s.as_ref())
    }

    pub fn names(&self) -> Vec<&str> {
        self.0.iter().map(|s| s.name()).collect()
    }

    /// What `argument` comes to, nothing fetched yet: a path is itself;
    /// `name:rest`, with `name` a registered source, is what that source
    /// resolves it to, each item once.
    pub fn pending(&self, argument: &str) -> Result<Vec<Pending<'_>>> {
        let Some((source, reference)) = self.split(argument) else {
            return Ok(vec![Pending::File(PathBuf::from(argument))]);
        };
        let mut seen = std::collections::HashSet::new();
        Ok(source
            .resolve(reference)?
            .into_iter()
            // A playlist can hold a track twice; fetched twice at once, the
            // two would write the same file.
            .filter(|item| seen.insert(item.id.clone()))
            .map(|item| Pending::Item(source, item))
            .collect())
    }

    /// `argument` as files, fetched. A track that cannot be fetched, a
    /// delisted one say, is reported and left out rather than failing the
    /// rest of its playlist.
    pub fn files(&self, argument: &str) -> Result<Vec<PathBuf>> {
        let pending = self.pending(argument)?;
        let files: Vec<PathBuf> = pending
            .iter()
            .filter_map(|p| p.fetch().map_err(|err| eprintln!("{}: {err:#}", p.name())).ok())
            .collect();
        if files.is_empty() && !pending.is_empty() {
            bail!("{argument}: none of its {} tracks could be fetched", pending.len());
        }
        Ok(files)
    }

    /// One file for `argument`, which must name one track.
    pub fn file(&self, argument: &str) -> Result<PathBuf> {
        let mut pending = self.pending(argument)?;
        if pending.len() != 1 {
            bail!("{argument} names {} tracks, not one", pending.len());
        }
        pending.remove(0).fetch()
    }

    fn split<'a>(&self, argument: &'a str) -> Option<(&dyn Source, &'a str)> {
        let (name, rest) = argument.split_once(':')?;
        Some((self.get(name)?, rest))
    }
}

/// `$XDG_CACHE_HOME/stelly-mix/<source>`, where a source's fetched files go.
pub fn cache_dir(source: &str) -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"))
        .join("stelly-mix")
        .join(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    impl Source for Fake {
        fn name(&self) -> &str {
            "fake"
        }
        fn resolve(&self, reference: &str) -> Result<Vec<Item>> {
            let item = |id: &str| Item {
                id: id.into(),
                artist: "A/B".into(),
                title: "T".into(),
                seconds: None,
            };
            Ok(match reference {
                "all" => vec![item("1"), item("2")],
                "playlist" => vec![item("1"), item("delisted"), item("1"), item("2")],
                "gone" => vec![item("delisted")],
                id => vec![item(id)],
            })
        }
        fn search(&self, _: &str, _: usize) -> Result<Vec<Item>> {
            Ok(Vec::new())
        }
        fn fetch(&self, item: &Item, cache: &Path) -> Result<PathBuf> {
            if item.id == "delisted" {
                bail!("not streamable");
            }
            Ok(cache.join(format!("{}.flac", item.file_stem())))
        }
    }

    #[test]
    fn references_go_to_their_source_and_paths_stay_paths() {
        let sources = Sources::new(vec![Box::new(Fake)]);
        assert_eq!(sources.file("some/track.flac").unwrap(), PathBuf::from("some/track.flac"));
        // An unknown prefix is a path with a colon in it, not an error.
        assert_eq!(sources.file("other:thing.flac").unwrap(), PathBuf::from("other:thing.flac"));
        let file = sources.file("fake:7").unwrap();
        assert_eq!(file.file_name().unwrap(), "A-B - T [7].flac");
        assert!(file.starts_with(cache_dir("fake")));
        assert_eq!(sources.files("fake:all").unwrap().len(), 2);
        assert!(sources.file("fake:all").is_err());
    }

    #[test]
    fn a_track_that_cannot_be_fetched_does_not_stop_its_playlist() {
        let sources = Sources::new(vec![Box::new(Fake)]);
        // The repeat is fetched once, the delisted track left out.
        assert_eq!(sources.pending("fake:playlist").unwrap().len(), 3);
        assert_eq!(sources.files("fake:playlist").unwrap().len(), 2);
        assert!(sources.files("fake:gone").is_err());
        // One track asked for by itself still fails with its own reason.
        let err = sources.file("fake:delisted").unwrap_err();
        assert!(format!("{err:#}").contains("not streamable"), "{err:#}");
    }
}

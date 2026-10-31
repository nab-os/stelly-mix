//! A mix library held in memory, named by the provider's id, and what is asked of it:
//! what a server answers its clients with. No input or output here: the
//! server loads the tracks, and embeds a description with its own text tower.

use crate::api::{Match, SetPlan, SetRequest, TrackSummary, Way};
use crate::library::Track;
use crate::transitions::{self, SoundScale};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::sync::Arc;

/// Tracks are held through `Arc`, so `with` shares them with the library it
/// came from: a server adds what its queue analyses to tens of thousands of
/// tracks without copying them, or reading them again.
pub struct Mix {
    tracks: Vec<Arc<Track>>,
    ids: Vec<i64>,
    by_id: HashMap<i64, usize>,
    scale: SoundScale,
}

impl Mix {
    /// `tracks`, each with its provider's id.
    pub fn new(entries: Vec<(i64, Track)>) -> Self {
        let (ids, tracks): (Vec<i64>, Vec<Arc<Track>>) = entries.into_iter().map(|(id, track)| (id, Arc::new(track))).unzip();
        let by_id = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        let scale = SoundScale::of(&tracks);
        Self { tracks, ids, by_id, scale }
    }

    /// This library with `entries` in it, each in place of the one of its id
    /// when there was one. The scale stays as it was measured: a few tracks
    /// among thousands do not move it, and measuring it again is the one
    /// slow part of a library's making.
    pub fn with(&self, entries: Vec<(i64, Track)>) -> Self {
        let (mut tracks, mut ids, mut by_id) = (self.tracks.clone(), self.ids.clone(), self.by_id.clone());
        for (id, track) in entries {
            let track = Arc::new(track);
            match by_id.get(&id) {
                Some(&i) => tracks[i] = track,
                None => {
                    by_id.insert(id, tracks.len());
                    ids.push(id);
                    tracks.push(track);
                }
            }
        }
        Self { tracks, ids, by_id, scale: self.scale }
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn track(&self, id: i64) -> Option<&Track> {
        self.by_id.get(&id).map(|&i| &*self.tracks[i])
    }

    fn index(&self, id: i64) -> Result<usize> {
        self.by_id.get(&id).copied().with_context(|| format!("track {id} has no mix analysis"))
    }

    pub fn summaries(&self) -> Vec<TrackSummary> {
        self.ids.iter().zip(&self.tracks).map(|(&id, t)| TrackSummary::of(id, t)).collect()
    }

    /// Where to go from `id`, best first, at most `limit`.
    pub fn next(&self, id: i64, stretch_percent: f64, limit: usize) -> Result<Vec<Way>> {
        let playing = self.index(id)?;
        let options = transitions::Options {
            max_stretch: stretch_percent / 100.0,
            sound: self.scale,
            ..Default::default()
        };
        let transitions = transitions::from(&self.tracks[playing], &self.tracks, &options);
        Ok(transitions
            .iter()
            .take(limit)
            .map(|t| Way::of(t, &self.tracks[playing], &self.tracks[t.to], self.ids[t.to]))
            .collect())
    }

    /// Every track's likeness to a description, embedded as `query`, against
    /// the baseline `text::BASELINE` embedded as `baseline`, by the text tower
    /// the phrases' audio tower shares a space with.
    pub fn likeness(&self, query: &[f32], baseline: &[f32]) -> Vec<f64> {
        crate::text::scores_for(query, baseline, &self.tracks)
    }

    /// The best matches for a description, as `likeness` gives them.
    pub fn find(&self, query: &[f32], baseline: &[f32], limit: usize) -> Vec<Match> {
        let like = self.likeness(query, baseline);
        let mut order: Vec<usize> = (0..like.len()).collect();
        order.sort_by(|&a, &b| like[b].total_cmp(&like[a]));
        order.into_iter().take(limit).map(|i| Match { id: self.ids[i], score: like[i] }).collect()
    }

    /// A set as `request` asks, `like` the library's likeness to its
    /// description when it has one.
    pub fn set(&self, request: &SetRequest, like: Option<Vec<f64>>) -> Result<SetPlan> {
        let start = match (request.first, &like) {
            (Some(first), _) => self.index(first)?,
            // The best match that keeps a steady tempo.
            (None, Some(like)) => (0..self.tracks.len())
                .filter(|&i| self.tracks[i].analysis.grid.as_ref().is_some_and(|g| g.steady))
                .max_by(|&a, &b| like[a].total_cmp(&like[b]))
                .context("no track in the library keeps a steady tempo")?,
            (None, None) => bail!("a set wants a first track or a description"),
        };
        let options = crate::set::Options {
            seconds: request.minutes.max(1.0) * 60.0,
            max_stretch: request.stretch / 100.0,
            beam: request.beam.max(1),
            like,
            min_transition: request.min_transition,
            ..Default::default()
        };
        let plan = crate::set::plan(&self.tracks, start, &options).context("the first track has no tempo")?;
        let ways = plan
            .tracks
            .windows(2)
            .zip(&plan.transitions)
            .map(|(pair, t)| Way::of(t, &self.tracks[pair[0]], &self.tracks[pair[1]], self.ids[pair[1]]))
            .collect();
        Ok(SetPlan {
            tracks: plan.tracks.iter().map(|&i| self.ids[i]).collect(),
            ways,
            seconds: plan.seconds,
            mean_score: plan.mean_score(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transitions::tests::track;

    fn library() -> Mix {
        Mix::new(vec![
            (10, track("a", &[1, 1, 2, 2], false)),
            (20, track("b", &[2, 3, 4, 5], false)),
            (30, track("c", &[9, 9, 9, 9], false)),
            (40, track("d", &[2, 2, 6, 7], false)),
        ])
    }

    #[test]
    fn ways_and_sets_speak_in_provider_ids() {
        let mix = library();
        let ways = mix.next(10, 6.0, 10).unwrap();
        assert!(!ways.is_empty());
        assert!(ways.iter().all(|w| [20, 30, 40].contains(&w.to)));
        // The best way in sounds like the playing track's end.
        assert!([20, 40].contains(&ways[0].to), "{:?}", ways[0]);
        assert!(mix.next(99, 6.0, 10).is_err());

        let request = SetRequest { first: Some(10), minutes: 2.0, min_transition: 0.0, ..Default::default() };
        let plan = mix.set(&request, None).unwrap();
        assert_eq!(plan.tracks[0], 10);
        assert_eq!(plan.ways.len() + 1, plan.tracks.len());
        for (pair, way) in plan.tracks.windows(2).zip(&plan.ways) {
            assert_eq!(way.to, pair[1]);
        }
        assert!(mix.set(&SetRequest::default(), None).is_err(), "neither a first track nor a description");
    }

    #[test]
    fn tracks_added_join_the_library_or_replace_their_own() {
        let mix = library();
        let grown = mix.with(vec![(50, track("e", &[1, 1, 2, 2], false)), (30, track("c2", &[1, 1, 1, 1], false))]);
        assert_eq!(grown.len(), 5);
        assert_eq!(grown.summaries().iter().map(|s| s.id).collect::<Vec<_>>(), [10, 20, 30, 40, 50]);
        assert_eq!(grown.track(30).unwrap().path, std::path::PathBuf::from("c2"));
        assert_eq!(grown.scale, mix.scale, "measured once");
        // The one it came from is as it was.
        assert_eq!(mix.len(), 4);
        assert_eq!(mix.track(30).unwrap().path, std::path::PathBuf::from("c"));
        assert!(grown.next(10, 6.0, 10).unwrap().iter().any(|w| w.to == 50));
    }

    #[test]
    fn a_way_goes_back_to_what_the_renderer_takes() {
        let mix = library();
        let way = &mix.next(10, 6.0, 1).unwrap()[0];
        let transition = way.transition(1);
        assert_eq!((transition.to, transition.out, transition.into), (1, way.out, way.into));
        let next = mix.track(way.to).unwrap();
        assert_eq!(next.segments[way.into].start, way.into_at.0);
    }
}

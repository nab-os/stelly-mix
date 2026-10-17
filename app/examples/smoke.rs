//! What the window's buttons do, run against the real library without the
//! window: the next tracks, a transition rendered, a set planned and
//! rendered, a description searched, and a waveform read.

use stelly_mix_app::work;

fn main() -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    let loaded = work::load()?;
    println!("library: {} tracks in {:.1}s, sound scale {:?}", loaded.tracks.len(), started.elapsed().as_secs_f64(), loaded.scale);

    let first = (0..loaded.tracks.len())
        .find(|&i| loaded.tracks[i].analysis.grid.as_ref().is_some_and(|g| g.steady) && loaded.tracks[i].segments.len() > 6)
        .expect("a steady track");
    let track = &loaded.tracks[first];
    let peaks = work::peaks(&track.path)?;
    println!("waveform of {}: {} points, loudest {:.2}", track.name(), peaks.len(), peaks.iter().cloned().fold(0.0, f32::max));

    let ways = work::next(&loaded, first, 0.06);
    let best = ways.first().expect("somewhere to go");
    println!("next: {} candidates, best {:.2} into {}", ways.len(), best.score, loaded.tracks[best.to].name());

    let t = std::time::Instant::now();
    let clip = work::render_transition(track, &loaded.tracks[best.to], best, &Default::default())?;
    println!("listen: {} ({:.0}s) in {:.1}s, marks {:?}", clip.path.display(), clip.seconds, t.elapsed().as_secs_f64(), clip.marks);

    let t = std::time::Instant::now();
    let plan = work::plan(&loaded, first, &stelly_mix::set::Options { seconds: 6.0 * 60.0, ..Default::default() })?;
    println!("set: {} tracks, mean {:.2}, planned in {:.1}s", plan.tracks.len(), plan.mean_score(), t.elapsed().as_secs_f64());
    let t = std::time::Instant::now();
    let set = work::render_set(&loaded, &plan)?;
    println!("set render: {} ({:.0}s) in {:.1}s, {} entries", set.path.display(), set.seconds, t.elapsed().as_secs_f64(), set.marks.len());

    let t = std::time::Instant::now();
    let like = work::like(&loaded, "dark rolling hard techno")?;
    let top = (0..like.len()).max_by(|&a, &b| like[a].total_cmp(&like[b])).unwrap();
    let t2 = std::time::Instant::now();
    let _ = work::like(&loaded, "soft piano")?;
    println!("find: best {} in {:.1}s, the next search in {:.2}s", loaded.tracks[top].name(), (t2 - t).as_secs_f64(), t2.elapsed().as_secs_f64());
    Ok(())
}

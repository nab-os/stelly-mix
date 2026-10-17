//! How a description ranks the tracks of one folder against the rest of the
//! library: a check that `text::scores` means something.

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let folder = std::path::PathBuf::from(args.next().expect("a folder"));
    let description = args.next().expect("a description");
    let tracks = stelly_mix::library::Library::open(stelly_mix::library::Library::default_dir())?.all()?;
    let mut encoder = stelly_mix::text::TextEncoder::load(&stelly_mix::models::dir())?;
    let like = stelly_mix::text::scores(&mut encoder, &tracks, &description)?;
    let (inside, outside): (Vec<f64>, Vec<f64>) = {
        let (mut i, mut o) = (Vec::new(), Vec::new());
        for (t, l) in tracks.iter().zip(&like) {
            if t.path.starts_with(&folder) { i.push(*l) } else { o.push(*l) }
        }
        (i, o)
    };
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    println!("{description:?}: {} in the folder, mean rank {:.2}; {} elsewhere, mean rank {:.2}", inside.len(), mean(&inside), outside.len(), mean(&outside));
    Ok(())
}

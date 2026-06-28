//! Dump the decoded chords for every sample (run from the workspace root or the
//! crate dir). Reads the git-ignored private sample set when present.
fn main() {
    let dir = ["../private/samples", "private/samples"]
        .into_iter()
        .find(|d| std::path::Path::new(d).exists());
    let Some(dir) = dir else {
        eprintln!("no private/samples directory — nothing to validate");
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map_or(false, |x| x.eq_ignore_ascii_case("mgu") || x.eq_ignore_ascii_case("mid"))
        })
        .collect();
    files.sort();
    for p in files {
        let data = std::fs::read(&p).unwrap();
        match leadsheet::parse(&data) {
            Ok(s) => {
                let chords: Vec<String> = s.chords.iter().take(18).map(|c| c.text.clone()).collect();
                println!(
                    "{:22} {:>3}BPM key={}{}  {}",
                    s.title,
                    s.tempo_bpm,
                    leadsheet::pitch_class_name(s.key_pc),
                    if s.key_minor { "m" } else { "" },
                    chords.join(" ")
                );
            }
            Err(e) => println!("{:?}: {e}", p.file_name().unwrap()),
        }
    }
}

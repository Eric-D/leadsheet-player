//! Generate ORIGINAL (royalty-free) `.MGU` files with the encoder: a bundled
//! demo for the web app and a public test fixture exercising every format case.
//! Chord progressions are not copyrightable, so these are safe to commit.
//! Run: `cargo run -p leadsheet --example gen`

use leadsheet::formats::biab::encode;
use leadsheet::{Chord, Note, Song, PPQ};

fn ch(beat: u32, root: u8, ext: u8, bass: u8) -> Chord {
    Chord {
        bar: (beat / 4) as u16 + 1,
        beat: (beat % 4) as u8,
        tick: beat * PPQ,
        text: String::new(),
        root,
        ext,
        bass,
    }
}

fn build(title: &str, key_pc: u8, minor: bool, tempo: u16, chords: Vec<Chord>,
         markers: Vec<(u16, u8)>, chorus: (u16, u16, u16), melody: Vec<Note>) -> Song {
    let mut s = Song::default();
    s.title = title.into();
    s.key_pc = key_pc;
    s.key_minor = minor;
    s.tempo_bpm = tempo;
    s.form_bars = chorus.1.max(8);
    s.chorus_begin = chorus.0;
    s.chorus_end = chorus.1;
    s.choruses = chorus.2;
    s.part_markers = markers;
    s.chords = chords;
    s.melody = melody;
    s
}

fn note(beat: u32, pitch: u8) -> Note {
    Note { tick: beat * PPQ, pitch, vel: 96, dur: PPQ }
}

fn main() {
    // Bundled demo: an original I–vi–IV–V7 in C, two parts, a simple melody.
    let demo = build(
        "Démo libre", 0, false, 100,
        vec![
            ch(0, 0, 1, 255),  // C
            ch(4, 9, 16, 255), // Am
            ch(8, 5, 1, 255),  // F
            ch(12, 7, 64, 255),// G7
            ch(16, 0, 1, 255), // C
            ch(20, 9, 16, 255),// Am
            ch(24, 5, 1, 255), // F
            ch(28, 7, 64, 255),// G7
        ],
        vec![(1, 1), (5, 2)],
        (1, 8, 2),
        // Melody expanded over the 2 choruses (16 bars) so playback repeats too.
        (0..16).map(|b| note(b * 4, [60, 64, 65, 67, 60, 64, 65, 67][(b % 8) as usize])).collect(),
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    std::fs::write(root.join("web/demo.mgu"), encode(&demo)).unwrap();
    println!("wrote web/demo.mgu ({} bytes)", encode(&demo).len());

    // Public fixture: every decoder case (maj, min, Maj7, 6, m7, 7, slash,
    // multi-chord bar, part A/B). Key G.
    let sampler = build(
        "Sampler", 7, false, 120,
        vec![
            ch(0, 0, 1, 255),  // C
            ch(2, 9, 16, 255), // Am  (2nd chord in bar 1 → multi-chord bar)
            ch(4, 0, 6, 255),  // CMaj7
            ch(8, 2, 5, 255),  // D6
            ch(12, 4, 19, 255),// Em7
            ch(16, 7, 64, 255),// G7
            ch(20, 2, 1, 6),   // D/F#  (slash)
            ch(24, 5, 1, 255), // F
            ch(28, 7, 1, 255), // G
        ],
        vec![(1, 1), (5, 2)],
        (1, 8, 2),
        vec![],
    );
    let fx = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::create_dir_all(&fx).unwrap();
    std::fs::write(fx.join("sampler.mgu"), encode(&sampler)).unwrap();
    println!("wrote leadsheet/tests/fixtures/sampler.mgu");
}

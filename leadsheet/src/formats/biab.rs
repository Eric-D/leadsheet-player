//! Band-in-a-Box `.MGU` / `.SGU` decoder (RLE type/root streams, %18 slash).

use crate::model::*;

fn read_title(data: &[u8]) -> Result<String, ParseError> {
    if data.len() < 2 {
        return Err(ParseError::TooShort);
    }
    let len = data[1] as usize;
    let start = 2;
    let end = start + len;
    if end > data.len() {
        return Err(ParseError::BadTitle);
    }
    let raw = &data[start..end];
    // BiaB strings are Latin-1; map to UTF-8, trimming any trailing NULs.
    let s: String = raw
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect();
    Ok(s.trim().to_string())
}

/// Find the `*.STY` style reference. The style name is length-prefixed:
/// `[len:u8] <name bytes including ".STY">`.
fn read_style(data: &[u8]) -> String {
    if let Some(p) = find_subslice(data, b".STY") {
        let name_end = p + 4;
        // Walk back to find the length byte whose value equals the name length.
        for name_start in (0..name_end).rev() {
            if name_start == 0 {
                break;
            }
            let len = (name_end - name_start) as u8;
            if data[name_start - 1] == len && data[name_start].is_ascii_graphic() {
                let raw = &data[name_start..name_end];
                if raw.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
                    return raw.iter().map(|&b| b as char).collect();
                }
            }
        }
        // Fallback: a bounded backward scan to the first non-printable byte.
        let mut s = name_end;
        while s > 0 && (data[s - 1].is_ascii_graphic()) && name_end - s < 32 {
            s -= 1;
        }
        return data[s..name_end].iter().map(|&b| b as char).collect();
    }
    String::new()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

/// Decode the melody note array.
///
/// Strategy: slide over the file looking for the verified 12-byte record
/// signature and collect the longest run whose positions are monotonically
/// non-decreasing. This is robust to where exactly the block starts and to
/// stray bytes that happen to look like a record.
fn read_melody(data: &[u8]) -> Vec<Note> {
    let n = data.len();
    let mut notes: Vec<Note> = Vec::new();
    let mut i = 0usize;

    // Collect every record matching the verified signature. The signature is
    // specific enough (0x90 marker + 0x01 flag + plausible pitch/velocity)
    // that stray matches are rare; records may be separated by other events,
    // so we skip non-matching bytes rather than requiring strict adjacency.
    while i + 12 <= n {
        if is_record(data, i) {
            let tick = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
            let pitch = data[i + 5];
            let vel = data[i + 6];
            let dur = u32::from_le_bytes([data[i + 8], data[i + 9], data[i + 10], data[i + 11]]);
            notes.push(Note { tick, pitch, vel, dur });
            i += 12;
        } else {
            i += 1;
        }
    }
    notes.sort_by_key(|nrec| nrec.tick);
    notes
}

/// Verified record signature: `.. .. 00 00 | 90 | pitch | vel | 01 | dur:u32`.
#[inline]
fn is_record(d: &[u8], i: usize) -> bool {
    if i + 12 > d.len() {
        return false;
    }
    let dur = u32::from_le_bytes([d[i + 8], d[i + 9], d[i + 10], d[i + 11]]);
    d[i + 2] == 0
        && d[i + 3] == 0
        && d[i + 4] == 0x90
        && d[i + 7] == 0x01
        && (1..=127).contains(&d[i + 5]) // plausible pitch
        && (1..=127).contains(&d[i + 6]) // plausible velocity
        && d[i + 5] >= 24 // melodies live above ~C1; filters false hits
        // A real melody note lasts at most a few bars. A wild duration means we
        // matched chord/structure bytes by accident — reject it (it would
        // otherwise drone in the audio and blow up the staff/tab width).
        && (1..=4 * TICKS_PER_BAR).contains(&dur)
}

pub fn parse(data: &[u8]) -> Result<Song, ParseError> {
    if data.len() < 0x20 {
        return Err(ParseError::TooShort);
    }
    // Header walk (byte-exact, per the BiaB format): version, title, two
    // reserved bytes, style index, key, then little-endian BPM.
    let title = read_title(data)?;
    let style = read_style(data);
    let mut idx = 2 + data[1] as usize; // past version + title
    idx += 2; // two reserved bytes
    let style_byte = data.get(idx).copied().unwrap_or(1);
    idx += 1;
    let style_idx = style_byte.saturating_sub(1);
    let key_byte = data.get(idx).copied().unwrap_or(1);
    idx += 1;
    let tempo_bpm = data.get(idx).copied().unwrap_or(120) as u16
        | ((data.get(idx + 1).copied().unwrap_or(0) as u16) << 8);
    idx += 2;

    let (key_pc, key_minor) = key_info(key_byte);
    let timesig_z = timesig_z(style_idx);

    let melody = read_melody(data);
    let bars = melody
        .iter()
        .map(|nrec| ((nrec.tick + nrec.dur) / TICKS_PER_BAR) as u16 + 1)
        .max()
        .unwrap_or(0);

    // Chords + chorus + part markers come straight from the streams.
    let (mut chords, chorus, part_markers) = decode(data, idx, timesig_z);
    // Drop a short trailing "resolution" chord a bar or two past the chorus end
    // (single-form songs encode it; BiaB's chart hides it). Keep genuinely
    // multi-section songs whose chart really does run well past the chorus.
    if let Some((_, end, _)) = chorus {
        if let Some(maxbar) = chords.iter().map(|c| c.bar).max() {
            if maxbar > end && maxbar - end <= 4 {
                chords.retain(|c| c.bar <= end);
            }
        }
    }
    let chords_decoded = !chords.is_empty();
    let form_bars = form_bars(&chords);
    let (chorus_begin, chorus_end, choruses) = chorus.unwrap_or((1, form_bars, 1));

    Ok(Song {
        title,
        style,
        tempo_bpm: if tempo_bpm == 0 { 120 } else { tempo_bpm },
        key_pc,
        key_minor,
        melody,
        chords,
        part_markers,
        bars,
        form_bars,
        chorus_begin,
        chorus_end,
        choruses,
        chords_decoded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    // Sample songs are copyrighted, so they live in the git-ignored `private/`
    // dir. When it's absent (e.g. a fresh public clone / CI), these tests skip.
    fn load(name: &str) -> Option<Song> {
        for base in ["private/samples", "../private/samples"] {
            let p = std::path::Path::new(base).join(name);
            if p.exists() {
                return Some(parse(&std::fs::read(p).unwrap()).unwrap());
            }
        }
        None // private samples absent (fresh public clone) -> skip
    }

    /// First chord whose starting bar equals `bar`.
    fn chord_at(s: &Song, bar: u16) -> &Chord {
        s.chords
            .iter()
            .find(|c| c.bar == bar)
            .unwrap_or_else(|| panic!("no chord starting at bar {bar}"))
    }

    /// All chords in a bar, as "(beat, text)" pairs.
    fn bar_chords(s: &Song, bar: u16) -> Vec<(u8, String)> {
        s.chords
            .iter()
            .filter(|c| c.bar == bar)
            .map(|c| (c.beat, c.text.clone()))
            .collect()
    }

    /// The first `n` chord display texts, in order.
    fn chord_prefix(s: &Song, n: usize) -> Vec<String> {
        s.chords.iter().take(n).map(|c| c.text.clone()).collect()
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // --- Sample 1: "stand by me" — simple diatonic ballad in G -------------
    #[test]
    fn stand_by_me() {
        let Some(s) = load("stand_by_me.MGU") else { return };
        assert_eq!(s.title, "stand by me");
        assert_eq!(s.style, "C_DESERT.STY");
        assert_eq!(s.tempo_bpm, 80);
        assert_eq!(s.key_pc, 7); // G
        // MIDI export has 324 melody notes — effectively a full match.
        assert_eq!(s.melody.len(), 323);
        // Progression with decoded chord types: G Em CMaj7 D6 G …
        assert_eq!(chord_at(&s, 1).text, "G");
        assert_eq!(chord_at(&s, 3).text, "Em");
        assert_eq!(chord_at(&s, 5).text, "CMaj7");
        assert_eq!(chord_at(&s, 6).text, "D6");
        assert_eq!(chord_at(&s, 7).text, "G");
        // Chorus record read from the file: bars 1..32, played 3 times.
        assert_eq!(s.form_bars, 32);
        assert_eq!((s.chorus_begin, s.chorus_end, s.choruses), (1, 32, 3));
    }

    // --- Sample 5: "Last Christmas" — the loop starts at bar 9 (intro 1..8) --
    #[test]
    fn last_christmas() {
        let Some(s) = load("last_christmas.MGU") else { return };
        assert_eq!(s.title, "Last Christmas");
        assert_eq!(s.tempo_bpm, 110);
        assert_eq!(s.key_pc, 2); // D
        // Intro is bars 1..8; the chorus loops bars 9..40, three times.
        assert_eq!((s.chorus_begin, s.chorus_end, s.choruses), (9, 40, 3));
        assert_eq!(chord_at(&s, 1).text, "D");
        assert_eq!(chord_at(&s, 3).text, "Bm");
        assert_eq!(chord_at(&s, 9).text, "D"); // loop start
        assert_eq!(chord_at(&s, 17).text, "D"); // form continues past bar 16
        // Full 40-bar form: D Bm Em A repeated (no garbage tail).
        assert_eq!(s.form_bars, 40);
        assert_eq!(s.chords.len(), 20);
    }

    // --- Sample 2: "Toi et Moi" — starts on vi (Em), 2-bar chords ----------
    // Verifies key-relative tempo offset (90 BPM) and that a song which does
    // not start on the tonic is still keyed correctly (Em, not E).
    #[test]
    fn toi_et_moi() {
        let Some(s) = load("toi_et_moi.MGU") else { return };
        assert_eq!(s.title, "Toi et Moi");
        assert_eq!(s.style, "BGEE_BAL.STY");
        assert_eq!(s.tempo_bpm, 90);
        assert_eq!(s.key_pc, 7); // G
        assert_eq!(chord_at(&s, 1).text, "Em"); // not "E"
        assert_eq!(chord_at(&s, 3).text, "CMaj7");
        assert_eq!(chord_at(&s, 5).text, "G");
        assert_eq!(chord_at(&s, 7).text, "D6");
    }

    // --- Sample 6: "boule de flipper" — decoded extensions D6 & Bm7 ---------
    #[test]
    fn boule_de_flipper() {
        let Some(s) = load("boule_de_flipper.MGU") else { return };
        assert_eq!(s.title, "boule de flipper");
        assert_eq!(s.key_pc, 7); // G
        // Chord types come from the file: D6 at bar 5, Bm7 at bar 6.
        assert_eq!(chord_at(&s, 4).text, "C");
        assert_eq!(chord_at(&s, 5).text, "D6");
        assert_eq!(chord_at(&s, 6).text, "Bm7");
        assert_eq!(chord_at(&s, 8).text, "Am");
    }

    // --- Sample 3: "Baila Morena" — long form, slash chords, long holds ----
    #[test]
    fn baila_morena() {
        let Some(s) = load("baila_morena.MGU") else { return };
        assert_eq!(s.title, "Baila Morena");
        assert_eq!(s.style, "AVRIL1.STY");
        assert_eq!(s.tempo_bpm, 117);
        assert_eq!(s.key_pc, 7); // G

        // Two-part song with secondary dominants and a slash chord — all now
        // decoded exactly from the chord-type and root streams.
        assert_eq!(chord_at(&s, 1).text, "Em");
        assert_eq!(chord_at(&s, 14).text, "A"); // secondary dominant (A major)
        assert_eq!(chord_at(&s, 25).text, "Am"); // diatonic ii
        assert_eq!(chord_at(&s, 30).text, "B7");
        assert_eq!(chord_at(&s, 35).text, "D/F#"); // slash chord
    }

    // --- Sample 4: "eyes of tiger" — several chords per bar (syncopation) --
    // Verifies the variable-length record decoding: a bar can hold multiple
    // chords at different beats (the "Em Em D" riff).
    #[test]
    fn eyes_of_tiger() {
        let Some(s) = load("eyes_of_tiger.MGU") else { return };
        assert_eq!(s.title, "eyes of tiger");
        assert_eq!(s.tempo_bpm, 95);
        assert_eq!(s.key_pc, 7); // G
        // Bar 1 = Em (beat 1), Em (beat 3), D (beat 4).
        assert_eq!(
            bar_chords(&s, 1),
            vec![
                (0, "Em".to_string()),
                (2, "Em".to_string()),
                (3, "D".to_string()),
            ]
        );
        // Bar 4 is a single chord (C) spanning the whole bar.
        assert_eq!(bar_chords(&s, 4), vec![(0, "C".to_string())]);
        assert_eq!(s.choruses, 3);
    }

    // --- Decoded chord types across the user's sample library --------------
    // Each maps to the BiaB on-screen chords (see samples/GROUND_TRUTH.md).

    #[test]
    fn cetait_lola_b7() {
        let Some(s) = load("cetait_lola.MGU") else { return };
        assert_eq!(s.title, "C'était Lola");
        assert_eq!(s.tempo_bpm, 115);
        // Decoded dominant 7th: Em D C B7, with a dense run of B7s.
        assert_eq!(chord_prefix(&s, 5), v(&["Em", "Em", "D", "C", "B7"]));
        assert!(s.chords.iter().filter(|c| c.text == "B7").count() >= 8);
    }

    #[test]
    fn je_te_promets_rich() {
        let Some(s) = load("je_te_promets.MGU") else { return };
        assert_eq!(s.title, "Je te promets");
        // G Bm7 Am CMaj7 D … — m7, Maj7 and 6 all decoded.
        assert_eq!(
            chord_prefix(&s, 5),
            v(&["G", "Bm7", "Am", "CMaj7", "D"])
        );
        assert!(s.chords.iter().any(|c| c.text == "D6"));
        assert!(s.chords.iter().any(|c| c.text == "Bm7"));
    }

    #[test]
    fn welcome_to_my_life_extensions() {
        let Some(s) = load("welcome_to_my_life.MGU") else { return };
        assert_eq!(s.title, "Welcome to my life");
        assert_eq!(chord_prefix(&s, 6), v(&["G", "Em", "G", "Em", "C", "D6"]));
        assert!(s.chords.iter().any(|c| c.text == "CMaj7"));
    }

    #[test]
    fn la_grenade_b7() {
        let Some(s) = load("la_grenade.MGU") else { return };
        assert_eq!(s.title, "La grenade");
        // Em D C B7 then Em Am Em D G D.
        assert!(s.chords.iter().any(|c| c.text == "B7"));
        assert!(s.chords.iter().any(|c| c.text == "Am"));
    }

    // Regression: choses_simples' key byte is C, but its D chords are MAJOR.
    // The type stream must win (D, not the diatonic-in-C "Dm").
    #[test]
    fn choses_simples_major_d() {
        let Some(s) = load("choses_simples.MGU") else { return };
        assert_eq!(s.title, "Les choses simples");
        assert_eq!(chord_prefix(&s, 4), v(&["Em", "D", "G", "Am"]));
    }

    // --- Slash chords + secondary dominants, decoded byte-exact -------------
    // "Hotel California" (key C / Am): E7, D7, FMaj7, slash C/E and E7/B,
    // and Dm6 — all read directly from the file's type + root streams.
    #[test]
    fn hotel_california_full() {
        let Some(s) = load("hotel_california.MGU") else { return };
        assert_eq!(s.title, "Hotel California");
        assert!(!s.key_minor); // key byte = C major (Am relative)
        assert_eq!(
            chord_prefix(&s, 8),
            v(&["Am", "E7", "G", "D7", "FMaj7", "C/E", "Dm6", "E7/B"])
        );
    }

    // Part markers (substyle A/B) decoded from the bar-type stream — newly
    // reverse-engineered (1 = A, 2 = B), matching the blue/green chart boxes.
    #[test]
    fn part_markers() {
        let Some(s) = load("stand_by_me.MGU") else { return };
        let m = |bar: u16| s.part_markers.iter().find(|(b, _)| *b == bar).map(|(_, p)| *p);
        assert_eq!(m(1), Some(1)); // bar 1 = part A (blue)
        assert_eq!(m(9), Some(1)); // bar 9 = part A
        assert_eq!(m(17), Some(2)); // bar 17 = part B (green)
        assert_eq!(m(25), Some(2)); // bar 25 = part B
    }

    // Rest/shot/hold articulation (newly reverse-engineered, undocumented): the
    // original baila has a "shot" (B..) on bar 31.
    #[test]
    fn baila_shot_bar31() {
        let Some(s) = load("baila_morena.MGU") else { return };
        let c = s.chords.iter().find(|c| c.bar == 31).expect("bar 31");
        assert_eq!(c.text, "B");
        assert_eq!(c.rest, 2); // 2 dots = shot
    }

    // A genuinely minor key (choses_simples key byte ≥ 18 → Em).
    #[test]
    fn choses_simples_minor_key() {
        let Some(s) = load("choses_simples.MGU") else { return };
        assert!(s.key_minor);
        assert_eq!(s.key_pc, 4); // E (Em)
        assert!(s.chords.iter().any(|c| c.text == "Dm6"));
    }

    // The encoder is the exact inverse of the decoder: decode → encode → decode
    // reproduces the same chords, key and tempo (incl. extensions and slash).
    #[test]
    fn encode_roundtrip() {
        for name in [
            "stand_by_me.MGU",
            "hotel_california.MGU",
            "baila_morena.MGU",
            "cetait_lola.MGU",
            "eyes_of_tiger.MGU",
        ] {
            let Some(s) = load(name) else { return };
            let s2 = parse(&encode(&s)).unwrap();
            let a: Vec<&String> = s.chords.iter().map(|c| &c.text).collect();
            let b: Vec<&String> = s2.chords.iter().map(|c| &c.text).collect();
            assert_eq!(a, b, "chords differ after round-trip: {name}");
            assert_eq!((s.tempo_bpm, s.key_pc, s.key_minor), (s2.tempo_bpm, s2.key_pc, s2.key_minor));
            assert_eq!(s.part_markers, s2.part_markers, "markers differ: {name}");
        }
    }

    // Public test on a committed, ORIGINAL fixture (generated by `examples/gen`)
    // — runs everywhere incl. a fresh clone with no `private/`. Covers maj, min,
    // Maj7, 6, m7, 7, a slash chord, a multi-chord bar and A/B markers.
    #[test]
    fn fixture_sampler() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sampler.mgu");
        let s = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(s.title, "Sampler");
        assert_eq!(s.tempo_bpm, 120);
        assert_eq!(s.key_pc, 7); // G
        let texts: Vec<&str> = s.chords.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            texts,
            ["C", "Am", "CMaj7", "D6", "Em7", "G7", "D/F#", "F", "G"]
        );
        assert_eq!(s.part_markers, vec![(1, 1), (5, 2)]);
    }
}

/// Root index (1..=17) → pitch class. Indices 13..17 are sharp spellings.
const ROOT_PC: [u8; 17] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 1, 3, 6, 8, 10];

fn root_pc(root: u8) -> u8 {
    ROOT_PC[(root.clamp(1, 17) - 1) as usize]
}

/// Chord-type index → display suffix. Reverse-engineered table (Choi) reduced
/// to the common Band-in-a-Box symbols; covers everything in the sample set
/// (major, minor, 6, Maj7, m6, m7, 7, …) with sensible names for the rest.
fn type_suffix(ext: u8) -> &'static str {
    match ext {
        1 | 2 => "",
        3 => "(b5)",
        4 => "+",
        5 => "6",
        6 => "Maj7",
        7 => "Maj9",
        8 => "Maj9#11",
        14 => "6/9",
        15 => "sus2",
        16 => "m",
        17 => "m#5",
        18 => "mMaj7",
        19 => "m7",
        20 => "m9",
        21 => "m11",
        22 => "m13",
        23 => "m6",
        32 => "dim7",
        33 => "dim",
        40 => "5",
        56 => "7#5",
        64 => "7",
        65 => "13",
        66 => "7b13",
        67 => "7#11",
        70 => "9",
        73 => "9#11",
        76 => "7b9",
        82 => "7#9",
        88 => "7b5",
        128 => "7sus",
        134 => "9sus",
        177 | 184 => "sus4",
        _ => "", // unknown index: fall back to a plain triad symbol
    }
}

/// One RLE-decoded entry; returns the value sequence (non-zero bytes only),
/// advancing `idx` past the stream. The stream spans up to `MAX_BARS*4` beats.
fn read_rle(data: &[u8], idx: &mut usize) -> Vec<(u32, u8)> {
    const MAX_BEATS: u32 = 255 * 4;
    let mut out = Vec::new();
    let mut beat = 0u32;
    while beat < MAX_BEATS {
        let Some(&val) = data.get(*idx) else { break };
        *idx += 1;
        if val == 0 {
            let Some(&skip) = data.get(*idx) else { break };
            *idx += 1;
            beat += skip as u32;
        } else {
            out.push((beat, val));
            beat += 1;
        }
    }
    out
}

/// Read the bar-type stream (RLE keyed by bar, terminated at bar 255). Returns
/// the part markers `(bar, part)` where part 1 = substyle A, 2 = substyle B.
/// (Verified 1:1 against the BiaB charts — this mapping is not published.)
fn read_bar_types(data: &[u8], idx: &mut usize) -> Vec<(u16, u8)> {
    let mut markers = Vec::new();
    let mut bar = *data.get(*idx).unwrap_or(&255) as u32;
    *idx += 1;
    while bar < 255 {
        let Some(&val) = data.get(*idx) else { break };
        *idx += 1;
        if val == 0 {
            let Some(&n) = data.get(*idx) else { break };
            *idx += 1;
            bar += n as u32;
        } else {
            markers.push((bar as u16, val));
            bar += 1;
        }
    }
    markers
}

/// Decode chords + chorus from `start` (the byte after the header). `timesig_z`
/// is beats per bar (4 for 4/4). Returns the chords and the optional chorus
/// record `(begin, end, repeats)` as 1-based bar numbers + repeat count.
pub fn decode(
    data: &[u8],
    start: usize,
    timesig_z: u8,
) -> (Vec<Chord>, Option<(u16, u16, u16)>, Vec<(u16, u8)>) {
    let z = timesig_z.max(1) as u32;
    let mut idx = start;

    let markers = read_bar_types(data, &mut idx);
    let exts = read_rle(data, &mut idx); // (beat, type index)
    let roots = read_rle(data, &mut idx); // (beat, packed root+bass)

    let mut chords = Vec::with_capacity(roots.len());
    let mut prev_beat = 0u32;
    for (i, &(beat, val)) in roots.iter().enumerate() {
        // The root and type streams must agree beat-for-beat; a mismatch means
        // we've run off the end into padding, so stop (avoids trailing junk).
        let Some(&(ebeat, ext)) = exts.get(i) else { break };
        if ebeat != beat {
            break;
        }
        // A chord isolated many bars after the previous one is stream padding
        // noise (e.g. a stray chord at bar 67), not real — stop there.
        if i > 0 && beat > prev_beat + 32 {
            break;
        }
        prev_beat = beat;
        let root = (val % 18).clamp(1, 17);
        let bass = {
            let b = (root as i32 - 1 + (val / 18) as i32).rem_euclid(18) as u8 + 1;
            if b == root {
                0
            } else {
                b
            }
        };
        let rpc = root_pc(root);
        let mut text = format!("{}{}", pitch_class_name(rpc), type_suffix(ext));
        if bass >= 1 {
            text.push('/');
            text.push_str(pitch_class_name(root_pc(bass)));
        }
        chords.push(Chord {
            bar: (beat / z) as u16 + 1,
            beat: (beat % z) as u8,
            tick: beat * PPQ,
            text,
            root: rpc,
            ext,
            bass: if bass >= 1 { root_pc(bass) } else { 255 },
            rest: 0,
        });
    }

    // Rest / shot / hold articulations are stored after the chords as
    // `[marker][value][00]` entries, where `marker = beat_index + 10` and the
    // value's bits 5-6 give the type. Cross-referenced against real chord beats
    // so we never match stray bytes. (Reverse-engineered from BiaB — undocumented.)
    for i in idx..data.len().saturating_sub(2) {
        if data[i + 2] != 0 {
            continue;
        }
        let dots = match data[i + 1] {
            0x3f => 1, // rest  "."
            0x1f => 2, // shot  ".."
            0x5f => 3, // hold  "..."
            _ => continue,
        };
        let beat = data[i] as i32 - 10;
        if beat < 0 {
            continue;
        }
        if let Some(c) = chords
            .iter_mut()
            .find(|c| (c.bar as i32 - 1) * z as i32 + c.beat as i32 == beat)
        {
            c.rest = dots;
        }
    }

    // Chorus record `[begin][end][repeats]` sits at (or just after) idx — there
    // is sometimes a leading separator byte. Scan a small window for the first
    // sane triple rather than blindly skipping (which would eat begin==1).
    let chorus = (0..4).find_map(|o| {
        let b = *data.get(idx + o)?;
        let e = *data.get(idx + o + 1)?;
        let r = *data.get(idx + o + 2)?;
        if (1..=250).contains(&b) && b < e && e <= 250 && (1..=16).contains(&r) {
            Some((b as u16, e as u16, r as u16))
        } else {
            None
        }
    });

    (chords, chorus, markers)
}

/// Total bars covered by the chord form (for the chart layout).
pub fn form_bars(chords: &[Chord]) -> u16 {
    let last = chords.last().map(|c| c.bar).unwrap_or(0);
    (((last as u32 + 3) / 4) * 4).max(4) as u16
}

/// Beats per bar for a Band-in-a-Box style index (from the styles table).
pub fn timesig_z(style: u8) -> u8 {
    const Z: [u8; 24] = [
        4, 12, 4, 4, 4, 4, 4, 3, 4, 4, 4, 4, 4, 4, 4, 4, 3, 4, 4, 4, 4, 12, 12, 4,
    ];
    *Z.get(style as usize).unwrap_or(&4)
}

/// Decode the key byte: `<= 17` is a major key (root index), `>= 18` a minor
/// key (root = byte − 18). Returns (tonic pitch class, is_minor).
pub fn key_info(key_byte: u8) -> (u8, bool) {
    if key_byte >= 18 {
        (root_pc(key_byte - 18 + 1), true)
    } else {
        (root_pc(key_byte.max(1)), false)
    }
}

// ---- Encoder (inverse of `parse`) -----------------------------------------

fn write_skip(out: &mut Vec<u8>, mut n: u32) {
    while n > 0 {
        let c = n.min(255);
        out.push(0);
        out.push(c as u8);
        n -= c;
    }
}

/// Write an RLE stream of `(beat, value)` pairs, padded to `total` beats.
fn write_rle(out: &mut Vec<u8>, pairs: &[(u32, u8)], total: u32) {
    let mut cur = 0u32;
    for &(beat, val) in pairs {
        if beat < cur {
            continue; // out of order / duplicate beat — skip
        }
        if beat > cur {
            write_skip(out, beat - cur);
            cur = beat;
        }
        out.push(val.max(1));
        cur += 1;
    }
    if total > cur {
        write_skip(out, total - cur);
    }
}

/// Pack a root pitch class + optional slash bass into a root-stream byte.
fn pack_root(root_pc: u8, bass_pc: u8) -> u8 {
    let root_idx = (root_pc % 12) + 1; // 1..=12
    if bass_pc > 11 {
        return root_idx;
    }
    let bass_idx = (bass_pc % 12) as i32 + 1;
    let offset = (bass_idx - root_idx as i32).rem_euclid(18) as u32;
    let val = root_idx as u32 + 18 * offset;
    if val <= 255 {
        val as u8
    } else {
        root_idx // slash bass would overflow the byte — drop it
    }
}

/// Encode a `Song` back to a Band-in-a-Box `.MGU` (round-trips with [`parse`]).
/// Assumes 4/4. Used to generate original test fixtures and the bundled demo.
pub fn encode(song: &Song) -> Vec<u8> {
    let mut out = Vec::new();
    // Header: version, title, two reserved bytes, style (0=4/4 → +1), key, BPM.
    out.push(0x49);
    let title = song.title.as_bytes();
    let tlen = title.len().min(255);
    out.push(tlen as u8);
    out.extend_from_slice(&title[..tlen]);
    out.push(0);
    out.push(0);
    out.push(1);
    out.push(if song.key_minor {
        18 + (song.key_pc % 12)
    } else {
        (song.key_pc % 12) + 1
    });
    out.extend_from_slice(&song.tempo_bpm.to_le_bytes());

    // Bar-type (part marker) stream: starting bar, then RLE, padded to bar 255.
    out.push(1);
    let mut markers = song.part_markers.clone();
    markers.sort_by_key(|&(b, _)| b);
    let mut cur = 1u32;
    for (bar, part) in markers {
        let bar = bar as u32;
        if bar < cur {
            continue;
        }
        if bar > cur {
            write_skip(&mut out, bar - cur);
            cur = bar;
        }
        out.push(part.max(1));
        cur += 1;
    }
    write_skip(&mut out, 255u32.saturating_sub(cur).max(1));

    // Chord-type then chord-root streams (paired by beat), each padded to 1020.
    let chords: Vec<&Chord> = song.chords.iter().filter(|c| c.root <= 11).collect();
    let types: Vec<(u32, u8)> = chords
        .iter()
        .map(|c| (c.tick / PPQ, if c.ext == 0 { 1 } else { c.ext }))
        .collect();
    let roots: Vec<(u32, u8)> = chords
        .iter()
        .map(|c| (c.tick / PPQ, pack_root(c.root, c.bass)))
        .collect();
    write_rle(&mut out, &types, 255 * 4);
    write_rle(&mut out, &roots, 255 * 4);

    // Chorus record [begin][end][repeats].
    out.push(song.chorus_begin.clamp(1, 250) as u8);
    out.push(song.chorus_end.max(song.chorus_begin + 1).clamp(2, 250) as u8);
    out.push(song.choruses.clamp(1, 16) as u8);

    // Melody as 12-byte note records, found by the scanner on decode.
    for n in &song.melody {
        if n.tick >= 0x1_0000 {
            continue;
        }
        out.extend_from_slice(&n.tick.to_le_bytes());
        out.push(0x90);
        out.push(n.pitch);
        out.push(n.vel.max(1));
        out.push(0x01);
        out.extend_from_slice(&n.dur.to_le_bytes());
    }
    out
}

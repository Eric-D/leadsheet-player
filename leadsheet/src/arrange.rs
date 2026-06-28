//! Combine a [`Song`] (what to play) with a [`Style`] (how to play it) into a
//! flat list of playable note events. This is the seam the audio engine can
//! consume instead of computing accompaniment itself.

use crate::model::{Song, TICKS_PER_BAR};
use crate::style::Style;

/// Chord-tone intervals (from the root) for a Band-in-a-Box chord-type index.
/// Picks the right 3rd/5th so minor chords don't sound a major 3rd (the cause
/// of the clash on minor-key songs like "eyes of the tiger").
fn voicing(ext: u8) -> &'static [u8] {
    match ext {
        32 | 33 => &[0, 3, 6],                              // dim / dim7
        16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 => &[0, 3, 7], // minor family (m, m6, m7, …)
        4 => &[0, 4, 8],                                    // augmented
        _ => &[0, 4, 7],                                    // major / dominant / default
    }
}

/// Active substyle (1 = A, 2 = B) at `bar`, from the decoded part markers.
fn part_at(song: &Song, bar: u16) -> u8 {
    let mut cur = 1u8;
    for &(b, p) in &song.part_markers {
        if b <= bar {
            cur = p;
        } else {
            break;
        }
    }
    cur
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Melody,
    Bass,
    Comp,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub tick: u32,
    pub pitch: u8,
    pub vel: u8,
    pub dur: u32,
    pub instrument: u8,
    pub part: Part,
}

/// Produce events: the melody as-is, plus bass + comped chords on the style's
/// pattern beats. (Skeleton: comps a plain triad; quality-aware voicings later.)
pub fn arrange(song: &Song, style: &Style) -> Vec<Event> {
    let beat = TICKS_PER_BAR / 4;
    let mut out = Vec::new();

    for n in &song.melody {
        out.push(Event {
            tick: n.tick,
            pitch: n.pitch,
            vel: n.vel,
            dur: n.dur,
            instrument: 0,
            part: Part::Melody,
        });
    }

    for (i, c) in song.chords.iter().enumerate() {
        if c.root > 11 {
            continue; // slash-bass marker, not a chord
        }
        let start = c.tick;
        // A chord lasts until the next one (or one bar for the last). Staying
        // within this span means syncopated bars (several chords) never overlap.
        let end = song
            .chords
            .get(i + 1)
            .map(|n| n.tick)
            .unwrap_or(start + TICKS_PER_BAR)
            .max(start + beat);
        let ivs = voicing(c.ext); // quality-aware: minor 3rd for minor chords, etc.

        let mut emit = |t: u32, onset: bool| {
            let bar_no = (t / TICKS_PER_BAR) as u16 + 1;
            let beat_in_bar = ((t / beat) % 4) as u8;
            // Substyle B comps on every beat (busier); A uses the style pattern.
            let comp_on = part_at(song, bar_no) == 2 || style.comp.beats.contains(&beat_in_bar);
            let bass_on = style.bass.beats.contains(&beat_in_bar);
            // Always voice the chord at its onset, so every chord change sounds
            // (crucial for syncopation); fill the rest on the pattern beats.
            if onset || bass_on {
                out.push(Event { tick: t, pitch: 36 + c.root, vel: 90, dur: beat, instrument: style.bass.instrument, part: Part::Bass });
            }
            if onset || comp_on {
                for &iv in ivs {
                    out.push(Event { tick: t, pitch: 48 + c.root + iv, vel: 55, dur: beat, instrument: style.comp.instrument, part: Part::Comp });
                }
            }
        };

        emit(start, true);
        let mut t = (start / beat + 1) * beat; // next beat slot
        while t < end {
            emit(t, false);
            t += beat;
        }
    }

    out.sort_by_key(|e| e.tick);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Chord, Song};

    #[test]
    fn minor_chord_voices_minor_third_not_major() {
        let mut s = Song::default();
        s.chords = vec![Chord { bar: 1, beat: 0, tick: 0, text: "Em".into(), root: 4, ext: 16, bass: 255, rest: 0 }];
        let comp: Vec<u8> = arrange(&s, &Style::default())
            .into_iter()
            .filter(|e| e.part == Part::Comp)
            .map(|e| e.pitch)
            .collect();
        assert!(comp.contains(&55), "Em must comp G (minor 3rd, 48+4+3)");
        assert!(!comp.contains(&56), "Em must NOT comp G# (major 3rd, 48+4+4)");
    }
}

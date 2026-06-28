//! Combine a [`Song`] (what to play) with a [`Style`] (how to play it) into a
//! flat list of playable note events. This is the seam the audio engine can
//! consume instead of computing accompaniment itself.

use crate::model::{Song, TICKS_PER_BAR};
use crate::style::Style;

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
        let next = song
            .chords
            .get(i + 1)
            .map(|n| n.tick)
            .unwrap_or(c.tick + TICKS_PER_BAR);
        let bars = (next.saturating_sub(c.tick) / TICKS_PER_BAR).max(1);
        for b in 0..bars {
            let bar_tick = c.tick + b * TICKS_PER_BAR;
            let bar_no = (bar_tick / TICKS_PER_BAR) as u16 + 1;
            // Substyle B comps on every beat (busier); A uses the style pattern.
            let comp_beats: &[u8] = if part_at(song, bar_no) == 2 {
                &[0, 1, 2, 3]
            } else {
                &style.comp.beats
            };
            for &bt in &style.bass.beats {
                out.push(Event {
                    tick: bar_tick + bt as u32 * beat,
                    pitch: 36 + c.root,
                    vel: 90,
                    dur: beat,
                    instrument: style.bass.instrument,
                    part: Part::Bass,
                });
            }
            for &bt in comp_beats {
                for iv in [0u8, 4, 7] {
                    out.push(Event {
                        tick: bar_tick + bt as u32 * beat,
                        pitch: 48 + c.root + iv,
                        vel: 55,
                        dur: beat,
                        instrument: style.comp.instrument,
                        part: Part::Comp,
                    });
                }
            }
        }
    }

    out.sort_by_key(|e| e.tick);
    out
}

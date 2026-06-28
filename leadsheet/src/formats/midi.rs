//! Minimal Standard MIDI File reader → the common model. Decodes tempo, an
//! optional title, and note events (with durations) into `Song.melody`. Chords
//! are left empty (a future pass could infer them). Enough to prove the
//! multi-format architecture and to play imported `.mid` files.

use crate::model::{Note, ParseError, Song, PPQ, TICKS_PER_BAR};

fn be16(b: &[u8]) -> u16 {
    ((b[0] as u16) << 8) | b[1] as u16
}
fn be32(b: &[u8]) -> u32 {
    ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | b[3] as u32
}

/// Read a variable-length quantity at `p`, advancing it.
fn vlq(data: &[u8], p: &mut usize, end: usize) -> u32 {
    let mut v = 0u32;
    while *p < end {
        let c = data[*p];
        *p += 1;
        v = (v << 7) | (c & 0x7f) as u32;
        if c & 0x80 == 0 {
            break;
        }
    }
    v
}

fn close_note(active: &mut Vec<(u8, u32)>, melody: &mut Vec<Note>, pitch: u8, vel: u8, end_tick: u32) {
    if let Some(i) = active.iter().rposition(|&(p, _)| p == pitch) {
        let (_, start) = active.remove(i);
        melody.push(Note {
            tick: start,
            pitch,
            vel: vel.max(1),
            dur: end_tick.saturating_sub(start).max(1),
        });
    }
}

pub fn parse(data: &[u8]) -> Result<Song, ParseError> {
    if data.len() < 14 || &data[0..4] != b"MThd" {
        return Err(ParseError::Format("not a MIDI file"));
    }
    let ntrk = be16(&data[8..10]);
    let division = (be16(&data[12..14]) & 0x7fff).max(1) as u32; // ticks per quarter
    let mut song = Song::default();
    song.tempo_bpm = 120;

    let mut idx = 14usize;
    for _ in 0..ntrk {
        if idx + 8 > data.len() || &data[idx..idx + 4] != b"MTrk" {
            break;
        }
        let len = be32(&data[idx + 4..idx + 8]) as usize;
        let end = (idx + 8 + len).min(data.len());
        let mut p = idx + 8;
        let mut tick = 0u32;
        let mut status = 0u8;
        let mut active: Vec<(u8, u32)> = Vec::new(); // (pitch, start tick@120ppq)
        while p < end {
            tick += vlq(data, &mut p, end);
            if p >= end {
                break;
            }
            if data[p] & 0x80 != 0 {
                status = data[p];
                p += 1;
            }
            let t = tick * PPQ / division;
            match status & 0xF0 {
                0xFF => {
                    let meta = data.get(p).copied().unwrap_or(0);
                    p += 1;
                    let l = vlq(data, &mut p, end) as usize;
                    if meta == 0x51 && l == 3 && p + 3 <= end {
                        let us = ((data[p] as u32) << 16) | ((data[p + 1] as u32) << 8) | data[p + 2] as u32;
                        if us > 0 {
                            song.tempo_bpm = (60_000_000 / us) as u16;
                        }
                    }
                    if meta == 0x03 && song.title.is_empty() && p + l <= end {
                        song.title = data[p..p + l].iter().map(|&b| b as char).collect::<String>().trim().to_string();
                    }
                    p += l;
                }
                0xF0 | 0xF7 => {
                    let l = vlq(data, &mut p, end) as usize;
                    p += l;
                }
                0x90 => {
                    let (pitch, vel) = (data[p], data[p + 1]);
                    p += 2;
                    if vel > 0 {
                        active.push((pitch, t));
                    } else {
                        close_note(&mut active, &mut song.melody, pitch, 64, t);
                    }
                }
                0x80 => {
                    let pitch = data[p];
                    p += 2;
                    close_note(&mut active, &mut song.melody, pitch, 64, t);
                }
                0xA0 | 0xB0 | 0xE0 => p += 2,
                0xC0 | 0xD0 => p += 1,
                _ => p += 1,
            }
        }
        idx = end;
    }

    song.melody.sort_by_key(|n| n.tick);
    song.bars = song
        .melody
        .iter()
        .map(|n| ((n.tick + n.dur) / TICKS_PER_BAR) as u16 + 1)
        .max()
        .unwrap_or(0);
    song.form_bars = song.bars;
    song.chorus_end = song.bars;
    song.choruses = 1;
    Ok(song)
}

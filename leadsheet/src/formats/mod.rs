//! Input formats. Each decoder produces the common [`crate::model::Song`].

pub mod biab;
pub mod midi;

use crate::model::{ParseError, Song};

/// Parse, auto-detecting the format by magic bytes: `MThd` → Standard MIDI,
/// otherwise Band-in-a-Box (`.MGU`/`.SGU`).
pub fn parse(data: &[u8]) -> Result<Song, ParseError> {
    if data.starts_with(b"MThd") {
        midi::parse(data)
    } else {
        biab::parse(data)
    }
}

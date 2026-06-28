//! `leadsheet` — read song charts (chords + melody + structure) from several
//! formats into one common [`model::Song`], and arrange them with pluggable
//! [`style`]s. Adding a format = a module in [`formats`]; the rest is unchanged.

pub mod arrange;
pub mod formats;
pub mod model;
pub mod style;

pub use model::{
    pitch_class_name, pitch_name, Chord, Note, ParseError, Song, PPQ, TICKS_PER_BAR,
};

/// Parse any supported file, auto-detecting the format from its contents.
pub fn parse(data: &[u8]) -> Result<Song, ParseError> {
    formats::parse(data)
}

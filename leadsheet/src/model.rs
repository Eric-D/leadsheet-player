//! Format-agnostic lead-sheet model shared by every input format.

/// Ticks per quarter note used by Band-in-a-Box (matches its MIDI export).
pub const PPQ: u32 = 120;

/// Ticks in one 4/4 bar.
pub const TICKS_PER_BAR: u32 = PPQ * 4;

#[derive(Clone, Debug)]
pub struct Note {
    /// Absolute start position in ticks (120 PPQ).
    pub tick: u32,
    /// MIDI pitch (60 = middle C).
    pub pitch: u8,
    /// MIDI velocity 1..=127.
    pub vel: u8,
    /// Duration in ticks.
    pub dur: u32,
}

#[derive(Clone, Debug)]
pub struct Chord {
    /// 1-based bar number.
    pub bar: u16,
    /// 0-based beat within the bar (0..=3 in 4/4). Songs like "Eye of the
    /// Tiger" place several chords per bar at different beats.
    pub beat: u8,
    /// Absolute start position in ticks (120 PPQ).
    pub tick: u32,
    /// Display text, e.g. "CMaj7".
    pub text: String,
    /// Root as a pitch class 0..=11 (C=0), or 255 if unknown.
    pub root: u8,
    /// Chord-type index (Band-in-a-Box table); 0 if unknown. Kept for re-encoding.
    pub ext: u8,
    /// Slash-bass pitch class 0..=11, or 255 if no slash.
    pub bass: u8,
    /// Articulation dots, à la Band-in-a-Box: 0 = none, 1 = rest (`.`),
    /// 2 = shot (`..`), 3 = hold (`...`).
    pub rest: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Song {
    pub title: String,
    pub style: String,
    pub tempo_bpm: u16,
    /// Key as a pitch class 0..=11 (C=0). Default 0 (C major).
    pub key_pc: u8,
    /// True if the song's key is minor.
    pub key_minor: bool,
    pub melody: Vec<Note>,
    pub chords: Vec<Chord>,
    /// Part markers `(bar, part)` — part 1 = substyle A, 2 = substyle B.
    pub part_markers: Vec<(u16, u8)>,
    /// Number of bars covered by the melody (derived). The melody is stored
    /// expanded over every chorus, so this is usually several times `form_bars`.
    pub bars: u16,
    /// Total bars in the chord chart (intro + chorus + ending), shown once.
    pub form_bars: u16,
    /// First/last bar of the repeated chorus section (the loop). For songs
    /// with no intro, `chorus_begin` is 1.
    pub chorus_begin: u16,
    pub chorus_end: u16,
    /// Number of times the chorus repeats, read from the file.
    pub choruses: u16,
    /// True if chords were decoded (vs. provisional/empty).
    pub chords_decoded: bool,
}

#[derive(Debug)]
pub enum ParseError {
    TooShort,
    BadTitle,
    /// A format-specific decoding error (e.g. malformed MIDI).
    Format(&'static str),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::TooShort => write!(f, "file too short"),
            ParseError::BadTitle => write!(f, "could not read song title"),
            ParseError::Format(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Pitch-class (0..=11) to note name in the given key flavour (sharps).
pub fn pitch_class_name(pc: u8) -> &'static str {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    NAMES[(pc % 12) as usize]
}

/// MIDI pitch to "C4"-style name (MIDI 60 = C4).
pub fn pitch_name(pitch: u8) -> String {
    let pc = pitch % 12;
    let octave = (pitch as i32 / 12) - 1;
    format!("{}{}", pitch_class_name(pc), octave)
}

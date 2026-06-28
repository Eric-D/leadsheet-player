//! Accompaniment styles — *our own* simple, importable format (NOT Band-in-a-Box's
//! proprietary `.STY`, whose engine isn't reproducible). A style says, per part,
//! which instrument and on which beats to play. Importable/shareable as RON.

use serde::{Deserialize, Serialize};

/// One part's pattern over a bar: an instrument + the beats (0-based) it hits.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pattern {
    /// Instrument preset index (matches the web app's instrument palette).
    pub instrument: u8,
    /// Beats of the bar to play (0-based; e.g. `[0, 2]` = beats 1 and 3).
    pub beats: Vec<u8>,
}

/// How to accompany a chart. A later version can carry distinct patterns for
/// substyles A and B (driven by the decoded part markers).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Style {
    pub name: String,
    pub swing: bool,
    pub bass: Pattern,
    pub comp: Pattern,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            name: "Basique".into(),
            swing: false,
            bass: Pattern { instrument: 8, beats: vec![0, 2] },
            comp: Pattern { instrument: 3, beats: vec![0, 2] },
        }
    }
}

impl Style {
    /// Import a style from RON text (the editable, shareable format).
    pub fn import(ron_text: &str) -> Result<Self, String> {
        ron::from_str(ron_text).map_err(|e| e.to_string())
    }

    /// Serialize this style to RON.
    pub fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()).unwrap_or_default()
    }
}

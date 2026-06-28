//! Custom rendering of the chord chart, melody staff notation, and guitar
//! tablature using egui's painter.

use leadsheet::{Song, Note, TICKS_PER_BAR};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

const INK: Color32 = Color32::from_rgb(30, 30, 35);
const PAPER: Color32 = Color32::from_rgb(236, 232, 214);
const ACCENT: Color32 = Color32::from_rgb(196, 60, 50);
const HILITE: Color32 = Color32::from_rgba_premultiplied(80, 140, 220, 70);

/// Standard guitar tuning, lowest string first (MIDI): E2 A2 D3 G3 B3 E4.
const STRINGS: [u8; 6] = [40, 45, 50, 55, 59, 64];

fn form_bars(song: &Song) -> u16 {
    let last = song
        .chords
        .last()
        .map(|c| c.bar)
        .unwrap_or(0)
        .max((song.bars).min(64));
    // Round up to a multiple of 4 for a tidy grid.
    (((last as u32 + 3) / 4) * 4).max(4) as u16
}

/// BiaB-style chord chart: four bars per row, chord names placed at the beat
/// where they start (several per bar for syncopated songs). The current bar is
/// highlighted during playback. Clicking returns the tick to seek playback to.
pub fn chord_chart(ui: &mut Ui, song: &Song, current_bar: u16, follow: bool) -> Option<u32> {
    let bars = form_bars(song);
    let mut active_cell: Option<Rect> = None;
    let cols = 4u16;
    let rows = (bars + cols - 1) / cols;
    let avail_w = ui.available_width().max(320.0);
    let cell_w = avail_w / cols as f32;
    let cell_h = 64.0;
    let size = Vec2::new(avail_w, rows as f32 * cell_h + 8.0);
    let (resp, painter) = ui.allocate_painter(size, Sense::click());
    let origin = resp.rect.min;

    painter.rect_filled(resp.rect, 0.0, PAPER);

    // Translate a click into the tick of the cell + beat it landed on.
    let mut clicked_tick = None;
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let col = ((p.x - origin.x) / cell_w).floor() as i32;
            let row = ((p.y - origin.y) / cell_h).floor() as i32;
            if col >= 0 && col < cols as i32 && row >= 0 {
                let bar = (row * cols as i32 + col + 1) as u16;
                if bar >= 1 && bar <= bars {
                    // Horizontal position inside the cell selects the beat.
                    let frac = (((p.x - origin.x) - col as f32 * cell_w) / cell_w).clamp(0.0, 0.999);
                    let beat = (frac * 4.0).floor() as u32;
                    clicked_tick =
                        Some((bar as u32 - 1) * TICKS_PER_BAR as u32 + beat * (TICKS_PER_BAR as u32 / 4));
                }
            }
        }
    }

    const PART_A: Color32 = Color32::from_rgb(40, 80, 205); // substyle A (blue)
    const PART_B: Color32 = Color32::from_rgb(30, 140, 60); // substyle B (green)

    for bar in 1..=bars {
        let col = (bar - 1) % cols;
        let row = (bar - 1) / cols;
        let cell = Rect::from_min_size(
            origin + Vec2::new(col as f32 * cell_w, row as f32 * cell_h),
            Vec2::new(cell_w, cell_h),
        );
        if bar == current_bar {
            painter.rect_filled(cell, 0.0, HILITE);
            active_cell = Some(cell);
        }
        painter.rect_stroke(cell, 0.0, Stroke::new(1.0, Color32::from_gray(170)));

        // Part marker (substyle A/B) as a coloured box with "<bar><a|b>".
        let part = song.part_markers.iter().find(|(b, _)| *b == bar).map(|(_, p)| *p);
        if let Some(p) = part {
            let (col_c, letter) = if p == 2 { (PART_B, "b") } else { (PART_A, "a") };
            let box_r = Rect::from_min_size(cell.min, Vec2::new(26.0, 16.0));
            painter.rect_filled(box_r, 0.0, col_c);
            painter.text(
                box_r.center(),
                Align2::CENTER_CENTER,
                format!("{bar}{letter}"),
                FontId::proportional(11.0),
                Color32::WHITE,
            );
        } else {
            painter.text(
                cell.min + Vec2::new(4.0, 3.0),
                Align2::LEFT_TOP,
                bar.to_string(),
                FontId::proportional(11.0),
                Color32::from_rgb(150, 90, 150),
            );
        }
    }

    // Repeat structure: a begin sign at bar 1 and an end sign at the last bar
    // of the form, annotated with the chorus count (the form repeats N times).
    let cell_of = |bar: u16| -> Rect {
        let col = (bar - 1) % cols;
        let row = (bar - 1) / cols;
        Rect::from_min_size(
            origin + Vec2::new(col as f32 * cell_w, row as f32 * cell_h),
            Vec2::new(cell_w, cell_h),
        )
    };
    let thick = Stroke::new(3.0, ACCENT);
    let dot = |painter: &egui::Painter, p: Pos2| painter.circle_filled(p, 2.2, ACCENT);
    // Begin: ‖:  at the first bar of the repeated chorus.
    let first = song.chorus_begin.clamp(1, bars);
    {
        let c = cell_of(first);
        painter.vline(c.min.x + 2.0, c.min.y + 6.0..=c.max.y - 6.0, thick);
        dot(&painter, Pos2::new(c.min.x + 8.0, c.center().y - 6.0));
        dot(&painter, Pos2::new(c.min.x + 8.0, c.center().y + 6.0));
    }
    // End: :‖ ×N  at the last bar of the chorus.
    let last = song.chorus_end.clamp(first, bars);
    {
        let c = cell_of(last);
        painter.vline(c.max.x - 2.0, c.min.y + 6.0..=c.max.y - 6.0, thick);
        dot(&painter, Pos2::new(c.max.x - 8.0, c.center().y - 6.0));
        dot(&painter, Pos2::new(c.max.x - 8.0, c.center().y + 6.0));
        if song.choruses > 1 {
            painter.text(
                Pos2::new(c.max.x - 14.0, c.min.y + 3.0),
                Align2::RIGHT_TOP,
                format!("×{}", song.choruses),
                FontId::proportional(15.0),
                ACCENT,
            );
        }
    }

    // How many chords share each bar — drives the font size so several chords
    // fit on one row without overlapping.
    let mut per_bar = vec![0u8; bars as usize + 2];
    for c in &song.chords {
        if c.bar >= 1 && c.bar <= bars {
            per_bar[c.bar as usize] += 1;
        }
    }

    // Chord names at their beat position within the bar.
    for c in &song.chords {
        if c.bar < 1 || c.bar > bars {
            continue;
        }
        let col = (c.bar - 1) % cols;
        let row = (c.bar - 1) / cols;
        let cell = Rect::from_min_size(
            origin + Vec2::new(col as f32 * cell_w, row as f32 * cell_h),
            Vec2::new(cell_w, cell_h),
        );
        let multi = per_bar[c.bar as usize] > 1;
        let font = if multi { 18.0 } else { 30.0 };
        let fx = (c.beat as f32 / 4.0) * (cell_w - 16.0);
        painter.text(
            Pos2::new(cell.min.x + 8.0 + fx, cell.center().y + 4.0),
            Align2::LEFT_CENTER,
            &c.text,
            FontId::proportional(font),
            INK,
        );
    }

    // Auto-follow: keep the active bar in the top third of the viewport.
    if follow {
        if let Some(cell) = active_cell {
            follow_into_view(ui, cell, false);
        }
    }

    clicked_tick
}

/// Scroll the enclosing `ScrollArea` so `target` sits in the first third of the
/// viewport (top third when `horizontal` is false, left third when true).
fn follow_into_view(ui: &Ui, target: Rect, horizontal: bool) {
    let clip = ui.clip_rect();
    let shifted = if horizontal {
        let s = clip.width() / 3.0;
        Rect::from_min_max(
            Pos2::new(target.left() - s, target.top()),
            Pos2::new(target.right() - s, target.bottom()),
        )
    } else {
        let s = clip.height() / 3.0;
        Rect::from_min_max(
            Pos2::new(target.left(), target.top() - s),
            Pos2::new(target.right(), target.bottom() - s),
        )
    };
    ui.scroll_to_rect(shifted, Some(egui::Align::Min));
}

// ---- Staff notation -------------------------------------------------------

const DEG: [i32; 12] = [0, 0, 1, 1, 2, 3, 3, 4, 4, 5, 5, 6];
const ACC: [bool; 12] = [
    false, true, false, true, false, false, true, false, true, false, true, false,
];

/// Absolute diatonic step index (C-major lattice). E4 = 37.
fn staff_step(pitch: u8) -> i32 {
    let oct = pitch as i32 / 12;
    let pc = (pitch % 12) as usize;
    oct * 7 + DEG[pc]
}

/// Treble-clef melody staff. Notes are spaced by time; barlines every 4 beats.
pub fn staff(ui: &mut Ui, song: &Song, current_tick: u32, follow: bool) {
    if song.melody.is_empty() {
        ui.label("Pas de mélodie dans ce fichier.");
        return;
    }
    let line_gap = 9.0f32; // distance between staff lines
    let half = line_gap / 2.0;
    let bottom_step = staff_step(64); // E4 bottom line
    let px_per_tick = 0.18f32;
    let total_ticks = song
        .melody
        .iter()
        .map(|n| n.tick + n.dur)
        .max()
        .unwrap_or(0)
        .max(TICKS_PER_BAR);

    let left_pad = 46.0f32;
    let width = left_pad + total_ticks as f32 * px_per_tick + 40.0;
    let mid_y = 90.0f32;
    let height = 180.0f32;
    let (resp, painter) = ui.allocate_painter(Vec2::new(width, height), Sense::hover());
    let o = resp.rect.min;
    painter.rect_filled(resp.rect, 0.0, PAPER);

    let baseline = o.y + mid_y + 2.0 * line_gap; // y of bottom line (E4)
    let x_of = |tick: u32| o.x + left_pad + tick as f32 * px_per_tick;
    let y_of = |pitch: u8| baseline - (staff_step(pitch) - bottom_step) as f32 * half;

    // Five staff lines.
    let staff_stroke = Stroke::new(1.0, Color32::from_gray(90));
    for i in 0..5 {
        let y = baseline - i as f32 * line_gap;
        painter.hline(o.x + 8.0..=o.x + width - 8.0, y, staff_stroke);
    }
    // A simple "treble" marker.
    painter.text(
        Pos2::new(o.x + 12.0, baseline - 2.0 * line_gap),
        Align2::LEFT_CENTER,
        "𝄞",
        FontId::proportional(34.0),
        INK,
    );

    // Barlines.
    let bars = (total_ticks + TICKS_PER_BAR - 1) / TICKS_PER_BAR;
    for b in 0..=bars {
        let x = x_of(b * TICKS_PER_BAR);
        painter.vline(
            x,
            baseline - 4.0 * line_gap..=baseline,
            Stroke::new(1.0, Color32::from_gray(120)),
        );
    }

    // Notes.
    for n in &song.melody {
        let x = x_of(n.tick);
        let y = y_of(n.pitch);
        let playing = current_tick >= n.tick && current_tick < n.tick + n.dur;
        let col = if playing { ACCENT } else { INK };

        draw_ledger_lines(&painter, x, y, baseline, line_gap, half, n.pitch, bottom_step);

        // Note head.
        painter.circle_filled(Pos2::new(x, y), 3.6, col);
        // Stem.
        let stem_up = staff_step(n.pitch) < bottom_step + 4;
        let sy = if stem_up { y - line_gap * 2.6 } else { y + line_gap * 2.6 };
        painter.line_segment(
            [Pos2::new(x + if stem_up { 3.3 } else { -3.3 }, y), Pos2::new(x + if stem_up { 3.3 } else { -3.3 }, sy)],
            Stroke::new(1.3, col),
        );
        // Accidental.
        if ACC[(n.pitch % 12) as usize] {
            painter.text(
                Pos2::new(x - 6.0, y),
                Align2::RIGHT_CENTER,
                "♯",
                FontId::proportional(12.0),
                col,
            );
        }
    }

    // Playback cursor.
    if current_tick > 0 && current_tick < total_ticks {
        let x = x_of(current_tick);
        painter.vline(
            x,
            o.y + mid_y - 6.0..=baseline + 12.0,
            Stroke::new(1.5, ACCENT),
        );
        if follow {
            follow_into_view(ui, Rect::from_min_max(Pos2::new(x, o.y), Pos2::new(x + 1.0, o.y + height)), true);
        }
    }
}

fn draw_ledger_lines(
    painter: &egui::Painter,
    x: f32,
    _y: f32,
    baseline: f32,
    line_gap: f32,
    half: f32,
    pitch: u8,
    bottom_step: i32,
) {
    let step = staff_step(pitch);
    let stroke = Stroke::new(1.0, Color32::from_gray(90));
    // Below the staff (steps below E4).
    let mut s = bottom_step - 2;
    while s >= step {
        let y = baseline - (s - bottom_step) as f32 * half;
        painter.hline(x - 7.0..=x + 7.0, y, stroke);
        s -= 2;
    }
    // Above the staff (steps above F5 = bottom_step + 8).
    let top = bottom_step + 8;
    let mut s2 = top + 2;
    while s2 <= step {
        let y = baseline - (s2 - bottom_step) as f32 * half;
        painter.hline(x - 7.0..=x + 7.0, y, stroke);
        s2 += 2;
    }
    let _ = line_gap;
}

// ---- Tablature ------------------------------------------------------------

fn pitch_to_tab(pitch: u8) -> Option<(usize, u8)> {
    // Choose the string giving the smallest non-negative fret (prefers higher
    // strings / lower frets), capped at fret 19.
    let mut best: Option<(usize, u8)> = None;
    for (i, &open) in STRINGS.iter().enumerate() {
        if pitch >= open {
            let fret = pitch - open;
            if fret <= 19 {
                match best {
                    Some((_, bf)) if (fret as i32) >= bf as i32 => {}
                    _ => best = Some((i, fret)),
                }
            }
        }
    }
    best
}

/// Guitar tablature of the melody: six lines, fret numbers placed by time.
pub fn tablature(ui: &mut Ui, song: &Song, current_tick: u32, follow: bool) {
    if song.melody.is_empty() {
        ui.label("Pas de mélodie à tabuler.");
        return;
    }
    let line_gap = 16.0f32;
    let px_per_tick = 0.2f32;
    let total_ticks = song
        .melody
        .iter()
        .map(|n| n.tick + n.dur)
        .max()
        .unwrap_or(0)
        .max(TICKS_PER_BAR);
    let left_pad = 26.0f32;
    let width = left_pad + total_ticks as f32 * px_per_tick + 40.0;
    let top = 24.0f32;
    let height = top + 5.0 * line_gap + 30.0;
    let (resp, painter) = ui.allocate_painter(Vec2::new(width, height), Sense::hover());
    let o = resp.rect.min;
    painter.rect_filled(resp.rect, 0.0, PAPER);

    let x_of = |tick: u32| o.x + left_pad + tick as f32 * px_per_tick;
    let y_of = |string: usize| o.y + top + string as f32 * line_gap;

    // Six string lines + "TAB" label.
    let line_stroke = Stroke::new(1.0, Color32::from_gray(110));
    for s in 0..6 {
        painter.hline(o.x + 8.0..=o.x + width - 8.0, y_of(s), line_stroke);
    }
    painter.text(
        Pos2::new(o.x + 10.0, o.y + top + 2.5 * line_gap),
        Align2::LEFT_CENTER,
        "TAB",
        FontId::monospace(13.0),
        Color32::from_gray(120),
    );

    // Barlines.
    let bars = (total_ticks + TICKS_PER_BAR - 1) / TICKS_PER_BAR;
    for b in 0..=bars {
        let x = x_of(b * TICKS_PER_BAR);
        painter.vline(x, y_of(0)..=y_of(5), Stroke::new(1.0, Color32::from_gray(150)));
    }

    // Fret numbers.
    for n in &song.melody {
        if let Some((string, fret)) = pitch_to_tab(n.pitch) {
            let x = x_of(n.tick);
            let y = y_of(5 - string); // draw high string at top
            let playing = current_tick >= n.tick && current_tick < n.tick + n.dur;
            let col = if playing { ACCENT } else { INK };
            // White pad behind the number so the string line doesn't cross it.
            let txt = fret.to_string();
            let r = painter.text(
                Pos2::new(x, y),
                Align2::CENTER_CENTER,
                &txt,
                FontId::monospace(13.0),
                col,
            );
            painter.rect_filled(r.expand2(Vec2::new(1.0, 0.0)), 0.0, PAPER);
            painter.text(
                Pos2::new(x, y),
                Align2::CENTER_CENTER,
                &txt,
                FontId::monospace(13.0),
                col,
            );
        }
    }

    if current_tick > 0 && current_tick < total_ticks {
        let x = x_of(current_tick);
        painter.vline(x, y_of(0) - 6.0..=y_of(5) + 6.0, Stroke::new(1.5, ACCENT));
        if follow {
            follow_into_view(ui, Rect::from_min_max(Pos2::new(x, o.y), Pos2::new(x + 1.0, o.y + height)), true);
        }
    }
}

// ---- Chord diagrams -------------------------------------------------------

const FRAME: Color32 = Color32::from_rgb(40, 44, 52);
const SYMBOL: Color32 = Color32::from_rgb(190, 40, 150);

/// A playable voicing: fret per string (low-E→high-E; -1 muted, 0 open) plus
/// the fretting finger for each string (0 = none/open).
#[derive(Clone, Copy)]
pub struct ChordShape {
    pub frets: [i8; 6],
    pub fingers: [u8; 6],
}

const fn sh(frets: [i8; 6], fingers: [u8; 6]) -> ChordShape {
    ChordShape { frets, fingers }
}

/// A playable shape for a major/minor triad. Common chords use familiar open
/// voicings with standard fingerings; the rest fall back to movable E-shape /
/// A-shape barre chords.
pub fn chord_shape(root_pc: u8, minor: bool) -> ChordShape {
    match (root_pc % 12, minor) {
        (0, false) => sh([-1, 3, 2, 0, 1, 0], [0, 3, 2, 0, 1, 0]), // C
        (2, false) => sh([-1, -1, 0, 2, 3, 2], [0, 0, 0, 1, 3, 2]), // D
        (4, false) => sh([0, 2, 2, 1, 0, 0], [0, 2, 3, 1, 0, 0]),  // E
        (5, false) => sh([1, 3, 3, 2, 1, 1], [1, 3, 4, 2, 1, 1]),  // F
        (7, false) => sh([3, 2, 0, 0, 0, 3], [2, 1, 0, 0, 0, 3]),  // G
        (9, false) => sh([-1, 0, 2, 2, 2, 0], [0, 0, 1, 2, 3, 0]), // A
        (2, true) => sh([-1, -1, 0, 2, 3, 1], [0, 0, 0, 2, 3, 1]), // Dm
        (4, true) => sh([0, 2, 2, 0, 0, 0], [0, 2, 3, 0, 0, 0]),   // Em
        (9, true) => sh([-1, 0, 2, 2, 1, 0], [0, 0, 2, 3, 1, 0]),  // Am
        (11, true) => sh([-1, 2, 4, 4, 3, 2], [0, 1, 3, 4, 2, 1]), // Bm
        _ => barre(root_pc, minor),
    }
}

fn barre(root_pc: u8, minor: bool) -> ChordShape {
    let fe = ((root_pc + 12 - 4) % 12) as i8; // root on the low-E string
    if (1..=7).contains(&fe) {
        if minor {
            sh([fe, fe + 2, fe + 2, fe, fe, fe], [1, 3, 4, 1, 1, 1])
        } else {
            sh([fe, fe + 2, fe + 2, fe + 1, fe, fe], [1, 3, 4, 2, 1, 1])
        }
    } else {
        let fa = ((root_pc + 12 - 9) % 12) as i8; // root on the A string
        if minor {
            sh([-1, fa, fa + 2, fa + 2, fa + 1, fa], [0, 1, 3, 4, 2, 1])
        } else {
            sh([-1, fa, fa + 2, fa + 2, fa + 2, fa], [0, 1, 2, 3, 4, 1])
        }
    }
}

/// French note name (solfège) for a pitch class.
pub fn french_name(pc: u8) -> &'static str {
    const N: [&str; 12] = [
        "Do", "Do♯", "Ré", "Mi♭", "Mi", "Fa", "Fa♯", "Sol", "Sol♯", "La", "Si♭", "Si",
    ];
    N[(pc % 12) as usize]
}

fn finger_color(f: u8) -> Color32 {
    match f {
        1 => Color32::from_rgb(190, 40, 150), // magenta
        2 => Color32::from_rgb(40, 110, 200), // blue
        3 => Color32::from_rgb(45, 150, 70),  // green
        4 => Color32::from_rgb(220, 130, 30), // orange
        _ => Color32::from_rgb(120, 120, 130),
    }
}

/// Draw a polished guitar chord diagram (title, French name, fretboard with
/// numbered finger dots, X/O markers and string names). `active` tints the
/// title (used for the currently-playing chord).
pub fn chord_diagram(ui: &mut Ui, name: &str, root_pc: u8, minor: bool, active: bool) {
    let shape = chord_shape(root_pc, minor);
    // Scale the whole diagram to the available width so dragging the panel
    // divider makes it bigger (or smaller).
    let w = ui.available_width().clamp(124.0, 460.0);
    let s = w / 150.0; // scale factor relative to the base 150px design
    let h = 214.0 * s;
    let (resp, painter) = ui.allocate_painter(Vec2::new(w, h), Sense::hover());
    let o = resp.rect.min;

    if active {
        painter.rect_filled(resp.rect, 6.0 * s, Color32::from_rgb(247, 240, 248));
    }

    // Title: chord symbol + French name.
    painter.text(
        Pos2::new(o.x + w / 2.0, o.y + 2.0 * s),
        Align2::CENTER_TOP,
        name,
        FontId::proportional(26.0 * s),
        SYMBOL,
    );
    painter.text(
        Pos2::new(o.x + w / 2.0, o.y + 34.0 * s),
        Align2::CENTER_TOP,
        format!("{} {}", french_name(root_pc), if minor { "mineur" } else { "Majeur" }),
        FontId::proportional(13.0 * s),
        FRAME,
    );

    let nstr = 6usize;
    let nfret = 4usize;
    let left = o.x + 20.0 * s;
    let right = o.x + w - 14.0 * s;
    let top = o.y + 70.0 * s;
    let bottom = top + 100.0 * s;
    let dx = (right - left) / (nstr as f32 - 1.0);
    let dy = (bottom - top) / nfret as f32;

    let fretted: Vec<i8> = shape.frets.iter().copied().filter(|&f| f > 0).collect();
    let base = if fretted.iter().any(|&f| f > nfret as i8) {
        *fretted.iter().min().unwrap()
    } else {
        0
    };

    // Rounded fretboard frame (nut) + strings + frets.
    let frame_rect = Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom));
    painter.rect_stroke(frame_rect, 7.0 * s, Stroke::new(2.5 * s, FRAME));
    let grid = Stroke::new(1.0 * s, Color32::from_gray(150));
    for i in 1..nfret {
        let y = top + i as f32 * dy;
        painter.hline(left..=right, y, grid);
    }
    for i in 0..nstr {
        let x = left + i as f32 * dx;
        painter.vline(x, top..=bottom, Stroke::new(2.0 * s, FRAME));
    }
    // Fret numbers down the left side (1..4 for open shapes, shifted for barres).
    for i in 1..=nfret {
        let fret = if base == 0 {
            i as i32
        } else {
            base as i32 + i as i32 - 1
        };
        let y = top + (i as f32 - 0.5) * dy;
        painter.text(
            Pos2::new(left - 8.0 * s, y),
            Align2::RIGHT_CENTER,
            fret.to_string(),
            FontId::proportional(10.0 * s),
            Color32::from_gray(120),
        );
    }

    // X / O markers above the frame.
    for (si, &f) in shape.frets.iter().enumerate() {
        let x = left + si as f32 * dx;
        if f < 0 {
            painter.text(Pos2::new(x, top - 13.0 * s), Align2::CENTER_CENTER, "✕",
                FontId::proportional(14.0 * s), FRAME);
        } else if f == 0 {
            painter.circle_stroke(Pos2::new(x, top - 12.0 * s), 5.0 * s, Stroke::new(1.6 * s, FRAME));
        }
    }

    // Finger dots with numbers.
    for si in 0..nstr {
        let f = shape.frets[si];
        if f <= 0 {
            continue;
        }
        let x = left + si as f32 * dx;
        let rel = f as f32 - base as f32 + if base == 0 { 0.0 } else { 1.0 };
        let y = top + (rel - 0.5) * dy;
        let finger = shape.fingers[si];
        painter.circle_filled(Pos2::new(x, y), 10.0 * s, finger_color(finger));
        if finger > 0 {
            painter.text(Pos2::new(x, y), Align2::CENTER_CENTER, finger.to_string(),
                FontId::proportional(14.0 * s), Color32::WHITE);
        }
    }

    // Open-string names at the bottom (E A D G B E).
    const OPEN: [&str; 6] = ["E", "A", "D", "G", "B", "E"];
    for (si, lbl) in OPEN.iter().enumerate() {
        let x = left + si as f32 * dx;
        painter.text(Pos2::new(x, bottom + 6.0 * s), Align2::CENTER_TOP, *lbl,
            FontId::proportional(13.0 * s), FRAME);
    }
}

/// A compact legend mapping dot colours to finger numbers (1–4).
pub fn finger_legend(ui: &mut Ui) {
    let w = ui.available_width().min(168.0).max(120.0);
    let (resp, painter) = ui.allocate_painter(Vec2::new(w, 24.0), Sense::hover());
    let o = resp.rect.min;
    let cy = o.y + 12.0;
    painter.text(
        Pos2::new(o.x + 2.0, cy),
        Align2::LEFT_CENTER,
        "Doigts",
        FontId::proportional(11.0),
        FRAME,
    );
    let mut x = o.x + 48.0;
    for f in 1..=4u8 {
        painter.circle_filled(Pos2::new(x, cy), 8.0, finger_color(f));
        painter.text(
            Pos2::new(x, cy),
            Align2::CENTER_CENTER,
            f.to_string(),
            FontId::proportional(11.0),
            Color32::WHITE,
        );
        x += 27.0;
    }
}

#[allow(dead_code)]
fn _unused(_: &Note) {}

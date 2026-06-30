//! Minimal Web Audio playback engine with selectable instrument timbres,
//! a per-part volume mixer, and arbitrary-position (seek) scheduling.
//!
//! There is no soundfont: each "instrument" is a small synth preset built from
//! one or two oscillators plus an amplitude envelope. Three parts can play,
//! each with its own instrument and volume: the melody, a chord accompaniment,
//! and a bass line (chord roots). Signal path:
//!
//!     voices → part bus (melody / chords / bass) → master → destination
//!
//! Volume changes only touch the relevant gain node, so they apply live
//! without rescheduling. Playback can start at any tick (`start_tick`), which
//! is what powers "keep position on option change" and "click a chord to seek".

use leadsheet::{Song, PPQ};
use web_sys::{AudioContext, GainNode, OscillatorNode, OscillatorType};

#[derive(Clone, Copy)]
pub struct Instrument {
    pub name: &'static str,
    wave: OscillatorType,
    attack: f32,
    sustained: bool,
    decay: f32,
    octave: f32,
    peak: f32,
}

pub const PRESETS: &[Instrument] = &[
    Instrument { name: "Guitare nylon",      wave: OscillatorType::Triangle, attack: 0.005, sustained: false, decay: 0.75, octave: 0.15, peak: 1.0 },
    Instrument { name: "Guitare élec. jazz", wave: OscillatorType::Sine,     attack: 0.005, sustained: false, decay: 0.95, octave: 0.28, peak: 1.0 },
    Instrument { name: "Guitare acoustique", wave: OscillatorType::Triangle, attack: 0.004, sustained: false, decay: 0.9,  octave: 0.22, peak: 1.0 },
    Instrument { name: "Piano",              wave: OscillatorType::Triangle, attack: 0.003, sustained: false, decay: 1.5,  octave: 0.2,  peak: 1.0 },
    Instrument { name: "Piano élec.",        wave: OscillatorType::Sine,     attack: 0.003, sustained: false, decay: 1.2,  octave: 0.35, peak: 1.0 },
    Instrument { name: "Cordes",             wave: OscillatorType::Sawtooth, attack: 0.14,  sustained: true,  decay: 0.3,  octave: 0.0,  peak: 0.8 },
    Instrument { name: "Orgue",              wave: OscillatorType::Square,   attack: 0.01,  sustained: true,  decay: 0.06, octave: 0.3,  peak: 0.7 },
    Instrument { name: "Flûte",              wave: OscillatorType::Sine,     attack: 0.06,  sustained: true,  decay: 0.12, octave: 0.06, peak: 0.95 },
    Instrument { name: "Basse (doigt)",      wave: OscillatorType::Triangle, attack: 0.005, sustained: false, decay: 0.65, octave: 0.1,  peak: 1.15 },
    Instrument { name: "Synthé lead",        wave: OscillatorType::Sawtooth, attack: 0.005, sustained: false, decay: 0.55, octave: 0.0,  peak: 0.9 },
];

#[derive(Clone, Copy)]
pub struct Parts {
    pub melody_on: bool,
    pub melody_instr: usize,
    pub chords_on: bool,
    pub chords_instr: usize,
    pub bass_on: bool,
    pub bass_instr: usize,
}

impl Default for Parts {
    fn default() -> Self {
        Self {
            melody_on: true,
            melody_instr: 0,
            chords_on: true,
            chords_instr: 3,
            bass_on: true,
            bass_instr: 8,
        }
    }
}

pub struct AudioEngine {
    ctx: AudioContext,
    master: GainNode,
    mel_bus: GainNode,
    chd_bus: GainNode,
    bas_bus: GainNode,
    /// Live voices with their end time, so finished ones can be pruned — only a
    /// few seconds of notes exist at once (mobile audio caps the node count).
    voices: Vec<(OscillatorNode, GainNode, f64)>,
    /// Previous batch, kept one cycle so it can be fully disconnected after its
    /// fade — guarantees no node buildup across rapid reschedules (seeking by
    /// clicking a chord many times).
    dying: Vec<(OscillatorNode, GainNode)>,
    /// ctx time of the schedule origin.
    origin: f64,
    /// Song tick that maps to `origin` (the seek point).
    origin_tick: u32,
    sec_per_tick: f64,
    end_tick: u32,
    scheduled: bool,
    /// Ticks in one form repetition (for looping the accompaniment).
    form_ticks: u32,
    /// Notes computed but whose oscillator isn't created yet. We create them a
    /// few per frame so a (re)schedule never spikes the CPU, yet the whole song
    /// ends up in the audio graph — which keeps playing even if the tab is
    /// backgrounded (the render loop stops there, but the audio thread doesn't).
    pending: Vec<PendingNote>,
}

struct PendingNote {
    bus: BusKind,
    pitch: u8,
    when: f64,
    dur: f64,
    peak: f32,
    instr: Instrument,
}

fn midi_to_freq(pitch: u8) -> f32 {
    440.0 * 2f32.powf((pitch as f32 - 69.0) / 12.0)
}

/// Seconds of notes kept scheduled ahead of the playhead. Small enough to bound
/// the live audio-node count (mobile-safe), large enough to absorb frame drops.
const LOOKAHEAD: f64 = 6.0;

impl AudioEngine {
    pub fn new() -> Option<Self> {
        let ctx = AudioContext::new().ok()?;
        // Expose the context so the page can resume() it from a real touch/click
        // handler — mobile browsers (iPad/Safari) only start audio in a gesture,
        // and egui processes clicks in the render loop, outside that gesture.
        if let Some(win) = web_sys::window() {
            let _ = js_sys::Reflect::set(
                &win,
                &wasm_bindgen::JsValue::from_str("__audioCtx"),
                ctx.as_ref(),
            );
        }
        let master = ctx.create_gain().ok()?;
        master.gain().set_value(0.9);
        master.connect_with_audio_node(&ctx.destination()).ok()?;
        let mk_bus = || -> Option<GainNode> {
            let b = ctx.create_gain().ok()?;
            b.gain().set_value(1.0);
            b.connect_with_audio_node(&master).ok()?;
            Some(b)
        };
        let mel_bus = mk_bus()?;
        let chd_bus = mk_bus()?;
        let bas_bus = mk_bus()?;
        Some(Self {
            ctx,
            master,
            mel_bus,
            chd_bus,
            bas_bus,
            voices: Vec::new(),
            dying: Vec::new(),
            origin: 0.0,
            origin_tick: 0,
            sec_per_tick: 0.0,
            end_tick: 0,
            scheduled: false,
            form_ticks: 0,
            pending: Vec::new(),
        })
    }

    /// Smoothly approach a new gain (≈60 ms) so volume/part changes never pop.
    fn ramp(&self, g: &web_sys::AudioParam, v: f32) {
        let t = self.ctx.current_time();
        let _ = g.cancel_scheduled_values(t);
        let _ = g.set_target_at_time(v.max(0.0001) as f32, t, 0.02);
    }
    pub fn set_master_gain(&self, v: f32) {
        self.ramp(&self.master.gain(), v);
    }
    pub fn set_part_gains(&self, melody: f32, chords: f32, bass: f32) {
        self.ramp(&self.mel_bus.gain(), melody);
        self.ramp(&self.chd_bus.gain(), chords);
        self.ramp(&self.bas_bus.gain(), bass);
    }

    fn osc(&self, bus: &GainNode, freq: f32, start: f64, dur: f64, peak: f32, instr: &Instrument) -> Option<(OscillatorNode, GainNode, f64)> {
        let osc = self.ctx.create_oscillator().ok()?;
        let gain = self.ctx.create_gain().ok()?;
        osc.set_type(instr.wave);
        osc.frequency().set_value(freq);
        let g = gain.gain();
        let attack = instr.attack as f64;
        g.set_value_at_time(0.0001, start).ok()?;
        g.linear_ramp_to_value_at_time(peak, start + attack).ok()?;
        let end = if instr.sustained {
            let hold_end = (start + dur).max(start + attack + 0.02);
            g.set_value_at_time(peak, (hold_end - instr.decay as f64).max(start + attack)).ok()?;
            g.linear_ramp_to_value_at_time(0.0001, hold_end + instr.decay as f64).ok()?;
            hold_end + instr.decay as f64
        } else {
            let decay_end = start + attack + instr.decay as f64;
            g.exponential_ramp_to_value_at_time(0.001, decay_end).ok()?;
            decay_end
        };
        osc.connect_with_audio_node(&gain).ok()?;
        gain.connect_with_audio_node(bus).ok()?;
        osc.start_with_when(start).ok()?;
        osc.stop_with_when(end + 0.03).ok()?;
        Some((osc, gain, end + 0.03))
    }

    /// A kick-drum "boom" for the count-off: a low sine with a fast pitch drop,
    /// punchy enough that consecutive hits stay distinct. Accents hit harder.
    fn kick(&mut self, when: f64, accent: bool) {
        if let (Ok(osc), Ok(gain)) = (self.ctx.create_oscillator(), self.ctx.create_gain()) {
            osc.set_type(OscillatorType::Sine);
            let f = osc.frequency();
            let _ = f.set_value_at_time(if accent { 165.0 } else { 140.0 }, when);
            let _ = f.exponential_ramp_to_value_at_time(50.0, when + 0.09);
            let g = gain.gain();
            let peak = if accent { 1.0 } else { 0.7 };
            let _ = g.set_value_at_time(peak, when);
            let _ = g.exponential_ramp_to_value_at_time(0.001, when + 0.14);
            if osc.connect_with_audio_node(&gain).is_ok()
                && gain.connect_with_audio_node(&self.master).is_ok()
            {
                let _ = osc.start_with_when(when);
                let _ = osc.stop_with_when(when + 0.16);
                self.voices.push((osc, gain, when + 0.16));
            }
        }
    }

    fn note(&mut self, bus_kind: BusKind, pitch: u8, start: f64, dur: f64, peak: f32, instr: &Instrument) {
        let bus = match bus_kind {
            BusKind::Melody => self.mel_bus.clone(),
            BusKind::Chords => self.chd_bus.clone(),
            BusKind::Bass => self.bas_bus.clone(),
        };
        let f = midi_to_freq(pitch);
        if let Some(pair) = self.osc(&bus, f, start, dur, peak * instr.peak, instr) {
            self.voices.push(pair);
        }
        if instr.octave > 0.0 {
            if let Some(pair) = self.osc(&bus, f * 2.0, start, dur, peak * instr.peak * instr.octave, instr) {
                self.voices.push(pair);
            }
        }
    }

    /// Map an absolute song tick to a wall-clock time on the shared timeline
    /// (origin/origin_tick), clipping a note overlapping `floor` to start there.
    /// Returns None for a note already finished before `floor`.
    fn place(&self, abs_tick: u32, dur_ticks: u32, floor: u32) -> Option<(f64, f64)> {
        let end = abs_tick + dur_ticks;
        if end <= floor {
            return None;
        }
        let eff = abs_tick.max(floor);
        let when = self.origin + (eff as f64 - self.origin_tick as f64) * self.sec_per_tick;
        let dur = (end - eff) as f64 * self.sec_per_tick;
        Some((when, dur.max(0.03)))
    }

    /// Compute every note of the song (from `start_tick`) into `self.pending`
    /// without creating any oscillator yet — that part is cheap.
    fn build_pending(&mut self, song: &Song, parts: &Parts, start_tick: u32) {
        if parts.melody_on {
            let instr = PRESETS[parts.melody_instr.min(PRESETS.len() - 1)];
            for n in &song.melody {
                if let Some((when, dur)) = self.place(n.tick, n.dur, start_tick) {
                    let peak = 0.18 + (n.vel as f32 / 127.0) * 0.12;
                    self.pending.push(PendingNote { bus: BusKind::Melody, pitch: n.pitch, when, dur, peak, instr });
                }
            }
        }
        // Accompaniment from the `leadsheet` arranger (bass + comped chords on
        // the style's beats, busier in substyle B via the A/B markers).
        //
        // Lay it out on the LINEAR performance, mapping each played bar to its
        // chart bar via the song's structure — intro once, chorus ×N, ending
        // once — so a song with an intro/ending repeats only the chorus (not the
        // whole form), exactly like the chart's repeat brackets.
        if (parts.chords_on || parts.bass_on) && self.form_ticks > 0 {
            use leadsheet::arrange::Part as AP;
            let chord_i = PRESETS[parts.chords_instr.min(PRESETS.len() - 1)];
            let bass_i = PRESETS[parts.bass_instr.min(PRESETS.len() - 1)];
            let events = leadsheet::arrange::arrange(song, &leadsheet::style::Style::default());
            let bar = leadsheet::TICKS_PER_BAR;

            let cb = song.chorus_begin.max(1) as u32;
            let ce = song.chorus_end.max(song.chorus_begin).max(1) as u32;
            let form_bars = (song.form_bars as u32).max(ce).max(self.form_ticks / bar);
            let clen = (ce + 1 - cb).max(1);
            let choruses = song.choruses.max(1) as u32;
            // The performance is as long as the (expanded) melody when there is
            // one, otherwise the structural length intro + chorus×N + ending.
            let structural = (cb - 1) + clen * choruses + (form_bars - ce);
            let melody_bars = song
                .melody
                .iter()
                .map(|n| (n.tick + n.dur + bar - 1) / bar)
                .max()
                .unwrap_or(0);
            let total_bars = melody_bars.max(structural).max(form_bars);

            // Index the form's events by their chart bar (0-based).
            let mut by_bar: Vec<Vec<usize>> = vec![Vec::new(); (form_bars + 1) as usize];
            for (i, e) in events.iter().enumerate() {
                let b = (e.tick / bar) as usize;
                if b < by_bar.len() {
                    by_bar[b].push(i);
                }
            }

            for lb in 0..total_bars {
                let cbar = chart_bar0(lb, cb, ce, choruses, form_bars);
                let Some(idxs) = by_bar.get(cbar as usize) else { continue };
                for &i in idxs {
                    let e = &events[i];
                    let abs = lb * bar + e.tick % bar; // event at its linear bar
                    match e.part {
                        AP::Comp if parts.chords_on => {
                            if let Some((when, dur)) = self.place(abs, e.dur, start_tick) {
                                self.pending.push(PendingNote { bus: BusKind::Chords, pitch: e.pitch, when, dur: dur.min(2.0), peak: 0.06, instr: chord_i });
                            }
                        }
                        AP::Bass if parts.bass_on => {
                            if let Some((when, dur)) = self.place(abs, e.dur, start_tick) {
                                self.pending.push(PendingNote { bus: BusKind::Bass, pitch: e.pitch, when, dur: dur.min(1.2), peak: 0.13, instr: bass_i });
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        // Soonest last, so `pump` can pop the next note in O(1).
        self.pending.sort_by(|a, b| b.when.total_cmp(&a.when));
    }

    /// Realise the notes due within the look-ahead window and prune finished
    /// voices. Called every frame: only a few seconds of notes are ever alive at
    /// once, which keeps the audio-node count low (mobile browsers cap it, and a
    /// flood of nodes caused random dropouts on tablets).
    pub fn pump(&mut self) {
        let now = self.ctx.current_time();
        // Disconnect voices whose sound is over.
        self.voices.retain(|(osc, gain, end)| {
            if *end < now - 0.05 {
                let _ = osc.disconnect();
                let _ = gain.disconnect();
                false
            } else {
                true
            }
        });
        // Create everything starting within the look-ahead horizon (soonest are
        // at the end of `pending`, sorted descending by time).
        let horizon = now + LOOKAHEAD;
        while self.pending.last().map_or(false, |p| p.when <= horizon) {
            let p = self.pending.pop().unwrap();
            self.note(p.bus, p.pitch, p.when, p.dur, p.peak, &p.instr);
        }
    }

    /// (Re)build the schedule from `start_tick`. Notes are computed up front but
    /// their oscillators are created in batches (`pump`) to avoid a CPU spike.
    pub fn schedule(&mut self, song: &Song, tempo_factor: f32, parts: &Parts, start_tick: u32, count_in: bool) {
        self.clear();
        let bpm = (song.tempo_bpm as f32 * tempo_factor).max(1.0);
        self.sec_per_tick = 60.0 / (bpm as f64 * PPQ as f64);
        // Optional drum count-off, "1 — 2 — 1 2 3 4" over two bars (the "1"s
        // accented with a kick), then the song starts in time — like BiaB.
        let beat_dur = PPQ as f64 * self.sec_per_tick; // one beat
        let lead = self.ctx.current_time() + 0.12;
        if count_in {
            // Classic count-off: "1 — 2 —" then "1 2 3 4", the two "1"s accented.
            for &(b, accent) in &[(0.0, true), (2.0, false), (4.0, true), (5.0, false), (6.0, false), (7.0, false)] {
                self.kick(lead + b * beat_dur, accent);
            }
            self.origin = lead + 8.0 * beat_dur;
        } else {
            self.origin = lead;
        }
        self.origin_tick = start_tick;

        let melody_end = song.melody.iter().map(|n| n.tick + n.dur).max().unwrap_or(0);
        let cells = chord_cells(song);
        self.form_ticks = cells.iter().map(|(s, d, _)| s + d).max().unwrap_or(0);
        let accompaniment = (parts.chords_on || parts.bass_on) && self.form_ticks > 0;

        // Honour the chorus repeat (×N) in playback, like the chart does: the
        // linear performance is intro + chorus×choruses + ending bars. Used as a
        // floor so the accompaniment loops the right number of times even when
        // the melody isn't stored expanded over the choruses.
        let bar = leadsheet::TICKS_PER_BAR;
        let cb = song.chorus_begin.max(1) as u32;
        let ce = song.chorus_end.max(song.chorus_begin).max(1) as u32;
        let form_bars = (song.form_bars as u32).max(ce);
        let clen = (ce + 1 - cb).max(1);
        let perform = ((cb - 1) + clen * song.choruses.max(1) as u32 + (form_bars - ce)) * bar;

        self.end_tick = if accompaniment {
            melody_end.max(self.form_ticks).max(perform)
        } else {
            melody_end
        };

        self.build_pending(song, parts, start_tick);
        self.scheduled = true;
        // Realise the first look-ahead window now so playback starts on time;
        // subsequent windows are created frame by frame by `pump`.
        self.pump();
    }

    pub fn resume(&self) {
        let _ = self.ctx.resume();
    }
    pub fn suspend(&self) {
        let _ = self.ctx.suspend();
    }
    pub fn stop(&mut self) {
        self.clear();
    }

    fn clear(&mut self) {
        // Drop any not-yet-created notes from the previous schedule.
        self.pending.clear();
        // Ramp each voice to silence over a few ms before stopping, so cutting
        // playback (tempo change, seek, stop) doesn't produce a click/pop.
        let t = self.ctx.current_time();
        // The batch faded out by the previous clear() is now silent: stop and
        // DISCONNECT it for good, so nothing accumulates across rapid clicks.
        for (osc, gain) in self.dying.drain(..) {
            let _ = osc.stop_with_when(t);
            let _ = osc.disconnect();
            let _ = gain.disconnect();
        }
        for (osc, gain, _end) in self.voices.drain(..) {
            let g = gain.gain();
            let _ = g.cancel_scheduled_values(t);
            let _ = g.set_value_at_time(g.value(), t);
            let _ = g.linear_ramp_to_value_at_time(0.0001, t + 0.012);
            // Stop after the fade; the node is disconnected on the next clear().
            let _ = osc.stop_with_when(t + 0.02);
            self.dying.push((osc, gain));
        }
        self.scheduled = false;
    }

    pub fn is_scheduled(&self) -> bool {
        self.scheduled
    }

    pub fn position_ticks(&self) -> u32 {
        if !self.scheduled || self.sec_per_tick <= 0.0 {
            return self.origin_tick;
        }
        let elapsed = self.ctx.current_time() - self.origin;
        if elapsed <= 0.0 {
            self.origin_tick
        } else {
            self.origin_tick + (elapsed / self.sec_per_tick) as u32
        }
    }

    pub fn finished(&self) -> bool {
        self.scheduled && self.position_ticks() > self.end_tick + (PPQ * 2)
    }
}

#[derive(Clone, Copy)]
enum BusKind {
    Melody,
    Chords,
    Bass,
}

/// Map a 0-based linear performance bar to its 0-based chart bar: intro once,
/// then the chorus cycled EXACTLY `choruses` times, then the ending. This must
/// match the display's `chart_bar` (app.rs) bar-for-bar, otherwise the (raw,
/// expanded) melody and the (reconstructed) accompaniment drift apart. It does
/// NOT key off the melody length: a melody whose last note rings a few bars past
/// the form must not make the accompaniment loop the chorus an extra time.
fn chart_bar0(lb: u32, cb: u32, ce: u32, choruses: u32, form_bars: u32) -> u32 {
    let intro = cb - 1;
    let clen = (ce + 1 - cb).max(1);
    if lb < intro {
        return lb;
    }
    let after = lb - intro;
    let total_chorus = clen * choruses;
    if after < total_chorus {
        intro + after % clen
    } else {
        (ce + (after - total_chorus)).min(form_bars.saturating_sub(1))
    }
}

/// (start_tick, dur_ticks, root_pc) for each visible chord, using the decoded
/// sub-bar tick positions so syncopated bars play correctly.
fn chord_cells(song: &Song) -> Vec<(u32, u32, u8)> {
    let beat = leadsheet::TICKS_PER_BAR / 4;
    let mut out = Vec::new();
    for (i, c) in song.chords.iter().enumerate() {
        let start = c.tick;
        let end = song
            .chords
            .get(i + 1)
            .map(|n| n.tick)
            .unwrap_or(start + leadsheet::TICKS_PER_BAR);
        let dur = end.saturating_sub(start).max(beat);
        out.push((start, dur, c.root));
    }
    out
}

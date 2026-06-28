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
    voices: Vec<(OscillatorNode, GainNode)>,
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
    /// Real end of the song; we schedule only up to `window_end_tick` at a time.
    song_end_tick: u32,
    window_end_tick: u32,
}

fn midi_to_freq(pitch: u8) -> f32 {
    440.0 * 2f32.powf((pitch as f32 - 69.0) / 12.0)
}

impl AudioEngine {
    pub fn new() -> Option<Self> {
        let ctx = AudioContext::new().ok()?;
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
            song_end_tick: 0,
            window_end_tick: 0,
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

    fn osc(&self, bus: &GainNode, freq: f32, start: f64, dur: f64, peak: f32, instr: &Instrument) -> Option<(OscillatorNode, GainNode)> {
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
        Some((osc, gain))
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

    /// Seconds of music scheduled ahead at once. A bounded window means far
    /// fewer live oscillators, so a reschedule (instrument/part switch) is cheap
    /// and doesn't spike the CPU / crackle.
    const HORIZON_SEC: f64 = 20.0;
    fn horizon_ticks(&self) -> u32 {
        (Self::HORIZON_SEC / self.sec_per_tick.max(1e-9)) as u32
    }

    /// Schedule every note/accompaniment event starting in `[from, to)`. With
    /// `clip` (the first window) also include a note already sounding at `from`.
    fn schedule_window(&mut self, song: &Song, parts: &Parts, from: u32, to: u32, clip: bool) {
        let want = |tick: u32, dur: u32| tick < to && (if clip { tick + dur > from } else { tick >= from });
        if parts.melody_on {
            let instr = PRESETS[parts.melody_instr.min(PRESETS.len() - 1)];
            for n in &song.melody {
                if n.tick >= to {
                    break;
                }
                if want(n.tick, n.dur) {
                    if let Some((when, dur)) = self.place(n.tick, n.dur, from) {
                        let peak = 0.18 + (n.vel as f32 / 127.0) * 0.12;
                        self.note(BusKind::Melody, n.pitch, when, dur, peak, &instr);
                    }
                }
            }
        }
        // Accompaniment from the `leadsheet` arranger (bass + comped chords on
        // the style's beats, busier in substyle B via the A/B markers).
        if (parts.chords_on || parts.bass_on) && self.form_ticks > 0 {
            use leadsheet::arrange::Part as AP;
            let chord_i = PRESETS[parts.chords_instr.min(PRESETS.len() - 1)];
            let bass_i = PRESETS[parts.bass_instr.min(PRESETS.len() - 1)];
            let events = leadsheet::arrange::arrange(song, &leadsheet::style::Style::default());
            let form = self.form_ticks;
            let mut base = (from / form) * form;
            while base < to {
                for e in &events {
                    let abs = base + e.tick;
                    if !want(abs, e.dur) {
                        continue;
                    }
                    match e.part {
                        AP::Comp if parts.chords_on => {
                            if let Some((when, dur)) = self.place(abs, e.dur, from) {
                                self.note(BusKind::Chords, e.pitch, when, dur.min(2.0), 0.06, &chord_i);
                            }
                        }
                        AP::Bass if parts.bass_on => {
                            if let Some((when, dur)) = self.place(abs, e.dur, from) {
                                self.note(BusKind::Bass, e.pitch, when, dur.min(1.2), 0.13, &bass_i);
                            }
                        }
                        _ => {}
                    }
                }
                base += form;
            }
        }
    }

    /// (Re)build the schedule from `start_tick`, but only the first window's
    /// worth of notes; `maybe_extend` appends the rest as playback advances.
    pub fn schedule(&mut self, song: &Song, tempo_factor: f32, parts: &Parts, start_tick: u32) {
        self.clear();
        let bpm = (song.tempo_bpm as f32 * tempo_factor).max(1.0);
        self.sec_per_tick = 60.0 / (bpm as f64 * PPQ as f64);
        self.origin = self.ctx.current_time() + 0.12;
        self.origin_tick = start_tick;

        let melody_end = song.melody.iter().map(|n| n.tick + n.dur).max().unwrap_or(0);
        let cells = chord_cells(song);
        self.form_ticks = cells.iter().map(|(s, d, _)| s + d).max().unwrap_or(0);
        let accompaniment = (parts.chords_on || parts.bass_on) && self.form_ticks > 0;
        let song_end = if accompaniment { melody_end.max(self.form_ticks) } else { melody_end };

        let window_end = song_end.min(start_tick + self.horizon_ticks());
        self.schedule_window(song, parts, start_tick, window_end, true);

        self.song_end_tick = song_end;
        self.window_end_tick = window_end;
        self.end_tick = song_end;
        self.scheduled = true;
    }

    /// Append the next window once playback nears the scheduled horizon. Appends
    /// on the existing timeline (no clear), so it's seamless. Call every frame.
    pub fn maybe_extend(&mut self, song: &Song, parts: &Parts) {
        if !self.scheduled || self.window_end_tick >= self.song_end_tick {
            return;
        }
        let refill = (6.0 / self.sec_per_tick.max(1e-9)) as u32;
        if self.position_ticks() + refill < self.window_end_tick {
            return;
        }
        let to = self.song_end_tick.min(self.window_end_tick + self.horizon_ticks());
        let from = self.window_end_tick;
        self.schedule_window(song, parts, from, to, false);
        self.window_end_tick = to;
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
        for (osc, gain) in self.voices.drain(..) {
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

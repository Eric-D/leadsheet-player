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
        })
    }

    pub fn set_master_gain(&self, v: f32) {
        self.master.gain().set_value(v);
    }
    pub fn set_part_gains(&self, melody: f32, chords: f32, bass: f32) {
        self.mel_bus.gain().set_value(melody);
        self.chd_bus.gain().set_value(chords);
        self.bas_bus.gain().set_value(bass);
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

    /// Map an absolute song event to (when, dur) relative to `now`, clipped so
    /// that events overlapping the seek point start immediately. Returns None
    /// for events that are already over at `start_tick`.
    fn place(&self, abs_tick: u32, dur_ticks: u32, start_tick: u32, now: f64, spt: f64) -> Option<(f64, f64)> {
        let end = abs_tick + dur_ticks;
        if end <= start_tick {
            return None;
        }
        let eff = abs_tick.max(start_tick);
        let when = now + (eff - start_tick) as f64 * spt;
        let dur = (end - eff) as f64 * spt;
        Some((when, dur.max(0.03)))
    }

    /// (Re)build the schedule from `start_tick` for the chosen parts and tempo.
    pub fn schedule(&mut self, song: &Song, tempo_factor: f32, parts: &Parts, start_tick: u32) {
        self.clear();
        let bpm = (song.tempo_bpm as f32 * tempo_factor).max(1.0);
        let spt = 60.0 / (bpm as f64 * PPQ as f64);
        let now = self.ctx.current_time() + 0.12;
        self.origin = now;
        self.origin_tick = start_tick;
        self.sec_per_tick = spt;

        let melody_end = song.melody.iter().map(|n| n.tick + n.dur).max().unwrap_or(0);

        if parts.melody_on {
            let instr = PRESETS[parts.melody_instr.min(PRESETS.len() - 1)];
            for n in &song.melody {
                if let Some((when, dur)) = self.place(n.tick, n.dur, start_tick, now, spt) {
                    let peak = 0.18 + (n.vel as f32 / 127.0) * 0.12;
                    self.note(BusKind::Melody, n.pitch, when, dur, peak, &instr);
                }
            }
        }

        // Accompaniment comes from the `leadsheet` arranger: it lays out bass +
        // comped chords on a style's pattern beats, and makes substyle-B sections
        // busier using the decoded A/B part markers. We keep the user's per-part
        // instrument choice, on/off toggles and gain buses.
        let cells = chord_cells(song);
        let form_ticks = cells.iter().map(|(s, d, _)| s + d).max().unwrap_or(0);
        let total_end = melody_end.max(form_ticks);
        if (parts.chords_on || parts.bass_on) && form_ticks > 0 {
            use leadsheet::arrange::Part as AP;
            let chord_i = PRESETS[parts.chords_instr.min(PRESETS.len() - 1)];
            let bass_i = PRESETS[parts.bass_instr.min(PRESETS.len() - 1)];
            let events = leadsheet::arrange::arrange(song, &leadsheet::style::Style::default());
            let mut base = 0u32;
            while base < total_end {
                for e in &events {
                    match e.part {
                        AP::Comp if parts.chords_on => {
                            if let Some((when, dur)) = self.place(base + e.tick, e.dur, start_tick, now, spt) {
                                self.note(BusKind::Chords, e.pitch, when, dur.min(2.0), 0.06, &chord_i);
                            }
                        }
                        AP::Bass if parts.bass_on => {
                            if let Some((when, dur)) = self.place(base + e.tick, e.dur, start_tick, now, spt) {
                                self.note(BusKind::Bass, e.pitch, when, dur.min(1.2), 0.13, &bass_i);
                            }
                        }
                        _ => {}
                    }
                }
                base += form_ticks;
            }
        }

        self.end_tick = total_end;
        self.scheduled = true;
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

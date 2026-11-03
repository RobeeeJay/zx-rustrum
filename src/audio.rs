//! Sound: the 1-bit beeper, the AY-3-8912 of the 128K, and the mixer that
//! turns both into samples for the host audio device.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Samples waiting to be played. The emulator pushes, the audio callback pops.
pub type SharedQueue = Arc<Mutex<VecDeque<f32>>>;

/// The AY's 16 volume steps are logarithmic, roughly 3 dB apart.
const AY_LEVELS: [f32; 16] = [
    0.0000, 0.0137, 0.0205, 0.0291, 0.0423, 0.0618, 0.0847, 0.1369, 0.1691, 0.2647, 0.3527, 0.4499,
    0.5704, 0.6873, 0.8482, 1.0000,
];

/// AY-3-8912 (the 8910 without the second I/O port), as fitted to the 128K.
#[derive(Clone)]
pub struct Ay {
    pub regs: [u8; 16],
    pub selected: u8,

    tone_counter: [u32; 3],
    tone_state: [bool; 3],
    noise_counter: u32,
    noise_lfsr: u32,
    noise_state: bool,

    env_counter: u32,
    env_pos: u8,
    env_level: u8,
    env_attack: bool,
    env_holding: bool,

    /// Left-over fraction of an AY tick between calls to `advance`.
    tick_frac: f64,
}

impl Default for Ay {
    fn default() -> Self {
        Self::new()
    }
}

impl Ay {
    pub fn new() -> Self {
        Ay {
            regs: [0; 16],
            selected: 0,
            tone_counter: [0; 3],
            tone_state: [false; 3],
            noise_counter: 0,
            noise_lfsr: 1,
            noise_state: false,
            env_counter: 0,
            env_pos: 0,
            env_level: 0,
            env_attack: false,
            env_holding: false,
            tick_frac: 0.0,
        }
    }

    pub fn reset(&mut self) {
        *self = Ay::new();
    }

    pub fn write(&mut self, value: u8) {
        let r = (self.selected & 0x0f) as usize;
        let value = match r {
            1 | 3 | 5 | 13 => value & 0x0f,
            6 => value & 0x1f,
            8..=10 => value & 0x1f,
            _ => value,
        };
        self.regs[r] = value;
        if r == 13 {
            // Writing the shape register restarts the envelope.
            self.env_pos = 0;
            self.env_counter = 0;
            self.env_holding = false;
            self.env_attack = value & 0x04 != 0;
            self.env_level = if self.env_attack { 0 } else { 15 };
        }
    }

    pub fn read(&self) -> u8 {
        self.regs[(self.selected & 0x0f) as usize]
    }

    fn tone_period(&self, ch: usize) -> u32 {
        let lo = self.regs[ch * 2] as u32;
        let hi = (self.regs[ch * 2 + 1] & 0x0f) as u32;
        let p = (hi << 8) | lo;
        if p == 0 {
            1
        } else {
            p
        }
    }

    fn noise_period(&self) -> u32 {
        let p = (self.regs[6] & 0x1f) as u32;
        if p == 0 {
            1
        } else {
            p
        }
    }

    fn env_period(&self) -> u32 {
        let p = ((self.regs[12] as u32) << 8) | self.regs[11] as u32;
        if p == 0 {
            1
        } else {
            p
        }
    }

    /// Amplitude of one channel right now, 0.0..=1.0.
    fn channel_out(&self, ch: usize) -> f32 {
        let mixer = self.regs[7];
        let tone_off = mixer & (1 << ch) != 0;
        let noise_off = mixer & (8 << ch) != 0;
        let on = (self.tone_state[ch] || tone_off) && (self.noise_state || noise_off);
        if !on {
            return 0.0;
        }
        let amp = self.regs[8 + ch];
        let level = if amp & 0x10 != 0 {
            self.env_level
        } else {
            amp & 0x0f
        };
        AY_LEVELS[(level & 0x0f) as usize]
    }

    pub fn output(&self) -> f32 {
        (self.channel_out(0) + self.channel_out(1) + self.channel_out(2)) / 3.0
    }

    /// One internal step: the AY divides its clock by 8 for the tone, noise
    /// and envelope counters (giving clock/16 per full square wave cycle).
    fn tick(&mut self) {
        for ch in 0..3 {
            self.tone_counter[ch] += 1;
            if self.tone_counter[ch] >= self.tone_period(ch) {
                self.tone_counter[ch] = 0;
                self.tone_state[ch] = !self.tone_state[ch];
            }
        }

        self.noise_counter += 1;
        if self.noise_counter >= self.noise_period() {
            self.noise_counter = 0;
            // 17-bit LFSR, taps at bits 0 and 3.
            let bit = (self.noise_lfsr ^ (self.noise_lfsr >> 3)) & 1;
            self.noise_lfsr = (self.noise_lfsr >> 1) | (bit << 16);
            self.noise_state = self.noise_lfsr & 1 != 0;
        }

        self.env_counter += 1;
        if self.env_counter >= self.env_period() * 2 {
            self.env_counter = 0;
            self.env_step();
        }
    }

    fn env_step(&mut self) {
        if self.env_holding {
            return;
        }
        self.env_pos += 1;
        if self.env_pos >= 16 {
            self.env_pos = 0;
            let shape = self.regs[13];
            if shape & 0x08 == 0 {
                // Not CONTINUE: one pass, then silence.
                self.env_holding = true;
                self.env_level = 0;
                return;
            }
            if shape & 0x02 != 0 {
                self.env_attack = !self.env_attack;
            }
            if shape & 0x01 != 0 {
                self.env_holding = true;
                self.env_level = if self.env_attack { 15 } else { 0 };
                return;
            }
        }
        self.env_level = if self.env_attack {
            self.env_pos
        } else {
            15 - self.env_pos
        };
    }

    /// Advance by `clocks` AY clocks and return the average output over that
    /// interval, which keeps high frequencies from aliasing badly.
    pub fn advance(&mut self, clocks: f64) -> f32 {
        self.tick_frac += clocks / 8.0;
        let ticks = self.tick_frac.floor();
        self.tick_frac -= ticks;
        let ticks = ticks as u32;
        if ticks == 0 {
            return self.output();
        }
        let mut sum = 0.0;
        for _ in 0..ticks.min(4096) {
            self.tick();
            sum += self.output();
        }
        sum / ticks.min(4096) as f32
    }
}

/// How loud the speech chip is against the beeper. The Currah went into the
/// television's sound alongside the machine's own, at about the same level.
const SPEECH_GAIN: f32 = 0.6;

/// Mixes the beeper and the AY into a stream of samples.
#[derive(Clone)]
pub struct Audio {
    /// The master switch: off, and nothing at all is heard.
    pub enabled: bool,
    /// The beeper, which on a 48K is the whole of the machine's sound — and
    /// the tape's hiss with it, since that comes out of the same speaker.
    pub beeper_on: bool,
    /// The AY, and any add-on that is one: a Fuller Box gives a 48K its own
    /// and a 128K a second.
    pub ay_on: bool,
    /// What the other add-ons make: the SpecDrum's converter, the µSpeech's
    /// chip, and anything else that is neither the beeper nor a sound chip.
    pub hardware_on: bool,
    /// The Currah µSpeech's SP0256, when one is fitted. It runs on its own
    /// oscillator rather than the machine's clock, so it is clocked here in
    /// T-states of the machine and asked for a sample when its own time comes.
    pub speech: Option<crate::sp0256::Sp0256>,
    /// T-states between one sample of the speech chip and the next.
    speech_t: f64,
    speech_acc: f64,
    speech_level: f32,

    pub volume: f32,
    /// Mute automatically when not running at roughly normal speed, so
    /// fast-forwarding does not shriek.
    pub mute_off_speed: bool,
    pub speed_ok: bool,

    pub sample_rate: f64,
    /// Somewhere to keep a copy of every sample, while a video is being
    /// recorded. `None` the rest of the time: a recording of the sound is
    /// only wanted while something is recording it, and keeping one otherwise
    /// would be a growing buffer nobody reads.
    pub tap: Option<Vec<f32>>,
    /// The Scope window's probe: the last few hundred milliseconds of output,
    /// kept only while the window is open, since it costs a write per sample.
    pub scope: Option<crate::scope::Scope>,
    pub cpu_hz: f64,
    t_per_sample: f64,

    acc: f32,
    acc_t: f64,
    last_t: u64,

    /// Current beeper amplitude, from the last OUT to port $FE.
    pub beeper: f32,
    /// How loud the tape hisses, which is a level rather than a signal: the
    /// noise itself is made here, a sample at a time.
    pub tape_hiss: f32,
    /// The noise generator's state. It need not repeat the way the deck's own
    /// hiss does — nothing reads this but an ear.
    hiss_state: u32,
    pub ay: Ay,
    /// A second sound chip, for the add-ons that brought their own: the Fuller
    /// Audio Box gave a 48K the AY it did not have.
    pub extra_ay: Option<Ay>,
    /// An eight-bit converter's output, for the add-ons that had one. The
    /// SpecDrum is nothing but this: a byte written to a port is a sample, and
    /// the program feeds it drum sounds from memory as fast as it can.
    pub dac: f32,
    pub ay_present: bool,

    /// One-pole DC blocker, as the real machine's output is AC coupled: the
    /// beeper is a square wave between 0 and full, and without this every
    /// change of duty cycle would thump.
    dc_x1: f32,
    dc_y1: f32,
    /// Gain is ramped rather than switched, so muting does not click.
    gain: f32,

    queue: Option<SharedQueue>,
    pending: Vec<f32>,
    queue_cap: usize,
    pub produced: u64,
    pub dropped: u64,
    /// Peak level of the last flushed batch, for a level meter.
    pub peak: f32,
}

impl Audio {
    pub fn new(cpu_hz: f64) -> Self {
        Audio {
            enabled: true,
            beeper_on: true,
            ay_on: true,
            hardware_on: true,
            speech: None,
            speech_t: 358.0,
            speech_acc: 0.0,
            speech_level: 0.0,

            volume: 0.5,
            mute_off_speed: true,
            speed_ok: true,
            sample_rate: 48_000.0,
            tap: None,
            scope: None,
            cpu_hz,
            t_per_sample: cpu_hz / 48_000.0,
            acc: 0.0,
            acc_t: 0.0,
            last_t: 0,
            beeper: 0.0,
            tape_hiss: 0.0,
            hiss_state: 0x1234_5678,
            ay: Ay::new(),
            extra_ay: None,
            dac: 0.0,
            ay_present: false,
            dc_x1: 0.0,
            dc_y1: 0.0,
            gain: 0.0,
            queue: None,
            pending: Vec::with_capacity(256),
            queue_cap: 12_000,
            produced: 0,
            dropped: 0,
            peak: 0.0,
        }
    }

    pub fn attach(&mut self, queue: SharedQueue, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.t_per_sample = self.cpu_hz / sample_rate;
        self.queue_cap = (sample_rate * 0.25) as usize;
        self.queue = Some(queue);
    }

    /// Whether this is the machine the sound card is listening to.
    pub fn attached(&self) -> bool {
        self.queue.is_some()
    }

    /// Take over the sound output from the machine being replaced.
    ///
    /// A restored quicksave is a copy of the machine as it was, chips and all,
    /// but the sound card and the listener's settings belong to now: the
    /// queue, the rate, the volume and the three switches come from the
    /// machine going away. What the copy had not yet sent when it was taken is
    /// thrown out rather than played — it is sound from the past, and it would
    /// come out as a blip on top of the present.
    pub fn take_output_from(&mut self, live: &mut Audio) {
        live.flush();
        self.queue = live.queue.take();
        self.sample_rate = live.sample_rate;
        self.t_per_sample = self.cpu_hz / self.sample_rate;
        self.queue_cap = live.queue_cap;
        self.enabled = live.enabled;
        self.beeper_on = live.beeper_on;
        self.ay_on = live.ay_on;
        self.hardware_on = live.hardware_on;
        self.volume = live.volume;
        self.mute_off_speed = live.mute_off_speed;
        self.speed_ok = live.speed_ok;
        self.gain = live.gain;
        self.tap = live.tap.take();
        self.scope = live.scope.take();
        self.pending.clear();
    }

    /// Stop sending samples anywhere.
    ///
    /// The queue is shared with the sound device, so a copy of a machine holds
    /// the same one and would play its own sound over the real machine's.
    pub fn detach(&mut self) {
        self.queue = None;
    }

    /// How many T-states go into one sample, which is the clock the mixer
    /// believes it is counting.
    pub fn t_per_sample(&self) -> f64 {
        self.t_per_sample
    }

    pub fn set_cpu_hz(&mut self, hz: f64) {
        self.cpu_hz = hz;
        self.t_per_sample = hz / self.sample_rate;
    }

    /// Called when the emulated clock is rewound (reset, model change).
    pub fn rebase(&mut self, now: u64) {
        self.last_t = now;
        self.acc = 0.0;
        self.acc_t = 0.0;
    }

    /// Generate samples up to absolute T-state `now`.
    pub fn advance_to(&mut self, now: u64) {
        if now <= self.last_t {
            // Never move the clock backwards: doing so would replay the same
            // stretch of time twice on the next call.
            return;
        }
        let mut dt = (now - self.last_t) as f64;
        self.last_t = now;

        // A very long gap (paused, or single-stepping) would produce a burst
        // of stale samples; skip the audio for it but keep the clock in step.
        if dt > self.cpu_hz * 0.25 {
            self.acc = 0.0;
            self.acc_t = 0.0;
            return;
        }

        while dt > 0.0 {
            let chunk = dt.min(self.t_per_sample - self.acc_t);
            let ay_out = if self.ay_present {
                // The AY runs at half the CPU clock on a 128K.
                self.ay.advance(chunk / 2.0)
            } else {
                0.0
            };
            // What an add-on is making, if one is fitted: another sound chip,
            // and a converter somebody is feeding samples to.
            let extra = match &mut self.extra_ay {
                Some(ay) => ay.advance(chunk / 2.0),
                None => 0.0,
            };
            // The hiss is drawn whether or not it is wanted, so muting the
            // beeper does not change the noise the tape makes when it comes
            // back — it is one sequence, not one per switch.
            let hiss = self.hiss();
            let beeper = if self.beeper_on {
                self.beeper + hiss
            } else {
                0.0
            };
            let chips = if self.ay_on { ay_out + extra } else { 0.0 };
            // The speech chip, at its own rate: a sample every 312 clocks of
            // its oscillator, held between times.
            let mut level = self.speech_level;
            if let Some(chip) = self.speech.as_mut() {
                self.speech_acc += chunk;
                while self.speech_acc >= self.speech_t {
                    self.speech_acc -= self.speech_t;
                    level = f32::from(chip.sample()) / 32768.0 * SPEECH_GAIN;
                }
                self.speech_level = level;
            }
            let boxes = if self.hardware_on {
                self.dac + level
            } else {
                0.0
            };
            self.acc += (beeper + chips + boxes) * chunk as f32;
            self.acc_t += chunk;
            dt -= chunk;
            if self.acc_t >= self.t_per_sample - 1e-9 {
                let sample = self.acc / self.t_per_sample as f32;
                self.push(sample);
                self.acc = 0.0;
                self.acc_t = 0.0;
            }
        }
    }

    /// How fast the speech chip's own oscillator runs, in the machine's
    /// T-states. The µSpeech has two: about 3.05MHz, and 7% above it.
    pub fn set_speech_pitch(&mut self, high: bool) {
        let hz = if high { 3_260_000.0 } else { 3_050_000.0 };
        self.speech_t = self.cpu_hz * f64::from(crate::sp0256::CLOCK_DIVIDER) / hz;
    }

    /// A sample of hiss: white noise at whatever the deck says it is worth.
    fn hiss(&mut self) -> f32 {
        if self.tape_hiss <= 0.0 {
            return 0.0;
        }
        // xorshift, which is white enough for a hiss and costs nothing.
        self.hiss_state ^= self.hiss_state << 13;
        self.hiss_state ^= self.hiss_state >> 17;
        self.hiss_state ^= self.hiss_state << 5;
        let unit = (self.hiss_state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0;
        unit * self.tape_hiss
    }

    fn push(&mut self, sample: f32) {
        // Remove the DC component: only changes in level are audible.
        let blocked = sample - self.dc_x1 + 0.9995 * self.dc_y1;
        self.dc_x1 = sample;
        self.dc_y1 = blocked;

        let muted = !self.enabled || (self.mute_off_speed && !self.speed_ok);
        let target = if muted { 0.0 } else { self.volume };
        // ~10 ms ramp at 48 kHz, so mute, unmute and volume moves are silent.
        self.gain += (target - self.gain) * 0.002;

        let out = blocked * self.gain;
        self.peak = self.peak.max(out.abs());
        let out = out.clamp(-1.0, 1.0);
        if let Some(tap) = &mut self.tap {
            tap.push(out);
        }
        if let Some(scope) = &mut self.scope {
            scope.push(out);
        }
        self.pending.push(out);
        self.produced += 1;
        if self.pending.len() >= 128 {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        let Some(queue) = &self.queue else {
            self.pending.clear();
            return;
        };
        if let Ok(mut q) = queue.lock() {
            for s in self.pending.drain(..) {
                if q.len() >= self.queue_cap {
                    // Running faster than real time: throw the oldest away so
                    // latency does not grow without bound.
                    q.pop_front();
                    self.dropped += 1;
                }
                q.push_back(s);
            }
        } else {
            self.pending.clear();
        }
    }

    /// How far ahead of the device the queue is, in seconds.
    pub fn latency(&self) -> f64 {
        self.queue_len() as f64 / self.sample_rate
    }

    /// Scale factor for how much emulation to run this host frame so the
    /// queue stays near `target` seconds deep: a little more when the device
    /// is starving, a little less when it is backing up. Without this the
    /// queue drifts until it either underruns (clicks) or overflows (dropped
    /// samples, also clicks).
    pub fn pace(&self, target: f64) -> f32 {
        if self.queue.is_none() || target <= 0.0 {
            return 1.0;
        }
        let error = (target - self.latency()) / target;
        (1.0 + error * 0.1).clamp(0.9, 1.1) as f32
    }

    pub fn queue_len(&self) -> usize {
        self.queue
            .as_ref()
            .and_then(|q| q.lock().ok().map(|q| q.len()))
            .unwrap_or(0)
    }
}

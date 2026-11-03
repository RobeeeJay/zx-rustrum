//! The oscilloscope and spectrum analyser's measurements: a copy of the last
//! few hundred milliseconds of what the mixer sent to the sound card, where a
//! trace starts, and how loud each frequency in it is.
//!
//! The probe is on the mixer's output, after the DC blocker, the volume and the
//! mute, so the trace is the signal the speaker is given and nothing cleaner.

use std::f32::consts::PI;

/// Samples kept: 340ms at 96kHz, enough for the slowest timebase and the
/// longest transform with room to look for a trigger before them.
pub const KEPT: usize = 1 << 15;
/// Samples in one transform: about 85ms at 48kHz, which puts the bins 11.7Hz
/// apart — fine enough to tell the beeper's notes apart at the bottom end.
pub const FFT: usize = 4096;
/// How far a signal has to fall below the trigger level before a rise through
/// it counts again, as a fraction of full scale. Without it the noise on a
/// tape's hiss triggers on every wobble and the trace never stands still.
const HYSTERESIS: f32 = 0.02;

/// The last `KEPT` samples, oldest overwritten first.
#[derive(Clone)]
pub struct Scope {
    ring: Vec<f32>,
    /// Samples recorded since the scope was switched on.
    pub written: u64,
}

impl Default for Scope {
    fn default() -> Self {
        Scope {
            ring: vec![0.0; KEPT],
            written: 0,
        }
    }
}

impl Scope {
    pub fn push(&mut self, sample: f32) {
        self.ring[(self.written % KEPT as u64) as usize] = sample;
        self.written += 1;
    }

    /// The newest `n` samples in the order they were played, padded with
    /// silence at the front when fewer than that have been recorded.
    pub fn latest(&self, n: usize) -> Vec<f32> {
        let n = n.min(KEPT);
        let have = (self.written as usize).min(n);
        let mut out = vec![0.0; n - have];
        let end = self.written as usize;
        out.extend((end - have..end).map(|i| self.ring[i % KEPT]));
        out
    }
}

/// Where a trace of `width` samples should start in `samples` so that it
/// begins on a rise through `level`: the latest such rise that leaves a whole
/// trace after it, so the picture is as fresh as it can be and a steady note
/// stands still. `None` when nothing rises through the level.
pub fn trigger(samples: &[f32], width: usize, level: f32) -> Option<usize> {
    let last = samples.len().checked_sub(width)?;
    let mut armed = false;
    let mut found = None;
    for (i, &s) in samples.iter().enumerate().take(last + 1) {
        if s < level - HYSTERESIS {
            armed = true;
        } else if armed && s >= level {
            found = Some(i);
            armed = false;
        }
    }
    found
}

/// In-place radix-2 transform; `re.len()` must be a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let step = -2.0 * PI / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (step * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// How loud each frequency is in `samples` (a power of two of them), in
/// decibels against full scale: a sine from -1 to +1 reads 0dB in its bin.
/// Bin `k` is `k * rate / samples.len()` hertz; there are half as many bins
/// as samples. A Hann window keeps one loud note from smearing over the rest.
pub fn spectrum(samples: &[f32]) -> Vec<f32> {
    let n = samples.len();
    let window: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
        .collect();
    let gain: f32 = window.iter().sum::<f32>() / 2.0;
    let mut re: Vec<f32> = samples.iter().zip(&window).map(|(s, w)| s * w).collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    (0..n / 2)
        .map(|k| {
            let magnitude = (re[k] * re[k] + im[k] * im[k]).sqrt() / gain;
            20.0 * magnitude.max(1e-9).log10()
        })
        .collect()
}

/// The frequency of the loudest bin above the lowest few, refined between
/// bins by fitting a parabola through it and its neighbours, and how loud it
/// is. `None` for silence. The bottom bins are left out because the DC
/// blocker's slow recovery lives there and is not a note.
pub fn loudest(db: &[f32], rate: f64) -> Option<(f64, f32)> {
    let n = db.len() * 2;
    let (k, &peak) = db
        .iter()
        .enumerate()
        .skip(3)
        .take(db.len().saturating_sub(4))
        .max_by(|a, b| a.1.total_cmp(b.1))?;
    if peak < -90.0 {
        return None;
    }
    let (a, b, c) = (db[k - 1], peak, db[k + 1]);
    let shift = 0.5 * (a - c) / (a - 2.0 * b + c);
    let shift = if shift.is_finite() {
        shift.clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some(((k as f64 + shift as f64) * rate / n as f64, peak))
}

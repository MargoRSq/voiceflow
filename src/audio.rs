use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

pub const SAMPLE_RATE: u32 = 16000;

/// One meter sample per 10 ms of audio, which is what the overlay scrolls.
const WINDOW: usize = SAMPLE_RATE as usize / 100;

pub enum Cmd {
    Start,
    Stop,
}

/// Owns the cpal stream on its own thread (`cpal::Stream` is not `Send`).
/// The device is opened on `Start` and closed on `Stop` — PipeWire's ALSA
/// device rejects `snd_pcm_pause`, and this keeps the mic shut between
/// dictations anyway.
pub fn spawn(
    tx: Sender<crate::asr::Msg>,
    cmds: Receiver<Cmd>,
    state: Arc<Mutex<crate::State>>,
) -> Result<()> {
    probe().context("no usable input device")?;

    std::thread::spawn(move || {
        let mut stream: Option<cpal::Stream> = None;
        while let Ok(cmd) = cmds.recv() {
            match cmd {
                Cmd::Start => {
                    if stream.is_none() {
                        let opened = build(tx.clone(), state.clone()).and_then(|s| {
                            s.play()?;
                            Ok(s)
                        });
                        match opened {
                            Ok(s) => stream = Some(s),
                            Err(e) => {
                                // Leaving `recording` set would show a live
                                // overlay that can never produce a word.
                                eprintln!("audio: {e}");
                                if let Ok(mut st) = state.lock() {
                                    st.recording = false;
                                    st.started_at = None;
                                }
                            }
                        }
                    }
                }
                Cmd::Stop => stream = None,
            }
        }
    });
    Ok(())
}

fn probe() -> Result<()> {
    let device = cpal::default_host()
        .default_input_device()
        .context("no input device")?;
    let config = device.default_input_config()?;
    let name = device
        .description()
        .map(|d| d.name().to_string())
        .unwrap_or_default();
    eprintln!(
        "audio: {name} @ {} Hz {} ch",
        config.sample_rate(),
        config.channels()
    );
    Ok(())
}

fn build(tx: Sender<crate::asr::Msg>, state: Arc<Mutex<crate::State>>) -> Result<cpal::Stream> {
    let device = cpal::default_host()
        .default_input_device()
        .context("no input device")?;
    let config = device.default_input_config()?;
    let mut down = Downmix::new(config.sample_rate(), config.channels() as usize);
    let mut meter = Meter::default();
    let cfg = config.config();

    // Devices are not all f32; building an f32 stream on an I16 one just fails.
    macro_rules! stream {
        ($t:ty, $to_f32:expr) => {{
            let to_f32: fn($t) -> f32 = $to_f32;
            device.build_input_stream(
                cfg,
                move |data: &[$t], _: &_| {
                    let mono: Vec<f32> = data.iter().copied().map(to_f32).collect();
                    let out = down.process(&mono);
                    if !out.is_empty() {
                        meter.feed(&out, &state);
                        let _ = tx.send(crate::asr::Msg::Samples(out));
                    }
                },
                |e| eprintln!("audio: {e}"),
                None,
            )?
        }};
    }

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => stream!(f32, |s| s),
        cpal::SampleFormat::I16 => stream!(i16, |s| s as f32 / 32768.0),
        cpal::SampleFormat::U16 => stream!(u16, |s| (s as f32 - 32768.0) / 32768.0),
        other => bail!("unsupported sample format {other:?}"),
    };
    Ok(stream)
}

/// Rolling loudness for the overlay. Kept here rather than in the ASR thread
/// because that one only wakes up every 560 ms — far too coarse to look alive.
#[derive(Default)]
struct Meter {
    buf: Vec<f32>,
}

impl Meter {
    fn feed(&mut self, samples: &[f32], state: &Arc<Mutex<crate::State>>) {
        self.buf.extend_from_slice(samples);
        if self.buf.len() < WINDOW {
            return;
        }

        let mut levels = Vec::new();
        while self.buf.len() >= WINDOW {
            let w: Vec<f32> = self.buf.drain(..WINDOW).collect();
            let rms = (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt();
            // -50 dBFS (room tone) .. -12 dBFS (loud speech) -> 0..1
            let db = 20.0 * rms.max(1e-6).log10();
            levels.push(((db + 50.0) / 38.0).clamp(0.0, 1.0));
        }

        if let Ok(mut st) = state.lock() {
            for l in levels {
                st.push_level(l);
            }
        }
    }
}

/// Interleaved multi-channel at `in_rate` -> mono at 16 kHz.
/// Box-averaging doubles as a crude anti-aliasing filter.
struct Downmix {
    channels: usize,
    per_out: f32,
    sum: f32,
    /// Samples actually summed, for the average.
    n: u32,
    /// Fractional sample clock, for the output rate.
    acc: f32,
}

impl Downmix {
    fn new(in_rate: u32, channels: usize) -> Self {
        Self {
            channels,
            per_out: in_rate as f32 / SAMPLE_RATE as f32,
            sum: 0.0,
            n: 0,
            acc: 0.0,
        }
    }

    fn process(&mut self, data: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(data.len() / self.channels + 1);
        for frame in data.chunks(self.channels) {
            let mono = frame.iter().sum::<f32>() / self.channels as f32;
            self.sum += mono;
            self.n += 1;
            self.acc += 1.0;
            if self.acc >= self.per_out {
                out.push(self.sum / self.n as f32);
                self.sum = 0.0;
                self.n = 0;
                // Carry the remainder. Zeroing it would round `per_out` up to a
                // whole number of input samples: 44100 Hz would come out at
                // 14700 Hz, not 16000, and every dictation would be pitch-shifted.
                self.acc -= self.per_out;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of input must yield one second at 16 kHz, whatever the rate.
    fn rate_of(in_rate: u32, channels: usize) -> usize {
        let mut d = Downmix::new(in_rate, channels);
        let frame = vec![0.5f32; channels];
        let input: Vec<f32> = (0..in_rate).flat_map(|_| frame.clone()).collect();
        d.process(&input).len()
    }

    #[test]
    fn resamples_integer_ratio() {
        assert_eq!(rate_of(48000, 2), 16000);
    }

    #[test]
    fn resamples_fractional_ratio() {
        // 44100 / 16000 = 2.756: the old code emitted every 3rd sample (14700).
        let n = rate_of(44100, 2) as i64;
        assert!((n - 16000).abs() <= 1, "got {n} samples, want ~16000");
    }

    #[test]
    fn already_at_target_rate_passes_through() {
        assert_eq!(rate_of(16000, 1), 16000);
    }

    #[test]
    fn averaging_preserves_amplitude() {
        let mut d = Downmix::new(44100, 1);
        let out = d.process(&vec![0.5f32; 44100]);
        let peak = out.iter().cloned().fold(0.0f32, f32::max);
        assert!((peak - 0.5).abs() < 1e-4, "amplitude drifted to {peak}");
    }
}

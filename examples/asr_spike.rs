use anyhow::{Context, Result, bail};
use parakeet_rs::Nemotron;
use std::time::Instant;

const CHUNK: usize = 8960; // 560 ms @ 16 kHz

fn main() -> Result<()> {
    let wav_path = std::env::args()
        .nth(1)
        .context("usage: asr_spike <file.wav> [lang]")?;
    let lang = std::env::args().nth(2).unwrap_or_else(|| "ru-RU".into());

    let model_dir = std::env::var("VOICEFLOW_MODEL").unwrap_or_else(|_| {
        format!(
            "{}/.local/share/voiceflow/models/nemotron_multi",
            std::env::var("HOME").unwrap_or_default()
        )
    });

    let t0 = Instant::now();
    let mut model = Nemotron::from_pretrained(&model_dir, None)
        .with_context(|| format!("loading model from {model_dir}"))?;
    eprintln!("model loaded in {:.2}s", t0.elapsed().as_secs_f64());

    model.set_target_lang(&lang)?;
    eprintln!("target lang: {lang}\n");

    let samples = read_wav_16k_mono(&wav_path)?;
    eprintln!(
        "audio: {} samples = {:.2}s\n",
        samples.len(),
        samples.len() as f64 / 16000.0
    );

    let mut infer_total = 0.0f64;
    let mut chunks = 0usize;

    for (i, c) in samples.chunks(CHUNK).enumerate() {
        let mut buf = c.to_vec();
        buf.resize(CHUNK, 0.0);

        let t = Instant::now();
        let out = model.transcribe_chunk(&buf)?;
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        infer_total += ms;
        chunks += 1;

        println!("[{i:>2}] {ms:>6.1} ms  {out:?}");
    }

    for _ in 0..3 {
        model.transcribe_chunk(&vec![0.0; CHUNK])?;
    }

    println!("\nFINAL: {}", model.get_transcript());
    eprintln!(
        "\navg {:.1} ms/chunk over {} chunks  (budget 560 ms, RTF {:.3})",
        infer_total / chunks as f64,
        chunks,
        (infer_total / chunks as f64) / 560.0
    );
    Ok(())
}

fn read_wav_16k_mono(path: &str) -> Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    if spec.sample_rate != 16000 || spec.channels != 1 {
        bail!(
            "need 16 kHz mono, got {} Hz {} ch",
            spec.sample_rate,
            spec.channels
        );
    }
    let samples = match spec.sample_format {
        hound::SampleFormat::Int => r
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
    };
    Ok(samples)
}

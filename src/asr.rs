use crate::State;
use anyhow::{Context, Result};
use parakeet_rs::Nemotron;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

pub const CHUNK: usize = 8960; // 560 ms @ 16 kHz

pub enum Msg {
    /// Sent on toggle-on. Drops anything the audio thread pushed after the
    /// previous `Finish`: `Cmd::Stop` and `Finish` travel on different channels,
    /// so a cpal callback already in flight lands here late and would otherwise
    /// be prepended to the next dictation.
    Start,
    Samples(Vec<f32>),
    /// Recording stopped: flush the tail and hand back the final transcript.
    Finish,
}

pub fn model_dir() -> String {
    std::env::var("VOICEFLOW_MODEL").unwrap_or_else(|_| {
        format!(
            "{}/.local/share/voiceflow/models/nemotron_multi",
            std::env::var("HOME").unwrap_or_default()
        )
    })
}

/// Loads the model up front (~2.5 s) and streams chunks for the lifetime of the
/// daemon, so a toggle never pays the load cost.
pub fn spawn(state: Arc<Mutex<State>>, rx: Receiver<Msg>, lang: String) -> Result<()> {
    let dir = model_dir();
    let mut model =
        Nemotron::from_pretrained(&dir, None).with_context(|| format!("loading model {dir}"))?;
    model.set_target_lang(&lang)?;
    eprintln!("asr: model ready, lang={lang}");

    // History is a nice-to-have: a broken db must not stop dictation working.
    let db = match crate::store::open() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("asr: history disabled ({e})");
            None
        }
    };

    std::thread::spawn(move || {
        let mut buf: Vec<f32> = Vec::with_capacity(CHUNK * 2);

        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Start => buf.clear(),
                Msg::Samples(s) => {
                    buf.extend_from_slice(&s);
                    while buf.len() >= CHUNK {
                        let chunk: Vec<f32> = buf.drain(..CHUNK).collect();
                        match model.transcribe_chunk(&chunk) {
                            Ok(_) => {
                                let text = model.get_transcript();
                                state.lock().unwrap().partial = text;
                            }
                            Err(e) => eprintln!("asr: {e}"),
                        }
                    }
                }
                Msg::Finish => {
                    // Read the duration before flushing: the flush runs four
                    // model passes and the user can start a new recording during
                    // them, whose `started_at` must not be consumed here.
                    let spoken_ms = state
                        .lock()
                        .ok()
                        .and_then(|mut st| st.started_at.take())
                        .map(|t| t.elapsed().as_millis() as i64)
                        .unwrap_or(0);

                    if !buf.is_empty() {
                        let mut tail = std::mem::take(&mut buf);
                        tail.resize(CHUNK, 0.0);
                        let _ = model.transcribe_chunk(&tail);
                    }
                    for _ in 0..3 {
                        let _ = model.transcribe_chunk(&vec![0.0; CHUNK]);
                    }
                    let text = model.get_transcript().trim().to_string();
                    model.reset();
                    buf.clear();

                    {
                        let mut st = state.lock().unwrap();
                        // Only clear the preview if no new recording began while
                        // the flush was running.
                        if !st.recording {
                            st.partial.clear();
                        }
                        st.final_text = Some(text.clone());
                    }

                    if let Some(db) = &db {
                        if let Err(e) = crate::store::insert(db, &text, &lang, spoken_ms) {
                            eprintln!("asr: history write failed: {e}");
                        }
                    }
                }
            }
        }
    });

    Ok(())
}

//! The speech engine: owns the loaded model on a dedicated thread and serves
//! transcription requests in order.

use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use transcribe_rs::onnx::Quantization;
use transcribe_rs::onnx::canary::CanaryModel;
use transcribe_rs::onnx::parakeet::ParakeetModel;
use transcribe_rs::{SpeechModel, TranscribeOptions};

use crate::learn::{self, Heard, Pieces};
use crate::models::{self, Family, ModelInfo};

#[derive(Clone, Debug, PartialEq)]
pub enum EngineStatus {
    /// No model selected or the selected one isn't downloaded.
    Missing(String),
    Loading(String),
    Ready { id: String, load_ms: u64 },
    /// Model was unloaded to save memory; it reloads on next use.
    Unloaded(String),
    Failed { id: String, error: String },
}

#[derive(Clone, Debug)]
pub struct Transcript {
    pub request: u64,
    pub text: String,
    pub audio_secs: f32,
    pub infer_ms: u64,
    pub model: String,
    /// The words as the model heard them, with its confidence in each
    /// (Parakeet only; learning from corrections uses them).
    pub heard: Option<Vec<Heard>>,
}

pub enum EngineEvent {
    Status(EngineStatus),
    /// The loaded model's vocabulary, which learning needs. None for models
    /// learning doesn't support.
    Pieces(Option<Arc<Pieces>>),
    Done(Transcript),
    Failed { request: u64, error: String },
}

enum Cmd {
    Select { id: String, threads: Option<usize>, idle_unload: Option<Duration> },
    Transcribe { request: u64, samples: Vec<f32>, language: Option<String> },
    Unload,
}

#[derive(Clone)]
pub struct Engine {
    tx: Sender<Cmd>,
}

impl Engine {
    pub fn spawn(events: Sender<EngineEvent>) -> Engine {
        let (tx, rx) = unbounded();
        thread::Builder::new()
            .name("bars-engine".into())
            .spawn(move || Worker::new(events).run(rx))
            .expect("spawn engine thread");
        Engine { tx }
    }

    /// Choose (and eagerly load) a model.
    pub fn select(&self, id: &str, threads: Option<usize>, unload_after_mins: u32) {
        let idle_unload = (unload_after_mins > 0)
            .then(|| Duration::from_secs(unload_after_mins as u64 * 60));
        let _ = self.tx.send(Cmd::Select {
            id: id.to_string(),
            threads,
            idle_unload,
        });
    }

    pub fn transcribe(&self, request: u64, samples: Vec<f32>, language: Option<String>) {
        let _ = self.tx.send(Cmd::Transcribe {
            request,
            samples,
            language,
        });
    }

    pub fn unload(&self) {
        let _ = self.tx.send(Cmd::Unload);
    }
}

/// Parakeet is kept by type: learning reads its confidences.
enum Model {
    Parakeet(ParakeetModel),
    Other(Box<dyn SpeechModel + Send>),
}

impl Model {
    fn speech(&mut self) -> &mut dyn SpeechModel {
        match self {
            Model::Parakeet(m) => m,
            Model::Other(m) => m.as_mut(),
        }
    }

    /// The words of the last transcription, with confidences.
    fn heard(&mut self) -> Option<Vec<Heard>> {
        let Model::Parakeet(m) = self else { return None };
        let record = m.take_decode()?;
        let vocab = m.vocab();
        let tokens: Vec<&str> = record
            .tokens
            .iter()
            .map(|&t| vocab.get(t as usize).map_or("", String::as_str))
            .collect();
        Some(learn::heard_words(&tokens, &record.probs))
    }
}

struct Loaded {
    id: String,
    family: Family,
    model: Model,
}

struct Worker {
    events: Sender<EngineEvent>,
    selected: Option<String>,
    threads: Option<usize>,
    idle_unload: Option<Duration>,
    loaded: Option<Loaded>,
    last_used: Instant,
}

impl Worker {
    fn new(events: Sender<EngineEvent>) -> Self {
        Worker {
            events,
            selected: None,
            threads: None,
            idle_unload: None,
            loaded: None,
            last_used: Instant::now(),
        }
    }

    fn status(&self, s: EngineStatus) {
        let _ = self.events.send(EngineEvent::Status(s));
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        loop {
            let cmd = match rx.recv_timeout(Duration::from_secs(30)) {
                Ok(c) => c,
                Err(RecvTimeoutError::Timeout) => {
                    if let (Some(limit), Some(l)) = (self.idle_unload, &self.loaded) {
                        if self.last_used.elapsed() > limit {
                            let id = l.id.clone();
                            self.loaded = None;
                            log::info!("unloaded idle model {id}");
                            self.status(EngineStatus::Unloaded(id));
                        }
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };
            match cmd {
                Cmd::Select { id, threads, idle_unload } => {
                    let changed = self.selected.as_deref() != Some(&id) || self.threads != threads;
                    self.selected = Some(id);
                    self.threads = threads;
                    self.idle_unload = idle_unload;
                    if changed || self.loaded.is_none() {
                        self.loaded = None;
                        let _ = self.ensure_loaded();
                    }
                }
                Cmd::Transcribe { request, samples, language } => {
                    let result = self.ensure_loaded().and_then(|()| self.run_one(&samples, language));
                    self.last_used = Instant::now();
                    match result {
                        Ok((text, infer_ms)) => {
                            let model = self.loaded.as_ref().map(|l| l.id.clone()).unwrap_or_default();
                            let heard = self.loaded.as_mut().and_then(|l| l.model.heard());
                            let _ = self.events.send(EngineEvent::Done(Transcript {
                                request,
                                text,
                                audio_secs: samples.len() as f32 / crate::audio::SAMPLE_RATE as f32,
                                infer_ms,
                                model,
                                heard,
                            }));
                        }
                        Err(e) => {
                            let _ = self.events.send(EngineEvent::Failed {
                                request,
                                error: format!("{e:#}"),
                            });
                        }
                    }
                }
                Cmd::Unload => {
                    if let Some(l) = self.loaded.take() {
                        self.status(EngineStatus::Unloaded(l.id));
                    }
                }
            }
        }
    }

    fn ensure_loaded(&mut self) -> anyhow::Result<()> {
        if self.loaded.is_some() {
            return Ok(());
        }
        let id = self.selected.clone().ok_or_else(|| anyhow!("no model selected"))?;
        let Some(info) = models::find(&id) else {
            self.status(EngineStatus::Missing(id.clone()));
            return Err(anyhow!("unknown model {id}"));
        };
        if !info.is_installed() {
            self.status(EngineStatus::Missing(id.clone()));
            return Err(anyhow!("{} is not downloaded yet", info.name));
        }
        self.status(EngineStatus::Loading(id.clone()));
        let started = Instant::now();
        match load_model(info, &info.dir()) {
            Ok(mut model) => {
                // Warm-up pass: the first inference pays for allocator/kernel
                // setup; do it now instead of on the user's first dictation.
                let _ = model.speech().transcribe(&vec![0.0; 16_000], &TranscribeOptions::default());
                let _ = model.heard();
                let pieces = match &model {
                    Model::Parakeet(m) => Some(Arc::new(Pieces::from_vocab(m.vocab()))),
                    Model::Other(_) => None,
                };
                let _ = self.events.send(EngineEvent::Pieces(pieces));
                let load_ms = started.elapsed().as_millis() as u64;
                log::info!("loaded {} in {load_ms} ms", info.id);
                self.loaded = Some(Loaded {
                    id: id.clone(),
                    family: info.family,
                    model,
                });
                self.last_used = Instant::now();
                self.status(EngineStatus::Ready { id, load_ms });
                Ok(())
            }
            Err(e) => {
                let error = format!("{e:#}");
                log::error!("loading {id} failed: {error}");
                self.status(EngineStatus::Failed { id, error: error.clone() });
                Err(anyhow!(error))
            }
        }
    }

    fn run_one(&mut self, samples: &[f32], language: Option<String>) -> anyhow::Result<(String, u64)> {
        let loaded = self.loaded.as_mut().ok_or_else(|| anyhow!("model not loaded"))?;
        let started = Instant::now();
        let options = TranscribeOptions {
            language: language.clone(),
            // A little trailing silence helps decoders finish the last word.
            trailing_silence_ms: Some(200),
            ..Default::default()
        };
        let options = match loaded.family {
            // Canary needs an explicit source language; default to English.
            Family::Canary => TranscribeOptions {
                language: Some(language.unwrap_or_else(|| "en".into())),
                ..options
            },
            _ => options,
        };
        let result = loaded.model.speech().transcribe(samples, &options).map_err(|e| anyhow!("{e}"))?;
        Ok((result.text.trim().to_string(), started.elapsed().as_millis() as u64))
    }
}

fn load_model(info: &ModelInfo, dir: &Path) -> anyhow::Result<Model> {
    Ok(match info.family {
        Family::Parakeet => Model::Parakeet(ParakeetModel::load(dir, &Quantization::Int8).map_err(|e| anyhow!("{e}"))?),
        _ => Model::Other(load(info, dir, None)?),
    })
}

/// Loads a model by family. ONNX Runtime picks its intra-op thread count
/// (physical cores) itself.
pub fn load(info: &ModelInfo, dir: &Path, _threads: Option<usize>) -> anyhow::Result<Box<dyn SpeechModel + Send>> {
    let q = Quantization::Int8;
    let model: Box<dyn SpeechModel + Send> = match info.family {
        Family::Parakeet => Box::new(ParakeetModel::load(dir, &q).map_err(|e| anyhow!("{e}"))?),
        Family::Canary => Box::new(CanaryModel::load(dir, &q).map_err(|e| anyhow!("{e}"))?),
    };
    Ok(model)
}

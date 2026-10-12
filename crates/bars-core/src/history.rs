//! Local dictation history (JSON Lines) and usage stats.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub at: DateTime<Local>,
    pub text: String,
    /// The model's output before clean-up, when it differs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    pub model: String,
    pub audio_secs: f32,
    pub infer_ms: u64,
    pub words: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub dictations: usize,
    pub words: usize,
    pub audio_secs: f64,
    /// Speaking pace across all dictations.
    pub wpm: f64,
    /// Estimated minutes saved versus typing at 40 WPM.
    pub minutes_saved: f64,
    pub streak_days: u32,
    pub avg_latency_ms: f64,
}

const TYPING_WPM: f64 = 40.0;

pub struct History {
    path: PathBuf,
    entries: Vec<Entry>,
}

impl History {
    pub fn open(path: PathBuf) -> History {
        let mut entries = Vec::new();
        if let Ok(file) = std::fs::File::open(&path) {
            for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
                if let Ok(e) = serde_json::from_str::<Entry>(&line) {
                    entries.push(e);
                }
            }
        }
        History { path, entries }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn next_id(&self) -> u64 {
        self.entries.last().map_or(1, |e| e.id + 1)
    }

    /// Adds and saves an entry. (With history off, the controller doesn't
    /// call this at all: every entry in the list ends up on disk.)
    pub fn append(&mut self, entry: Entry) {
        if let Err(e) = self.write_line(&entry) {
            log::warn!("could not save history: {e}");
        }
        self.entries.push(entry);
    }

    fn write_line(&self, entry: &Entry) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(entry)?)
    }

    pub fn delete(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
        self.rewrite();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        let _ = std::fs::remove_file(&self.path);
    }

    fn rewrite(&self) {
        let tmp = self.path.with_extension("jsonl.tmp");
        let result = (|| -> std::io::Result<()> {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            for e in &self.entries {
                writeln!(f, "{}", serde_json::to_string(e)?)?;
            }
            f.flush()?;
            std::fs::rename(&tmp, &self.path)
        })();
        if let Err(e) = result {
            log::warn!("could not rewrite history: {e}");
        }
    }

    pub fn stats(&self) -> Stats {
        let dictations = self.entries.len();
        let words: usize = self.entries.iter().map(|e| e.words).sum();
        let audio_secs: f64 = self.entries.iter().map(|e| e.audio_secs as f64).sum();
        let wpm = if audio_secs > 1.0 {
            words as f64 / (audio_secs / 60.0)
        } else {
            0.0
        };
        let minutes_saved = (words as f64 / TYPING_WPM - audio_secs / 60.0).max(0.0);
        let avg_latency_ms = if dictations > 0 {
            self.entries.iter().map(|e| e.infer_ms as f64).sum::<f64>() / dictations as f64
        } else {
            0.0
        };
        Stats {
            dictations,
            words,
            audio_secs,
            wpm,
            minutes_saved,
            streak_days: streak(self.entries.iter().map(|e| e.at.date_naive()), Local::now().date_naive()),
            avg_latency_ms,
        }
    }
}

/// Consecutive days with at least one dictation, ending today (or yesterday).
fn streak(days: impl Iterator<Item = NaiveDate>, today: NaiveDate) -> u32 {
    let set: std::collections::BTreeSet<NaiveDate> = days.collect();
    let mut day = if set.contains(&today) {
        today
    } else {
        match today.pred_opt() {
            Some(y) if set.contains(&y) => y,
            _ => return 0,
        }
    };
    let mut n = 0;
    while set.contains(&day) {
        n += 1;
        match day.pred_opt() {
            Some(d) => day = d,
            None => break,
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streak_counts_back_from_today() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 11).unwrap();
        let d = |n: u32| NaiveDate::from_ymd_opt(2026, 10, n).unwrap();
        assert_eq!(streak([d(11), d(10), d(9), d(7)].into_iter(), today), 3);
        assert_eq!(streak([d(10), d(9)].into_iter(), today), 2);
        assert_eq!(streak([d(8)].into_iter(), today), 0);
    }
}

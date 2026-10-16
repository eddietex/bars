//! Splits a long dictation into pieces at pauses while it's being
//! recorded, so each piece can be transcribed (and polished) while the user
//! goes on talking, and only the last one is left when they stop.
//!
//! A piece is at least `MIN_SECS` long, so short dictations are never
//! split, and is cut in the middle of the first pause after that. Without
//! a pause by `MAX_SECS` (one long breath), it's cut at the quietest moment
//! of the last few seconds instead.
//!
//! A piece comes with some of the audio either side of it: on its own, the
//! model takes a piece's start for the start of a sentence and its end for
//! the end of one ("customers. And watch" for "customers, and watch").
//! Only the words heard within the piece are kept.

use std::ops::Range;

/// Audio is judged in frames this long.
const FRAME_MS: usize = 20;
const FRAMES_PER_SEC: usize = 1000 / FRAME_MS;
/// No cut before a piece is this long: shorter dictations stay whole.
pub const MIN_SECS: usize = 12;
/// A piece is cut by this length even without a pause.
pub const MAX_SECS: usize = 28;
/// A quiet stretch this long is a pause to cut in.
const PAUSE_MS: usize = 400;
/// Without a pause, the cut goes at the quietest stretch this long...
const QUIETEST_MS: usize = 200;
/// ...within this much before `MAX_SECS`.
const SEARCH_SECS: usize = 8;
/// Audio heard before a piece, and after it (a piece waits for this).
const BEFORE_MS: usize = 3000;
const AFTER_MS: usize = 2000;

/// A piece of the recording to transcribe.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// The piece, with the audio around it.
    pub samples: Vec<f32>,
    /// Where the piece itself is in `samples`.
    pub keep: Range<usize>,
}

/// Follows a recording as it grows and hands out pieces of it.
pub struct Segmenter {
    /// Samples per frame at the recording's rate.
    frame: usize,
    /// Samples heard before and after a piece.
    before: usize,
    after: usize,
    /// The recording from sample `base` on.
    audio: Vec<f32>,
    base: usize,
    /// Where the piece being followed starts.
    cut: usize,
    /// Loudness (RMS) of each whole frame from `cut` on.
    levels: Vec<f32>,
    /// Where that piece ends, once found; it's handed out once `after` more
    /// has been recorded.
    end: Option<usize>,
}

impl Segmenter {
    /// For a recording at `rate` samples per second.
    pub fn new(rate: u32) -> Segmenter {
        let ms = |ms: usize| rate as usize * ms / 1000;
        Segmenter {
            frame: ms(FRAME_MS).max(1),
            before: ms(BEFORE_MS),
            after: ms(AFTER_MS),
            audio: Vec::new(),
            base: 0,
            cut: 0,
            levels: Vec::new(),
            end: None,
        }
    }

    /// Where in the recording the piece not yet handed out starts.
    pub fn cut(&self) -> usize {
        self.cut
    }

    /// Adds newly recorded samples. Returns a piece once one is ready (call
    /// again with nothing to see whether another is).
    pub fn push(&mut self, samples: &[f32]) -> Option<Piece> {
        self.audio.extend_from_slice(samples);
        let recorded = self.base + self.audio.len();
        while self.cut + (self.levels.len() + 1) * self.frame <= recorded {
            let start = self.cut + self.levels.len() * self.frame - self.base;
            let frame = &self.audio[start..start + self.frame];
            self.levels.push((frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt());
        }
        if self.end.is_none() {
            self.end = self.cut_frame().map(|at| self.cut + at * self.frame);
        }
        let end = self.end.filter(|end| end + self.after <= recorded)?;
        let from = self.cut.saturating_sub(self.before).max(self.base);
        let piece = Piece {
            samples: self.audio[from - self.base..end + self.after - self.base].to_vec(),
            keep: self.cut - from..end - from,
        };
        self.levels.drain(..(end - self.cut) / self.frame);
        self.cut = end;
        self.end = None;
        // Only what comes before the next piece is still needed.
        let base = end.saturating_sub(self.before).max(self.base);
        self.audio.drain(..base - self.base);
        self.base = base;
        Some(piece)
    }

    /// The last piece, from the whole `recording` once it has stopped.
    pub fn last(&self, recording: &[f32]) -> Piece {
        let cut = self.cut.min(recording.len());
        let from = cut.saturating_sub(self.before);
        Piece { samples: recording[from..].to_vec(), keep: cut - from..recording.len() - from }
    }

    /// The frame to cut the current piece at, if it's time.
    fn cut_frame(&self) -> Option<usize> {
        let (min, max) = (MIN_SECS * FRAMES_PER_SEC, MAX_SECS * FRAMES_PER_SEC);
        if self.levels.len() < min {
            return None;
        }
        // Quiet: well under how loud speech is in this piece (its 90th
        // percentile), and at least a floor, so a silent room counts.
        let mut sorted = self.levels.clone();
        sorted.sort_by(f32::total_cmp);
        let quiet = (sorted[sorted.len() * 9 / 10] * 0.2).max(0.001);
        // The first pause from `min` on (one that began just before counts),
        // cut in its middle.
        let pause = PAUSE_MS / FRAME_MS;
        let mut run = 0;
        for (i, level) in self.levels.iter().enumerate().skip(min - pause) {
            run = if *level < quiet { run + 1 } else { 0 };
            if run == pause {
                return Some(i + 1 - pause / 2);
            }
        }
        if self.levels.len() < max {
            return None;
        }
        // One long breath: the quietest moment shortly before `max`.
        let window = QUIETEST_MS / FRAME_MS;
        let from = max - SEARCH_SECS * FRAMES_PER_SEC;
        let loudness = |i: usize| self.levels[i..i + window].iter().sum::<f32>();
        let quietest = (from..max - window).min_by(|a, b| loudness(*a).total_cmp(&loudness(*b)))?;
        Some(quietest + window / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    /// "Speech": a loud tone with a wobble; "pause": near silence.
    fn speech(secs: f32) -> Vec<f32> {
        (0..(secs * RATE as f32) as usize).map(|i| ((i as f32) * 0.3).sin() * (0.1 + 0.05 * ((i as f32) * 0.001).sin())).collect()
    }

    fn pause(secs: f32) -> Vec<f32> {
        (0..(secs * RATE as f32) as usize).map(|i| ((i as f32) * 1.7).sin() * 0.0005).collect()
    }

    /// Feeds `audio` in 300 ms steps, as the controller does; returns the
    /// pieces and the segmenter, for the last one.
    fn pieces(audio: &[f32]) -> (Vec<Piece>, Segmenter) {
        let mut s = Segmenter::new(RATE);
        let mut out = Vec::new();
        for chunk in audio.chunks(RATE as usize * 3 / 10) {
            let mut next = s.push(chunk);
            while let Some(p) = next {
                out.push(p);
                next = s.push(&[]);
            }
        }
        (out, s)
    }

    /// How long each piece is, without the audio around it.
    fn secs(pieces: &[Piece]) -> Vec<f32> {
        pieces.iter().map(|p| p.keep.len() as f32 / RATE as f32).collect()
    }

    #[test]
    fn short_dictations_stay_whole() {
        let audio = [speech(4.0), pause(1.0), speech(6.0)].concat();
        let (cut, s) = pieces(&audio);
        assert!(cut.is_empty());
        assert_eq!(s.cut(), 0);
        assert_eq!(s.last(&audio), Piece { samples: audio.clone(), keep: 0..audio.len() });
    }

    #[test]
    fn cuts_in_the_first_pause_after_the_minimum() {
        // A pause at 5 s is too early; the one at 14 s is taken.
        let audio = [speech(5.0), pause(1.0), speech(8.0), pause(1.0), speech(10.0)].concat();
        let (cut, s) = pieces(&audio);
        let len = secs(&cut);
        assert_eq!(len.len(), 1, "{len:?}");
        // In the middle of the pause's first 400 ms: 14.2 s.
        assert!((len[0] - 14.2).abs() < 0.05, "{len:?}");
        assert_eq!(s.cut(), cut[0].keep.end);
    }

    #[test]
    fn a_long_breath_is_cut_at_its_quietest() {
        // Speech with only a short dip (too short to be a pause) at 25 s.
        let audio = [speech(25.0), pause(0.25), speech(15.0)].concat();
        let len = secs(&pieces(&audio).0);
        assert_eq!(len.len(), 1, "{len:?}");
        assert!(len[0] > 25.0 && len[0] < 25.3, "{len:?}");
        // Continuous speech: cut by the maximum anyway.
        let len = secs(&pieces(&speech(60.0)).0);
        assert!(len.len() == 2 && len.iter().all(|c| *c <= MAX_SECS as f32), "{len:?}");
    }

    #[test]
    fn pieces_add_up_to_the_recording() {
        let audio = [speech(13.0), pause(0.8), speech(20.0), pause(0.6), speech(14.0), pause(2.5)].concat();
        let (cut, s) = pieces(&audio);
        let len = secs(&cut);
        // The closing pause is a cut too: only silence is left at the end.
        assert_eq!(len.len(), 3, "{len:?}");
        assert!((len[2] - 14.6).abs() < 0.05, "{len:?}");
        let last = s.last(&audio);
        let kept: Vec<f32> = cut.iter().chain([&last]).flat_map(|p| p.samples[p.keep.clone()].to_vec()).collect();
        assert!(kept == audio, "the pieces are the recording, each sample once");
    }

    #[test]
    fn pieces_come_with_the_audio_around_them() {
        let audio = [speech(13.0), pause(0.8), speech(20.0), pause(0.6), speech(10.0)].concat();
        let (cut, s) = pieces(&audio);
        let (before, after) = (RATE as usize * 3, RATE as usize * 2);
        // The first starts the recording; each has 2 s after it...
        assert_eq!(cut[0].keep.start, 0);
        assert!(cut.iter().all(|p| p.samples.len() == p.keep.end + after));
        // ...and from the second on, 3 s before it, as recorded.
        let start = cut[0].keep.end;
        assert_eq!(cut[1].keep.start, before);
        assert!(cut[1].samples[..] == audio[start - before..start + cut[1].keep.len() + after]);
        // The last runs to the end of the recording.
        let last = s.last(&audio);
        assert_eq!((last.keep.start, last.keep.end), (before, last.samples.len()));
        assert!(last.samples[..] == audio[s.cut() - before..]);
    }

    #[test]
    fn a_piece_waits_for_the_audio_after_it() {
        // The cut at 13.2 s is found at 13.4 s, but handed out at 15.2 s.
        let audio = [speech(13.0), pause(0.8), speech(10.0)].concat();
        let mut s = Segmenter::new(RATE);
        let at = |secs: f32| (secs * RATE as f32) as usize;
        assert_eq!(s.push(&audio[..at(15.1)]), None);
        let piece = s.push(&audio[at(15.1)..at(15.3)]).expect("a piece");
        assert_eq!(piece.keep, 0..at(13.2));
        // Stopping before then leaves it all for the last piece.
        let mut s = Segmenter::new(RATE);
        assert_eq!(s.push(&audio[..at(14.0)]), None);
        assert_eq!(s.last(&audio[..at(14.0)]).keep, 0..at(14.0));
    }
}

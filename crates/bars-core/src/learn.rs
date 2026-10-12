//! Learning from corrections. When the user fixes a word Bars wrote, and
//! the fix looks like a mishearing put right rather than a rewording, it
//! becomes a dictionary entry. Everything is local: word-level diffing,
//! spelling and sound similarity, and how unsure the speech model was of
//! the word it wrote (Parakeet only).
//!
//! Field text is only ever held in memory; just the learned word pairs are
//! kept.

use std::collections::HashSet;

use crate::config::Replacement;

/// The model's confidence below which a word counts as unsure: on test
/// audio, words it got wrong scored 0.14–0.51, words it got right 0.74+.
pub const UNSURE: f32 = 0.6;

/// A suggestion seen this many times becomes a dictionary entry.
pub const PROMOTE_AFTER: u32 = 2;

/// One word as the model wrote it, and how sure it was (the lowest
/// probability among its letter-bearing pieces; punctuation doesn't count).
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub word: String,
    pub confidence: f32,
}

/// Groups a decode's tokens (texts from the vocabulary, word starts
/// beginning with a space) into words with confidences.
pub fn heard_words(tokens: &[&str], probs: &[f32]) -> Vec<Heard> {
    let mut out: Vec<Heard> = Vec::new();
    for (tok, &p) in tokens.iter().zip(probs) {
        let starts_word = tok.starts_with(' ') || out.is_empty();
        let letters = tok.chars().any(char::is_alphanumeric);
        if starts_word {
            out.push(Heard {
                word: tok.trim().to_string(),
                confidence: if letters { p } else { 1.0 },
            });
        } else if let Some(w) = out.last_mut() {
            w.word.push_str(tok);
            if letters {
                w.confidence = w.confidence.min(p);
            }
        }
    }
    out
}

/// The model's vocabulary pieces, to tell common words (one piece) from
/// rare ones and names (several pieces).
#[derive(Debug, Default)]
pub struct Pieces {
    set: HashSet<String>,
    max_chars: usize,
}

impl Pieces {
    pub fn from_vocab(vocab: &[String]) -> Pieces {
        // Special tokens (<unk>, <|endoftext|>, ...) are never pieces of text.
        let set: HashSet<String> = vocab
            .iter()
            .filter(|t| !t.is_empty() && !(t.starts_with('<') && t.ends_with('>')))
            .cloned()
            .collect();
        let max_chars = set.iter().map(|t| t.chars().count()).max().unwrap_or(1);
        Pieces { set, max_chars }
    }

    /// How many pieces the model needs for `text`, longest pieces first.
    pub fn count(&self, text: &str) -> usize {
        let mut n = 0;
        for word in text.split_whitespace() {
            let chars: Vec<char> = std::iter::once(' ').chain(word.chars()).collect();
            let mut i = 0;
            while i < chars.len() {
                let len = (1..=self.max_chars.min(chars.len() - i))
                    .rev()
                    .find(|&len| self.set.contains(&chars[i..i + len].iter().collect::<String>()))
                    .unwrap_or(1);
                n += 1;
                i += len;
            }
        }
        n
    }

    /// One piece, lowercase: an everyday word ("there", "good").
    fn is_common(&self, word: &str) -> bool {
        !self.set.is_empty() && word.chars().all(char::is_lowercase) && self.count(word) == 1
    }
}

/// A changed stretch: `from` (as inserted) became `to` (as edited).
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub from: String,
    pub to: String,
    /// The change isn't at the start of a sentence.
    pub mid_sentence: bool,
    /// Where `from` sits in the dictation, as a fraction of its words (to
    /// tell repeated words apart).
    pub at: f32,
}

/// A word with its surrounding punctuation set aside.
struct Word<'a> {
    core: &'a str,
    raw: &'a str,
}

fn split_words(text: &str) -> Vec<Word<'_>> {
    text.split_whitespace()
        .map(|raw| Word {
            core: raw.trim_matches(|c: char| !c.is_alphanumeric()),
            raw,
        })
        .filter(|w| !w.core.is_empty())
        .collect()
}

/// The small changes between what was inserted and what the field holds
/// now: one to three words replaced by one to three, next to an unchanged
/// word. None when more than half the words changed (a rewrite, not a fix).
pub fn find_edits(inserted: &str, edited: &str) -> Option<Vec<Edit>> {
    let a = split_words(inserted);
    let b = split_words(edited);
    if a.is_empty() || b.is_empty() {
        return None;
    }
    // Longest common subsequence of words.
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i].core == b[j].core {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut spans = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i].core == b[j].core {
            i += 1;
            j += 1;
            continue;
        }
        let (si, sj) = (i, j);
        while (i < n || j < m) && !(i < n && j < m && a[i].core == b[j].core) {
            if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
                j += 1;
            } else {
                i += 1;
            }
        }
        spans.push((si..i, sj..j));
    }
    let changed: usize = spans.iter().map(|(x, _)| x.len()).sum();
    if changed * 2 > n {
        return None;
    }
    let edits = spans
        .into_iter()
        .filter(|(x, y)| (1..=3).contains(&x.len()) && (1..=3).contains(&y.len()))
        // An unchanged word next to it anchors the fix.
        .filter(|(x, _)| x.start > 0 || x.end < n)
        .map(|(x, y)| Edit {
            from: a[x.clone()].iter().map(|w| w.core).collect::<Vec<_>>().join(" "),
            to: b[y].iter().map(|w| w.core).collect::<Vec<_>>().join(" "),
            mid_sentence: x.start > 0 && !a[x.start - 1].raw.ends_with(['.', '!', '?']),
            at: x.start as f32 / n as f32,
        })
        .collect();
    Some(edits)
}

/// The model's confidence in `edit.from`, found among the words it heard:
/// of repeated words, the one at the same place (clean-up may have changed
/// the text; then it's unknown).
pub fn confidence_of(edit: &Edit, heard: &[Heard]) -> Option<f32> {
    let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    let want: Vec<String> = edit.from.split_whitespace().map(norm).collect();
    let words: Vec<String> = heard.iter().map(|h| norm(&h.word)).collect();
    let expected = edit.at * words.len() as f32;
    words
        .windows(want.len())
        .enumerate()
        .filter(|(_, w)| *w == want.as_slice())
        .map(|(i, _)| i)
        .min_by(|a, b| (*a as f32 - expected).abs().total_cmp(&(*b as f32 - expected).abs()))
        .map(|i| heard[i..i + want.len()].iter().map(|h| h.confidence).fold(1.0, f32::min))
}

/// Where a dictation went in its field: the text right before and after
/// it, to find it again while the user edits.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    before: String,
    after: String,
}

/// Characters of surrounding text that pin a dictation down.
const ANCHOR_CHARS: usize = 40;

impl Placement {
    /// From the field as it was when the dictation went in.
    pub fn new(field: &crate::text::Surroundings) -> Placement {
        let n = field.before.chars().count();
        Placement {
            before: field.before.chars().skip(n.saturating_sub(ANCHOR_CHARS)).collect(),
            after: field.after.chars().take(ANCHOR_CHARS).collect(),
        }
    }

    /// The dictation as it reads now in `text` (the field around the
    /// cursor), or None once its surroundings are gone.
    pub fn locate(&self, text: &str, inserted_len: usize) -> Option<String> {
        let starts: Vec<usize> = if self.before.is_empty() {
            vec![0]
        } else {
            text.match_indices(self.before.as_str()).map(|(i, m)| i + m.len()).collect()
        };
        let best = starts
            .into_iter()
            .filter_map(|start| {
                let end = if self.after.is_empty() {
                    text.len()
                } else {
                    start + text[start..].find(self.after.as_str())?
                };
                Some((start, end))
            })
            // Several matches: the one closest in length to what went in.
            .min_by_key(|(s, e)| (e - s).abs_diff(inserted_len))?;
        let found = &text[best.0..best.1];
        // Far longer than the dictation: the anchors matched elsewhere.
        (found.len() <= inserted_len * 3 + 80).then(|| found.trim().to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// A clear fix: add it to the dictionary now.
    Learn,
    /// Plausible: suggest it, and learn it once seen again.
    Suggest,
    Ignore(&'static str),
}

/// Decides what to do with an edit. `confidence` is the model's (None when
/// unknown); `known` holds pairs already in the dictionary or turned down.
pub fn judge(edit: &Edit, confidence: Option<f32>, pieces: &Pieces, known: &[Replacement]) -> Verdict {
    let (from, to) = (edit.from.as_str(), edit.to.as_str());
    let (from_lc, to_lc) = (from.to_lowercase(), to.to_lowercase());
    if from == to {
        return Verdict::Ignore("unchanged");
    }
    if known.iter().any(|r| r.from.eq_ignore_ascii_case(from) && r.to == to) {
        return Verdict::Ignore("already known");
    }
    if from.chars().chain(to.chars()).any(|c| c.is_ascii_digit()) {
        return Verdict::Ignore("numbers");
    }
    if from_lc == to_lc {
        // Only capitalization: a name, if it's mid-sentence.
        return if edit.mid_sentence && to.starts_with(char::is_uppercase) {
            Verdict::Suggest
        } else {
            Verdict::Ignore("capitalization")
        };
    }
    if grammar_only(&from_lc, &to_lc) {
        return Verdict::Ignore("grammar");
    }
    // A rule for an everyday word would fire everywhere ("there" → "their").
    if !from.contains(' ') && pieces.is_common(from) {
        return Verdict::Ignore("common word");
    }
    let similar = compact(&from_lc) == compact(&to_lc)
        || similarity(&from_lc, &to_lc) >= 0.6
        || sounds_alike(&from_lc, &to_lc);
    if !similar {
        return Verdict::Ignore("reworded");
    }
    // Names, jargon and words the model needs several pieces for.
    let rare = to.chars().any(char::is_uppercase)
        || pieces.count(to) >= 3 * to.split_whitespace().count().max(1);
    let unsure = confidence.is_some_and(|c| c < UNSURE);
    match (unsure, rare) {
        (true, true) => Verdict::Learn,
        (true, false) | (false, true) => Verdict::Suggest,
        (false, false) => Verdict::Ignore("the model was sure"),
    }
}

/// Contractions, possessives and simple inflections: grammar, not hearing.
fn grammar_only(from: &str, to: &str) -> bool {
    let strip = |s: &str| s.replace(['\'', '’'], "");
    if strip(from) == strip(to) {
        return true;
    }
    let (short, long) = if from.len() <= to.len() { (from, to) } else { (to, from) };
    long.strip_prefix(short)
        .is_some_and(|rest| ["s", "es", "ed", "d", "ing", "'s", "’s", "ly"].contains(&rest))
}

/// Letters and digits only: "chat gpt" and "ChatGPT" compare equal.
fn compact(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// 1 − edit distance / length of the longer string.
fn similarity(a: &str, b: &str) -> f32 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f32 / longest as f32
}

/// A rough English sound key: consonants by sound, voiced and unvoiced
/// pairs merged (what speech models mix up), vowels dropped after the
/// first letter. "cooper netties" and "Kubernetes" both give "kprnts".
fn sound_key(s: &str) -> String {
    let w: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()).collect();
    let mut out = String::new();
    let mut i = 0;
    while i < w.len() {
        let next = w.get(i + 1).copied();
        let soft = matches!(next, Some('e' | 'i' | 'y'));
        let (code, skip): (Option<char>, usize) = match w[i] {
            'a' | 'e' | 'i' | 'o' | 'u' | 'y' => (if i == 0 { Some('a') } else { None }, 1),
            'p' if next == Some('h') => (Some('f'), 2),
            'c' if next == Some('k') => (Some('k'), 2),
            'c' if next == Some('h') => (Some('x'), 2),
            's' if next == Some('h') => (Some('x'), 2),
            't' if next == Some('h') => (Some('0'), 2),
            'w' if next == Some('h') => (Some('w'), 2),
            'g' if next == Some('h') => (None, 2),
            'k' | 'g' | 'w' if i == 0 && next == Some('n') => (None, 1),
            'w' if i == 0 && next == Some('r') => (None, 1),
            'c' => (Some(if soft { 's' } else { 'k' }), 1),
            'g' => (Some(if soft { 'j' } else { 'k' }), 1),
            'b' => (Some('p'), 1),
            'd' => (Some('t'), 1),
            'v' => (Some('f'), 1),
            'z' => (Some('s'), 1),
            'q' => (Some('k'), 1),
            'x' => (Some('s'), 1),
            'h' => (None, 1),
            c => (Some(c), 1),
        };
        if let Some(c) = code {
            if !out.ends_with(c) {
                out.push(c);
            }
        }
        i += skip;
    }
    out
}

fn sounds_alike(a: &str, b: &str) -> bool {
    let (ka, kb) = (sound_key(a), sound_key(b));
    ka.len().max(kb.len()) >= 3 && (ka == kb || similarity(&ka, &kb) >= 0.75)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces() -> Pieces {
        let vocab: Vec<String> = [" there", " their", " good", " great", " the", " cooper", " net", "ties", " K", "uber", "net", "es", " chat", " g", "pt", " is", " a", " tool", " county", " country"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        Pieces::from_vocab(&vocab)
    }

    fn edit(from: &str, to: &str) -> Edit {
        Edit { from: from.into(), to: to.into(), mid_sentence: true, at: 0.5 }
    }

    #[test]
    fn groups_pieces_into_words_with_confidence() {
        let heard = heard_words(&[" K", "uber", "net", "es", ",", " is"], &[0.9, 0.3, 0.8, 0.95, 0.1, 0.99]);
        assert_eq!(heard.len(), 2);
        assert_eq!(heard[0].word, "Kubernetes,");
        // The comma's low score doesn't count.
        assert_eq!(heard[0].confidence, 0.3);
    }

    #[test]
    fn finds_small_fixes_and_skips_rewrites() {
        let e = find_edits("We use cooper netties for this.", "We use Kubernetes for this.").unwrap();
        assert_eq!(e, vec![Edit { from: "cooper netties".into(), to: "Kubernetes".into(), mid_sentence: true, at: 2.0 / 6.0 }]);
        // Punctuation around a word isn't a change.
        assert_eq!(find_edits("Hello, world.", "Hello world!").unwrap(), vec![]);
        // More than half changed: a rewrite.
        assert!(find_edits("This is fine", "That was not okay").is_none());
        // A one-word dictation changed entirely is a rewrite too.
        assert!(find_edits("Chris", "Kris").is_none());
    }

    #[test]
    fn learns_unsure_fixes_of_rare_words() {
        let p = pieces();
        assert_eq!(judge(&edit("cooper netties", "Kubernetes"), Some(0.3), &p, &[]), Verdict::Learn);
        assert_eq!(judge(&edit("cooper netties", "Kubernetes"), Some(0.9), &p, &[]), Verdict::Suggest);
        assert_eq!(judge(&edit("chat gpt", "ChatGPT"), None, &p, &[]), Verdict::Suggest);
        assert_eq!(judge(&edit("chris", "Chris"), Some(0.5), &p, &[]), Verdict::Suggest);
    }

    #[test]
    fn ignores_rewordings_grammar_and_risky_rules() {
        let p = pieces();
        assert_eq!(judge(&edit("good", "great"), Some(0.3), &p, &[]), Verdict::Ignore("common word"));
        assert_eq!(judge(&edit("there", "their"), Some(0.3), &p, &[]), Verdict::Ignore("common word"));
        assert_eq!(judge(&edit("tools", "hammers"), Some(0.3), &p, &[]), Verdict::Ignore("reworded"));
        assert_eq!(judge(&edit("dont", "don't"), Some(0.3), &p, &[]), Verdict::Ignore("grammar"));
        assert_eq!(judge(&edit("tool", "tools"), Some(0.3), &p, &[]), Verdict::Ignore("grammar"));
        assert_eq!(judge(&edit("version 2", "version 3"), Some(0.3), &p, &[]), Verdict::Ignore("numbers"));
        let known = [Replacement { from: "cooper netties".into(), to: "Kubernetes".into(), learned: true }];
        assert_eq!(judge(&edit("cooper netties", "Kubernetes"), Some(0.3), &p, &known), Verdict::Ignore("already known"));
        let start = Edit { mid_sentence: false, ..edit("chris", "Chris") };
        assert_eq!(judge(&start, Some(0.3), &p, &[]), Verdict::Ignore("capitalization"));
    }

    #[test]
    fn finds_the_models_confidence_in_the_changed_words() {
        let heard = vec![
            Heard { word: "We".into(), confidence: 0.99 },
            Heard { word: "use".into(), confidence: 0.97 },
            Heard { word: "cooper".into(), confidence: 0.4 },
            Heard { word: "netties,".into(), confidence: 0.3 },
        ];
        assert_eq!(confidence_of(&edit("cooper netties", "Kubernetes"), &heard), Some(0.3));
        assert_eq!(confidence_of(&edit("kettle", "Kotlin"), &heard), None);
        // A repeated word: the occurrence at the edit's place.
        let heard = vec![
            Heard { word: "and".into(), confidence: 0.98 },
            Heard { word: "so".into(), confidence: 0.9 },
            Heard { word: "and".into(), confidence: 0.33 },
            Heard { word: "not".into(), confidence: 0.99 },
        ];
        let second = Edit { at: 0.5, ..edit("and", "ask") };
        assert_eq!(confidence_of(&second, &heard), Some(0.33));
    }

    #[test]
    fn finds_the_dictation_again_in_its_field() {
        let field = crate::text::Surroundings {
            before: "Notes: we deploy with ".into(),
            selected: String::new(),
            after: " every day.".into(),
        };
        let place = Placement::new(&field);
        let now = "Notes: we deploy with Kubernetes now every day.";
        assert_eq!(place.locate(now, "cooper netties".len()).as_deref(), Some("Kubernetes now"));
        // Its surroundings were deleted: it's gone.
        assert_eq!(place.locate("Something else entirely.", 14), None);
        // At the end of an empty field: everything after the start.
        let empty = Placement::new(&crate::text::Surroundings::default());
        assert_eq!(empty.locate("Hello there", 11).as_deref(), Some("Hello there"));
    }

    #[test]
    fn sound_keys_match_mishearings() {
        assert_eq!(sound_key("cooper netties"), sound_key("Kubernetes"));
        assert!(sounds_alike("jason", "json"));
        assert!(!sounds_alike("good", "great"));
        assert_eq!(pieces().count("Kubernetes"), 4);
        assert_eq!(pieces().count("there"), 1);
    }
}

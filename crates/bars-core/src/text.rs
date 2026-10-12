//! Local, deterministic clean-up of raw transcripts: filler removal, spoken
//! line-break commands, personal dictionary, snippets, and spacing.

use once_cell::sync::Lazy;
use regex::{Regex, RegexBuilder};

use crate::config::{Replacement, Snippet, TextConfig};

static FILLERS: Lazy<Regex> = Lazy::new(|| {
    // Leading comma/space handling keeps "So, um, I think" -> "So, I think".
    Regex::new(r"(?i)(?:,\s*)?\b(?:u+h+m*|u+m+|e+r+m+|e+h+|a+h+|h+m+|m+h*m+)\b[,.]?").unwrap()
});
static MULTI_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t]{2,}").unwrap());
static SPACE_BEFORE_PUNCT: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+([,.;:!?])").unwrap());
static DOUBLE_COMMA: Lazy<Regex> = Lazy::new(|| Regex::new(r",\s*([,.;:!?])").unwrap());
static NEW_PARAGRAPH: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\s*\bnew paragraph\b[,.]?\s*").unwrap());
static NEW_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\s*\bnew line\b[,.]?\s*").unwrap());

pub struct TextProcessor {
    remove_fillers: bool,
    voice_commands: bool,
    replacements: Vec<(Regex, String)>,
    snippets: Vec<(Regex, String)>,
}

fn phrase_regex(phrase: &str) -> Option<Regex> {
    let words: Vec<String> = phrase
        .split_whitespace()
        .map(|w| regex::escape(w.trim_matches(|c: char| c.is_ascii_punctuation())))
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return None;
    }
    // Words may be separated by whitespace and/or punctuation the model added
    // ("chat, gpt"). Boundaries are Unicode-aware.
    let body = words.join(r"[\s,.-]+");
    RegexBuilder::new(&format!(r"\b{body}\b"))
        .case_insensitive(true)
        .build()
        .ok()
}

impl TextProcessor {
    pub fn new(cfg: &TextConfig) -> Self {
        let compile = |items: Vec<(&str, &str)>| -> Vec<(Regex, String)> {
            let mut out: Vec<(Regex, String, usize)> = items
                .into_iter()
                .filter_map(|(from, to)| phrase_regex(from).map(|r| (r, to.to_string(), from.len())))
                .collect();
            // Longest phrases first so "new york times" wins over "new york".
            out.sort_by(|a, b| b.2.cmp(&a.2));
            out.into_iter().map(|(r, t, _)| (r, t)).collect()
        };
        TextProcessor {
            remove_fillers: cfg.remove_fillers,
            voice_commands: cfg.voice_commands,
            replacements: compile(
                cfg.dictionary
                    .iter()
                    .map(|Replacement { from, to }| (from.as_str(), to.as_str()))
                    .collect(),
            ),
            snippets: compile(
                cfg.snippets
                    .iter()
                    .map(|Snippet { trigger, expansion }| (trigger.as_str(), expansion.as_str()))
                    .collect(),
            ),
        }
    }

    pub fn process(&self, raw: &str) -> String {
        let mut text = raw.trim().to_string();
        if text.is_empty() {
            return text;
        }

        if self.remove_fillers {
            text = FILLERS.replace_all(&text, "").into_owned();
        }

        for (re, to) in &self.replacements {
            text = re.replace_all(&text, regex::NoExpand(to)).into_owned();
        }

        // Snippets: if the whole utterance is just the cue, emit the expansion
        // verbatim (no added punctuation); otherwise expand inline.
        for (re, expansion) in &self.snippets {
            let bare = text.trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
            if re.find(bare).is_some_and(|m| m.start() == 0 && m.end() == bare.len()) {
                return expansion.clone();
            }
            text = re.replace_all(&text, regex::NoExpand(expansion)).into_owned();
        }

        if self.voice_commands {
            text = NEW_PARAGRAPH.replace_all(&text, "\n\n").into_owned();
            text = NEW_LINE.replace_all(&text, "\n").into_owned();
        }

        text = tidy(&text);
        if self.voice_commands {
            text = capitalize_after_breaks(&text);
        }
        text
    }
}

fn tidy(text: &str) -> String {
    let mut t = MULTI_SPACE.replace_all(text, " ").into_owned();
    t = SPACE_BEFORE_PUNCT.replace_all(&t, "$1").into_owned();
    t = DOUBLE_COMMA.replace_all(&t, "$1").into_owned();
    let t = t
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    let t = t.trim_matches(|c: char| c == ' ' || c == ',').to_string();
    capitalize_first(&t)
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_lowercase() => c.to_uppercase().collect::<String>() + chars.as_str(),
        _ => s.to_string(),
    }
}

fn capitalize_after_breaks(s: &str) -> String {
    s.split('\n').map(capitalize_first).collect::<Vec<_>>().join("\n")
}

/// Joins text onto whatever came before. Adds a trailing space when asked
/// (and the text doesn't end in a line break).
pub fn finalize(text: &str, trailing_space: bool) -> String {
    if trailing_space && !text.is_empty() && !text.ends_with('\n') {
        format!("{text} ")
    } else {
        text.to_string()
    }
}

pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(cfg: TextConfig) -> TextProcessor {
        TextProcessor::new(&cfg)
    }

    #[test]
    fn removes_fillers() {
        let p = proc(TextConfig::default());
        assert_eq!(p.process("Um, so I think, uh, we should go."), "So I think we should go.");
        assert_eq!(p.process("uhh hmm okay"), "Okay");
        // Doesn't eat real words.
        assert_eq!(p.process("The umbrella is here."), "The umbrella is here.");
    }

    #[test]
    fn line_commands() {
        let p = proc(TextConfig::default());
        assert_eq!(
            p.process("Dear team, new paragraph thanks for coming. New line Chris"),
            "Dear team,\n\nThanks for coming.\nChris"
        );
    }

    #[test]
    fn dictionary_and_snippets() {
        let mut cfg = TextConfig::default();
        cfg.dictionary.push(Replacement {
            from: "chat gpt".into(),
            to: "ChatGPT".into(),
        });
        cfg.snippets.push(Snippet {
            trigger: "my email".into(),
            expansion: "me@example.xyz".into(),
        });
        let p = proc(cfg);
        assert_eq!(p.process("I asked Chat, GPT about it."), "I asked ChatGPT about it.");
        assert_eq!(p.process("My email."), "me@example.xyz");
        assert_eq!(p.process("Send it to my email please."), "Send it to me@example.xyz please.");
    }

    #[test]
    fn finalize_spacing() {
        assert_eq!(finalize("Hello.", true), "Hello. ");
        assert_eq!(finalize("Hello.\n", true), "Hello.\n");
        assert_eq!(finalize("Hello.", false), "Hello.");
    }
}

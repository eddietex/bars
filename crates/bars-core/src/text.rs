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

/// The text around the cursor in the field a dictation goes into.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Surroundings {
    pub before: String,
    /// Text the dictation replaces.
    pub selected: String,
    pub after: String,
}

/// Skipped when looking back for the word the cursor continues from.
const SKIPPABLE: &str = "—`,-/:;()[]{} ";
/// Closing punctuation that a space may follow.
const CLOSING: &str = "!%),.:;>?]}";
/// Operators that a space follows only when one comes before them ("a - ").
const OPERATORS: &str = "\\/|-&*+=~";
/// Full-width punctuation, which never takes a space after it.
const CJK_PUNCT: &str = "。、，．！？；：…‥「」『』（）〔〕【】《》〈〉〝〞〟・";

fn letter_or_digit(c: char) -> bool {
    c.is_alphabetic() || c.is_numeric()
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}' | '\u{FF66}'..='\u{FF9F}' | '\u{20000}'..='\u{2FFFF}')
}

/// The cursor sits mid-sentence: the last word before it (skipping commas,
/// dashes, brackets and spaces) is a letter or digit, not a full stop.
fn continues_sentence(before_line: &str) -> bool {
    for c in before_line.trim_end_matches([' ', '\u{a0}']).chars().rev() {
        if letter_or_digit(c) {
            return true;
        }
        if !SKIPPABLE.contains(c) {
            return false;
        }
    }
    false
}

/// The sentence carries on after the cursor (lowercase text or a comma).
fn sentence_carries_on(after_line: &str) -> bool {
    for c in after_line.trim_start_matches([' ', '\u{a0}']).chars() {
        if c.is_alphabetic() && !c.is_uppercase() {
            return true;
        }
        if "!%,.:;>?".contains(c) {
            return true;
        }
        if !SKIPPABLE.contains(c) {
            return false;
        }
    }
    false
}

fn needs_leading_space(before_line: &str) -> bool {
    let mut rev = before_line.chars().rev();
    let Some(last) = rev.next() else { return false };
    if is_cjk(last) {
        return false;
    }
    letter_or_digit(last) || CLOSING.contains(last) || (OPERATORS.contains(last) && matches!(rev.next(), None | Some(' ')))
}

fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match (chars.next(), s.chars().nth(1)) {
        (Some(c), Some(_)) => c.to_lowercase().collect::<String>() + chars.as_str(),
        _ => s.to_string(),
    }
}

fn strip_final_stop(s: &str) -> String {
    s.trim_end_matches(['.', '!', '?']).to_string()
}

/// Whether the first word must keep its capital: "I", acronyms and names
/// with inner capitals, dictionary words, and words the field itself
/// capitalizes mid-sentence (names already written there).
fn starts_with_name(text: &str, names: &[String], field: &Surroundings) -> bool {
    let first = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
    if first.is_empty() || !first.starts_with(char::is_uppercase) {
        return false;
    }
    if first == "I" || first.starts_with("I'") || first.starts_with("I’") {
        return true;
    }
    if first.chars().skip(1).any(char::is_uppercase) {
        return true;
    }
    if names.iter().any(|n| n.split_whitespace().next() == Some(first)) {
        return true;
    }
    [field.before.as_str(), field.after.as_str()].iter().any(|ctx| {
        ctx.match_indices(first).any(|(i, _)| {
            let whole_word = !ctx[..i].ends_with(char::is_alphanumeric)
                && !ctx[i + first.len()..].starts_with(char::is_alphanumeric);
            // Capitalized because it's a name, not because a sentence starts.
            let prev = ctx[..i].trim_end().chars().next_back();
            whole_word && prev.is_some_and(|c| letter_or_digit(c) || c == ',')
        })
    })
}

/// Fits a dictation into the text around the cursor: a leading space only
/// after a word or closing punctuation, a lowercase start when it continues
/// a sentence, no final full stop when the sentence carries on, and a
/// trailing space only where the next character wants one. At the end of a
/// line, `trailing_space` decides.
pub fn fit_to_field(text: &str, field: &Surroundings, trailing_space: bool, names: &[String]) -> String {
    let text = text.trim_matches(|c: char| c == ' ' || c == '\u{a0}');
    if text.is_empty() {
        return String::new();
    }
    let keep_case = starts_with_name(text, names, field);
    let mut out = text.to_string();

    if !field.selected.is_empty() {
        let sel = field.selected.trim();
        if !keep_case && sel.chars().next().is_some_and(|c| c.is_alphabetic() && c.is_lowercase()) {
            out = lowercase_first(&out);
        }
        if !sel.chars().next_back().is_some_and(|c| CLOSING.contains(c)) {
            out = strip_final_stop(&out);
        }
        if field.selected.starts_with(' ') {
            out.insert(0, ' ');
        }
        if field.selected.ends_with(' ') {
            out.push(' ');
        }
        return out;
    }

    let before_line = field.before.rsplit('\n').next().unwrap_or("");
    let after_line = field.after.split('\n').next().unwrap_or("");
    if !keep_case && continues_sentence(before_line) {
        out = lowercase_first(&out);
    }
    if sentence_carries_on(after_line) {
        out = strip_final_stop(&out);
    }
    if needs_leading_space(before_line) {
        out.insert(0, ' ');
    }
    let mut next = after_line.chars();
    let space_after = match next.next() {
        None => trailing_space,
        Some(c) => letter_or_digit(c) || (OPERATORS.contains(c) && next.next() == Some(' ')),
    };
    if space_after && !out.ends_with('\n') && !out.ends_with(|c: char| CJK_PUNCT.contains(c)) {
        out.push(' ');
    }
    out
}

/// A coding agent running in the focused terminal.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Agent {
    ClaudeCode,
    Codex,
}

/// Splits a long paste so a terminal coding agent shows it as text rather
/// than collapsing it into "[Pasted text]": Claude Code gets pieces of at
/// most 600 characters and 2 line breaks, Codex pieces of 1000 characters.
/// The pieces join back into exactly `text`.
pub fn split_for_agent(text: &str, agent: Agent) -> Vec<String> {
    let chars = text.chars().count();
    match agent {
        Agent::ClaudeCode if chars > 600 || text.matches('\n').count() >= 3 => split_at_lines(text, 600, 2),
        Agent::Codex if chars > 1000 => {
            let v: Vec<char> = text.chars().collect();
            v.chunks(1000).map(|c| c.iter().collect()).collect()
        }
        _ => vec![text.to_string()],
    }
}

/// Pieces of at most `max_chars` characters and `max_breaks` line breaks,
/// cut at line breaks (a break that ends a piece starts the next one) and
/// inside a line only when it doesn't fit in a piece of its own.
fn split_at_lines(text: &str, max_chars: usize, max_breaks: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut cur = String::new();
    let (mut chars, mut breaks) = (0, 0);
    for (i, line) in text.split('\n').enumerate() {
        let brk = usize::from(i > 0);
        let len = line.chars().count();
        if !cur.is_empty() && (chars + brk + len > max_chars || breaks + brk > max_breaks) {
            pieces.push(std::mem::take(&mut cur));
            (chars, breaks) = (0, 0);
        }
        if brk == 1 {
            cur.push('\n');
            chars += 1;
            breaks += 1;
        }
        let mut rest = line;
        while chars + rest.chars().count() > max_chars {
            let room = max_chars.saturating_sub(chars);
            let cut = rest.char_indices().nth(room).map_or(rest.len(), |(b, _)| b);
            cur.push_str(&rest[..cut]);
            rest = &rest[cut..];
            pieces.push(std::mem::take(&mut cur));
            (chars, breaks) = (0, 0);
        }
        cur.push_str(rest);
        chars += rest.chars().count();
    }
    if !cur.is_empty() {
        pieces.push(cur);
    }
    pieces
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

    fn field(before: &str, after: &str) -> Surroundings {
        Surroundings { before: before.into(), selected: String::new(), after: after.into() }
    }

    #[test]
    fn fits_into_the_sentence_around_the_cursor() {
        let fit = |b: &str, a: &str, t: &str| fit_to_field(t, &field(b, a), true, &[]);
        // Start of an empty field: as dictated, plus the end-of-line space.
        assert_eq!(fit("", "", "Hello there."), "Hello there. ");
        // After a word: space before, lowercase start.
        assert_eq!(fit("I think", "", "We should go."), " we should go. ");
        // After a comma too; after a full stop the capital stays.
        assert_eq!(fit("Well,", "", "Maybe."), " maybe. ");
        assert_eq!(fit("Done.", "", "Next one."), " Next one. ");
        // Already a space before: none added.
        assert_eq!(fit("Done. ", "", "Next."), "Next. ");
        // Lowercase text follows: no full stop, and a space before it.
        assert_eq!(fit("I said ", "and left", "Hello."), "hello ");
        // Punctuation follows: no space before it.
        assert_eq!(fit("Say ", ", please", "Hello."), "hello");
        // Only the current line counts.
        assert_eq!(fit("A line\n", "", "New start."), "New start. ");
        // "I", names written in the field and dictionary words keep their capital.
        assert_eq!(fit("and", "", "I agree."), " I agree. ");
        assert_eq!(fit("talked to Chris and", "", "Chris agreed."), " Chris agreed. ");
        assert_eq!(fit_to_field("Bars works.", &field("so", ""), true, &["Bars".into()]), " Bars works. ");
        // Opening brackets take no space.
        assert_eq!(fit("call (", ")", "Maybe"), "maybe");
        // End of line without the trailing-space setting.
        assert_eq!(fit_to_field("Hi.", &field("", ""), false, &[]), "Hi.");
    }

    #[test]
    fn fits_a_replaced_selection() {
        let f = Surroundings { before: "The ".into(), selected: "quick brown".into(), after: " fox".into() };
        assert_eq!(fit_to_field("Slow red.", &f, true, &[]), "slow red");
        let f = Surroundings { before: String::new(), selected: " Old text. ".into(), after: String::new() };
        assert_eq!(fit_to_field("New text.", &f, true, &[]), " New text. ");
    }

    #[test]
    fn splits_long_pastes_for_claude_code() {
        let short = "one\ntwo\nthree";
        assert_eq!(split_for_agent(short, Agent::ClaudeCode), vec![short]);
        let text = "a\nb\nc\nd\ne";
        let pieces = split_for_agent(text, Agent::ClaudeCode);
        assert_eq!(pieces, vec!["a\nb\nc", "\nd\ne"]);
        let long = format!("{}\n{}\n{}", "x".repeat(700), "y".repeat(10), "z".repeat(1300));
        let pieces = split_for_agent(&long, Agent::ClaudeCode);
        assert_eq!(pieces.concat(), long);
        for p in &pieces {
            assert!(p.chars().count() <= 600 && p.matches('\n').count() <= 2, "{p:?}");
        }
        assert_eq!(split_for_agent(&"é".repeat(2500), Agent::Codex).len(), 3);
        assert_eq!(split_for_agent("short\n\n\n\nlines", Agent::Codex).len(), 1);
    }

    #[test]
    fn finalize_spacing() {
        assert_eq!(finalize("Hello.", true), "Hello. ");
        assert_eq!(finalize("Hello.\n", true), "Hello.\n");
        assert_eq!(finalize("Hello.", false), "Hello.");
    }
}

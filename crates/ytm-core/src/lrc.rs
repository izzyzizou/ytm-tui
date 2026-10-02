//! LRC parsing: `[mm:ss.xx]text`, multiple stamps per line, `[offset:±ms]`, metadata tags.

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLine {
    pub at_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    /// False for plain (unsynced) lyrics: every `at_ms` is 0.
    pub synced: bool,
    /// Where they came from, shown in the panel footer ("lrclib", "local", ...).
    pub source: String,
}

impl Lyrics {
    pub fn plain(text: &str, source: &str) -> Self {
        Self {
            lines: text.lines().map(|l| LyricLine { at_ms: 0, text: l.trim_end().to_string() }).collect(),
            synced: false,
            source: source.into(),
        }
    }

    /// Parse LRC text. Lines without timestamps are dropped if any line has one;
    /// if none do, the text is treated as plain lyrics.
    pub fn parse_lrc(text: &str, source: &str) -> Self {
        let mut offset_ms: i64 = 0;
        let mut lines = Vec::new();
        for raw in text.lines() {
            let mut rest = raw.trim();
            let mut stamps = Vec::new();
            while let Some(after) = rest.strip_prefix('[') {
                let Some(end) = after.find(']') else { break };
                let tag = &after[..end];
                rest = &after[end + 1..];
                if let Some(ms) = parse_stamp(tag) {
                    stamps.push(ms);
                } else if let Some(v) = tag.strip_prefix("offset:") {
                    offset_ms = v.trim().parse().unwrap_or(0);
                }
            }
            let text = strip_word_stamps(rest.trim());
            for ms in stamps {
                lines.push((ms, text.clone()));
            }
        }
        if lines.is_empty() {
            return Self::plain(text, source);
        }
        lines.sort_by_key(|(ms, _)| *ms);
        // LRC's offset: positive = lyrics appear earlier.
        let lines =
            lines.into_iter().map(|(ms, text)| LyricLine { at_ms: (ms as i64).saturating_sub(offset_ms).max(0) as u64, text }).collect();
        Self { lines, synced: true, source: source.into() }
    }

    /// Index of the line being sung at `pos` (with a user offset in ms, + = later).
    pub fn current(&self, pos: Duration, user_offset_ms: i64) -> Option<usize> {
        if !self.synced || self.lines.is_empty() {
            return None;
        }
        let t = (pos.as_millis() as i64).saturating_sub(user_offset_ms).max(0) as u64;
        self.lines.partition_point(|l| l.at_ms <= t).checked_sub(1)
    }
}

fn parse_stamp(tag: &str) -> Option<u64> {
    let (m, s) = tag.split_once(':')?;
    let m: u64 = m.trim().parse().ok()?;
    let s = s.trim().replace(':', ".");
    let (sec, frac) = s.split_once('.').unwrap_or((&s, "0"));
    let sec: u64 = sec.parse().ok()?;
    if sec >= 60 || frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let frac_ms = match frac.len() {
        1 => frac.parse::<u64>().ok()? * 100,
        2 => frac.parse::<u64>().ok()? * 10,
        _ => frac[..3].parse::<u64>().ok()?,
    };
    m.checked_mul(60_000)?.checked_add(sec * 1000 + frac_ms)
}

/// Enhanced LRC word timings `<mm:ss.xx>` are removed (word highlighting is a TODO).
fn strip_word_stamps(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        match rest[i..].find('>') {
            Some(j) if parse_stamp(&rest[i + 1..i + j]).is_some() => {
                out.push_str(&rest[..i]);
                rest = &rest[i + j + 1..];
            }
            _ => {
                out.push_str(&rest[..=i]);
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LRC: &str = "[ar:Lowtide Assembly]\n[ti:Glasswater Signals]\n[offset:+500]\n[00:12.00]harbour goes quiet\n[00:15.5][01:40.25]glass on the water\n[00:19.123] <00:19.20>keeps <00:19.80>what I lost\nno stamp here\n";

    #[test]
    fn parses_lrc() {
        let l = Lyrics::parse_lrc(LRC, "test");
        assert!(l.synced);
        let got: Vec<(u64, &str)> = l.lines.iter().map(|x| (x.at_ms, x.text.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (11_500, "harbour goes quiet"),
                (15_000, "glass on the water"),
                (18_623, "keeps what I lost"),
                (99_750, "glass on the water")
            ]
        );
    }

    #[test]
    fn current_line() {
        let l = Lyrics::parse_lrc(LRC, "test");
        assert_eq!(l.current(Duration::from_secs(5), 0), None);
        assert_eq!(l.current(Duration::from_millis(11_500), 0), Some(0));
        assert_eq!(l.current(Duration::from_secs(16), 0), Some(1));
        assert_eq!(l.current(Duration::from_secs(16), 2000), Some(0), "user offset delays lyrics");
        assert_eq!(l.current(Duration::from_secs(300), 0), Some(3));
    }

    #[test]
    fn plain_fallback() {
        let l = Lyrics::parse_lrc("line one\nline two", "ytm");
        assert!(!l.synced);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.current(Duration::from_secs(1), 0), None);
    }
}

//! The bounded window a pane keeps of its worker's output.
//!
//! Full terminal logs are never operator-journal content: the spec routes them
//! to disposable signal frames. What lands durably is a tail plus an honest
//! count of what was dropped, so a reader can never mistake it for the whole
//! log.

use std::collections::VecDeque;

/// Lines a pane keeps by default.
pub const PANE_TAIL_LINES: usize = 200;
/// Bytes each kept line is truncated to.
pub const PANE_LINE_BYTES: usize = 2_000;

#[derive(Clone, Debug)]
pub struct PaneTail {
    lines: VecDeque<String>,
    capacity: usize,
    line_bytes: usize,
    dropped: usize,
}

impl PaneTail {
    pub fn new(capacity: usize, line_bytes: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            capacity: capacity.max(1),
            line_bytes: line_bytes.max(1),
            dropped: 0,
        }
    }

    pub fn push(&mut self, line: &str) {
        self.lines
            .push_back(truncate_on_char_boundary(line, self.line_bytes));
        while self.lines.len() > self.capacity {
            self.lines.pop_front();
            self.dropped += 1;
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    pub fn dropped_lines(&self) -> usize {
        self.dropped
    }
}

impl Default for PaneTail {
    fn default() -> Self {
        Self::new(PANE_TAIL_LINES, PANE_LINE_BYTES)
    }
}

/// Truncate to at most `max_bytes`, never splitting a UTF-8 character.
///
/// A naive `&value[..max_bytes]` panics mid-character, which would turn a
/// worker printing non-ASCII into a crash in the code that records it.
fn truncate_on_char_boundary(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tail_keeps_only_the_last_lines_and_counts_what_it_dropped() {
        let mut tail = PaneTail::new(3, 100);
        for line in ["one", "two", "three", "four", "five"] {
            tail.push(line);
        }
        assert_eq!(tail.lines(), ["three", "four", "five"]);
        assert_eq!(tail.dropped_lines(), 2);
    }

    #[test]
    fn a_long_line_is_truncated_on_a_character_boundary() {
        let mut tail = PaneTail::new(4, 8);
        tail.push("aaaaaaaaaaaaaaaa");
        assert_eq!(tail.lines(), ["aaaaaaaa"]);
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        let mut tail = PaneTail::new(4, 5);
        // Four 3-byte characters: a naive byte slice at 5 would panic.
        tail.push("日本語だ");
        let kept = &tail.lines()[0];
        assert!(kept.len() <= 5, "kept {} bytes", kept.len());
        assert_eq!(kept, "日");
    }

    #[test]
    fn an_empty_tail_reports_nothing_dropped() {
        let tail = PaneTail::new(3, 100);
        assert!(tail.lines().is_empty());
        assert_eq!(tail.dropped_lines(), 0);
    }
}

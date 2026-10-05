//! Per-job log positions are logical source lines, independent of widget geometry.
//! Only visible lines are rendered; even very large traces never become a terminal-
//! sized layout tree. Fetched history is kept in memory until its route is discarded.
use super::Trace;
use crate::ansi::{SgrState, csi_end};
use std::ops::Range;
use tui_lipan::{
    prelude::*,
    style::{RowStylePolicy, ansi::parse_ansi},
};

const CURRENT_MATCH_BG: Color = Color::Rgb(255, 215, 0);
const OTHER_MATCH_BG: Color = Color::Rgb(225, 211, 150);

#[derive(Default)]
pub struct TraceIndex {
    bytes: usize,
    starts: Vec<usize>,
    scanned: usize,
    sgr: SgrState,
    // Sparse start-of-line checkpoints: unchanged styles consume no extra space.
    styles: Vec<(usize, String)>,
}

impl TraceIndex {
    fn append(&mut self, text: &str, from: usize) {
        if self.starts.is_empty() && !text.is_empty() {
            self.starts.push(0);
        }
        self.starts.extend(
            text.as_bytes()[from..]
                .iter()
                .enumerate()
                .filter_map(|(i, b)| (*b == b'\n').then_some(from + i + 1)),
        );
        let mut cursor = self.scanned;
        while cursor < text.len() {
            let byte = text.as_bytes()[cursor];
            if byte == b'\x1b' && cursor + 1 == text.len() {
                break;
            }
            if byte == b'\x1b' && text.as_bytes().get(cursor + 1) == Some(&b'[') {
                let Some(end) = csi_end(text, cursor) else {
                    break;
                };
                if text.as_bytes()[end - 1] == b'm' {
                    self.sgr.apply(&text[cursor + 2..end - 1]);
                }
                cursor = end;
                continue;
            }
            if byte == b'\n' {
                let prefix = self.sgr.prefix();
                if self.styles.last().map_or("", |(_, value)| value.as_str()) != prefix {
                    self.styles.push((cursor + 1, prefix));
                }
            }
            cursor += 1;
        }
        self.scanned = cursor;
        self.bytes = text.len();
    }
}

impl Trace {
    pub fn append(&mut self, text: &str, reset: bool) {
        if reset {
            self.text.clear();
            *self.index.get_mut() = TraceIndex::default();
        }
        self.ensure_index();
        let from = self.text.len();
        self.text.push_str(text);
        self.index.get_mut().append(&self.text, from);
    }

    fn ensure_index(&self) {
        let mut index = self.index.borrow_mut();
        if index.bytes != self.text.len() {
            *index = TraceIndex::default();
            index.append(&self.text, 0);
        }
    }

    pub fn line_count(&self) -> usize {
        self.ensure_index();
        let index = self.index.borrow();
        index
            .starts
            .len()
            .saturating_sub(usize::from(index.starts.last() == Some(&self.text.len())))
    }

    pub fn plain_line(&self, row: usize) -> String {
        parse_ansi(self.line(row))
            .into_iter()
            .map(|span| span.content.to_string())
            .collect()
    }

    pub fn line_spans(&self, row: usize, query: &str) -> Vec<Span> {
        self.line_spans_with_current(row, query, false)
    }

    pub(crate) fn line_spans_with_current(
        &self,
        row: usize,
        query: &str,
        current: bool,
    ) -> Vec<Span> {
        self.ensure_index();
        let prefix = {
            let index = self.index.borrow();
            let start = index.starts.get(row).copied().unwrap_or(self.text.len());
            let checkpoint = index.styles.partition_point(|(offset, _)| *offset <= start);
            checkpoint
                .checked_sub(1)
                .map(|i| index.styles[i].1.clone())
                .unwrap_or_default()
        };
        let spans = parse_ansi(&format!("{prefix}{}", self.line(row)));
        highlight(spans, query, current)
    }

    pub fn line(&self, row: usize) -> &str {
        self.ensure_index();
        let index = self.index.borrow();
        let Some(&start) = index.starts.get(row) else {
            return "";
        };
        let end = index
            .starts
            .get(row + 1)
            .copied()
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }
}

/// Match in displayed text, never ANSI bytes. Unicode lowercase expansions are
/// mapped back to the original UTF-8 character boundaries (e.g. İ -> i + dot).
fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let query = query.to_lowercase();
    if text.is_ascii() {
        return text
            .to_lowercase()
            .match_indices(&query)
            .map(|(i, value)| i..i + value.len())
            .collect();
    }
    // Use the same string-level lowercase operation as navigation (including
    // contextual Greek sigma), while mapping length expansions back to source.
    let folded = text.to_lowercase();
    let mut folded_offset = 0;
    let mut boundaries = Vec::new();
    for (start, ch) in text.char_indices() {
        boundaries.push((folded_offset, start, start + ch.len_utf8()));
        folded_offset += ch.to_lowercase().map(char::len_utf8).sum::<usize>();
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (start, value) in folded.match_indices(&query) {
        let first = boundaries.partition_point(|&(offset, _, _)| offset <= start) - 1;
        let last = boundaries.partition_point(|&(offset, _, _)| offset < start + value.len()) - 1;
        let range = boundaries[first].1..boundaries[last].2;
        if let Some(previous) = ranges
            .last_mut()
            .filter(|previous| previous.end >= range.start)
        {
            previous.end = previous.end.max(range.end);
        } else {
            ranges.push(range);
        }
    }
    ranges
}

fn highlight(spans: Vec<Span>, query: &str, current: bool) -> Vec<Span> {
    let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
    let matches = match_ranges(&text, query);
    let mut result = Vec::new();
    let mut base = 0;
    for span in spans {
        let end = base + span.content.len();
        let mut cursor = base;
        let normal = span.style.contrast_policy(ContrastPolicy::Off);
        let mut marked = normal.fg(Color::Black).bg(if current {
            CURRENT_MATCH_BG
        } else {
            OTHER_MATCH_BG
        });
        marked.reverse = Some(false);
        marked.dim = Some(false);
        for range in matches.iter().filter(|r| r.start < end && r.end > base) {
            let start = range.start.max(base);
            let stop = range.end.min(end);
            if start > cursor {
                result.push(
                    Span::new(span.content[cursor - base..start - base].to_owned())
                        .style(normal)
                        .row_style_policy(RowStylePolicy::Disabled),
                );
            }
            result.push(
                Span::new(span.content[start - base..stop - base].to_owned())
                    .style(marked)
                    .row_style_policy(RowStylePolicy::Disabled),
            );
            cursor = stop;
        }
        if cursor < end {
            result.push(
                Span::new(span.content[cursor - base..].to_owned())
                    .style(normal)
                    .row_style_policy(RowStylePolicy::Disabled),
            );
        }
        base = end;
    }
    result
}

pub struct LogView {
    pub offset: usize,
    pub follow: bool,
    pub query: String,
    pub matches: Vec<usize>,
    pub current_match: Option<usize>,
    pub drag_grab: Option<usize>,
    pub wheel_epoch: u64,
}

impl Default for LogView {
    fn default() -> Self {
        Self {
            offset: 0,
            follow: true,
            query: String::new(),
            matches: Vec::new(),
            current_match: None,
            drag_grab: None,
            wheel_epoch: 0,
        }
    }
}

impl LogView {
    pub fn position(&self, total: usize, height: usize) -> usize {
        let max = total.saturating_sub(height.max(1));
        if self.follow {
            max
        } else {
            self.offset.min(max)
        }
    }

    pub fn scroll(&mut self, total: usize, height: usize, delta: isize) {
        let max = total.saturating_sub(height.max(1));
        let position = self
            .position(total, height)
            .saturating_add_signed(delta)
            .min(max);
        self.offset = position;
        self.follow = position == max;
    }

    pub fn refresh_matches(&mut self, trace: &Trace) {
        let selected = self
            .current_match
            .and_then(|i| self.matches.get(i))
            .copied();
        self.matches.clear();
        if !self.query.is_empty() {
            let query = self.query.to_lowercase();
            self.matches.extend(
                (0..trace.line_count())
                    .filter(|&row| trace.plain_line(row).to_lowercase().contains(&query)),
            );
        }
        self.current_match = selected.and_then(|row| self.matches.iter().position(|&r| r == row));
    }

    pub fn find(&mut self, trace: &Trace, height: usize, backwards: bool) {
        self.refresh_matches(trace);
        if self.matches.is_empty() {
            self.current_match = None;
            return;
        }
        let position = self.position(trace.line_count(), height);
        let index = match self.current_match {
            Some(i) if backwards => (i + self.matches.len() - 1) % self.matches.len(),
            Some(i) => (i + 1) % self.matches.len(),
            None if backwards => self
                .matches
                .iter()
                .rposition(|&row| row <= position)
                .unwrap_or(self.matches.len() - 1),
            None => self
                .matches
                .iter()
                .position(|&row| row >= position)
                .unwrap_or(0),
        };
        self.current_match = Some(index);
        self.offset = self.matches[index].min(trace.line_count().saturating_sub(height.max(1)));
        self.follow = self.offset == trace.line_count().saturating_sub(height.max(1));
    }

    pub fn reset(&mut self) {
        self.offset = 0;
        self.follow = true;
        self.current_match = None;
        self.matches.clear();
        self.drag_grab = None;
    }
}

/// `(thumb top, thumb height)`, using usize throughout so history is not limited
/// by terminal Rect's i16/u16 dimensions.
pub(super) fn scrollbar(total: usize, height: usize, offset: usize) -> (usize, usize) {
    let height = height.max(1);
    let thumb = ((height as u128 * height as u128) / total.max(1) as u128)
        .max(1)
        .min(height as u128) as usize;
    let travel = height - thumb;
    let max = total.saturating_sub(height);
    let top = if max == 0 {
        0
    } else {
        (offset.min(max) as u128 * travel as u128 / max as u128) as usize
    };
    (top, thumb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_index_matches_source_lines_at_every_chunk_boundary() {
        let text = "first\n\nλ second\r\nthird without newline";
        for split in (0..=text.len()).filter(|&i| text.is_char_boundary(i)) {
            let mut trace = Trace::default();
            trace.append(&text[..split], false);
            assert_eq!(trace.line_count(), text[..split].lines().count());
            trace.append(&text[split..], false);
            assert_eq!(trace.line_count(), text.lines().count());
            for (row, expected) in text.lines().enumerate() {
                assert_eq!(trace.line(row), expected);
            }
            trace.append("\nfinal\n", false);
            assert_eq!(trace.line_count(), 5);
            assert_eq!(trace.line(4), "final");
            trace.append("new\n", true);
            assert_eq!(trace.line_count(), 1);
            assert_eq!(trace.line(0), "new");
        }
    }

    #[test]
    fn positions_follow_only_at_the_end_and_resize_keeps_source_lines() {
        let mut view = LogView::default();
        assert_eq!(view.position(100, 10), 90);
        view.scroll(100, 10, -20);
        assert_eq!(view.position(100, 10), 70);
        assert_eq!(view.position(120, 20), 70);
        view.scroll(120, 20, isize::MAX);
        assert!(view.follow);
        assert_eq!(view.position(140, 15), 125);
        view.scroll(140, 15, isize::MIN);
        assert_eq!(view.position(140, 15), 0);
        view.scroll(2, 18, -1);
        assert!(view.follow);
        assert_eq!(view.position(2, 18), 0);
    }

    #[test]
    fn searches_are_literal_unicode_case_insensitive_and_can_find_new_output() {
        let mut trace = Trace::default();
        trace.append("begin\nNEEDLE λ .*\nend\n", false);
        let mut view = LogView {
            query: "needle λ .*".into(),
            follow: false,
            ..LogView::default()
        };
        view.find(&trace, 1, false);
        assert_eq!(view.matches, vec![1]);
        assert_eq!(view.offset, 1);
        trace.append("needle λ .*\n", false);
        view.find(&trace, 1, false);
        assert_eq!(view.matches, vec![1, 3]);
        assert_eq!(view.offset, 3);
        assert!(view.follow);
        view.find(&trace, 1, true);
        assert_eq!(view.offset, 1);
        view.query = "absent".into();
        view.find(&trace, 1, false);
        assert!(view.matches.is_empty());
        assert_eq!(view.offset, 1);
    }

    #[test]
    fn ansi_styles_survive_scrolling_chunk_boundaries_and_resets() {
        let mut trace = Trace::default();
        trace.append("\x1b[1;38;2;12;34;56;48;5;235mfirst\nsecond\n", false);
        let spans = trace.line_spans(1, "");
        assert_eq!(
            spans[0].style.fg,
            Style::new().fg(Color::Rgb(12, 34, 56)).fg
        );
        assert_eq!(spans[0].style.bg, Style::new().bg(Color::Indexed(235)).bg);
        assert_eq!(spans[0].style.bold, Some(true));
        trace.append("\x1b[22;39;", false);
        trace.append("49mnormal\nnext\n", false);
        for row in [2, 3] {
            let spans = trace.line_spans(row, "");
            assert_eq!(spans[0].style.fg, None);
            assert_eq!(spans[0].style.bg, None);
            assert_ne!(spans[0].style.bold, Some(true));
        }
        trace.append("replacement\n", true);
        assert_eq!(trace.line_spans(0, "")[0].style.fg, None);
        let colored = "\x1b[31mλ first\nsecond\n\x1b[0mnormal";
        for split in (0..=colored.len()).filter(|&i| colored.is_char_boundary(i)) {
            let mut trace = Trace::default();
            trace.append(&colored[..split], false);
            trace.append(&colored[split..], false);
            assert_eq!(
                trace.line_spans(1, "")[0].style.fg,
                Style::new().fg(Color::Red).fg,
                "split {split}"
            );
            assert_eq!(trace.line_spans(2, "")[0].style.fg, None);
        }
    }

    #[test]
    fn highlights_match_text_across_ansi_spans_and_restore_colors_afterwards() {
        let mut trace = Trace::default();
        trace.append(
            "\x1b[31mred nee\x1b[32mdle green NEEDLE end\x1b[0m\n",
            false,
        );
        let spans = trace.line_spans(0, "needle");
        assert_eq!(
            spans.iter().map(|s| s.content.as_ref()).collect::<String>(),
            "red needle green NEEDLE end"
        );
        let highlighted: String = spans
            .iter()
            .filter(|s| s.style.bg == Style::new().bg(OTHER_MATCH_BG).bg)
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(highlighted, "needleNEEDLE");
        let active = trace.line_spans_with_current(0, "needle", true);
        for (active, other) in active.iter().zip(&spans) {
            assert_eq!(active.content, other.content);
            if other.style.bg == Style::new().bg(OTHER_MATCH_BG).bg {
                assert_eq!(active.style.bg, Style::new().bg(CURRENT_MATCH_BG).bg);
                assert_eq!(active.style.fg, Style::new().fg(Color::Black).fg);
            } else {
                assert_eq!(
                    active.style, other.style,
                    "non-matching ANSI styles are unchanged"
                );
            }
        }
        assert_eq!(
            spans.first().unwrap().style.fg,
            Style::new().fg(Color::Red).fg
        );
        assert_eq!(
            spans.last().unwrap().style.fg,
            Style::new().fg(Color::Green).fg
        );
        assert_eq!(spans.last().unwrap().style.bg, None);
        let mut view = LogView {
            query: "needle".into(),
            ..LogView::default()
        };
        view.refresh_matches(&trace);
        assert_eq!(view.matches, vec![0]);
        view.query = "31".into();
        view.refresh_matches(&trace);
        assert!(
            view.matches.is_empty(),
            "SGR parameters are not searchable text"
        );
    }

    #[test]
    fn unicode_highlights_preserve_original_character_boundaries() {
        let mut trace = Trace::default();
        trace.append("İ λ İ\n", false);
        let spans = trace.line_spans(0, "i");
        let highlighted: String = spans
            .iter()
            .filter(|s| s.style.bg == Style::new().bg(OTHER_MATCH_BG).bg)
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(highlighted, "İİ");
        assert_eq!(
            spans.iter().map(|s| s.content.as_ref()).collect::<String>(),
            "İ λ İ"
        );
        let mut view = LogView {
            query: "i".into(),
            ..LogView::default()
        };
        view.refresh_matches(&trace);
        assert_eq!(view.matches, vec![0]);
        trace.append("ΟΣ", true);
        let spans = trace.line_spans(0, "ΟΣ");
        assert_eq!(spans[0].style.bg, Style::new().bg(OTHER_MATCH_BG).bg);
    }

    #[test]
    fn scrollbar_geometry_handles_extreme_histories_without_overflow() {
        for total in [0, 1, 10, 100, 100_000, usize::MAX] {
            for height in [1, 10, 50] {
                let max = total.saturating_sub(height);
                let (top, thumb) = scrollbar(total, height, 0);
                assert_eq!(top, 0);
                assert!((1..=height).contains(&thumb));
                let (top, thumb) = scrollbar(total, height, max);
                assert!(top + thumb <= height);
                if total > height {
                    assert_eq!(top + thumb, height);
                }
            }
        }
    }
}

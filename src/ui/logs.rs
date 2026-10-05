//! Per-job log positions are logical source lines, independent of widget geometry.
//! Only visible lines are rendered; even very large traces never become a terminal-
//! sized layout tree. Fetched history is kept in memory until its route is discarded.
use super::Trace;

#[derive(Default)]
pub struct TraceIndex {
    bytes: usize,
    starts: Vec<usize>,
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
                    .filter(|&row| trace.line(row).to_lowercase().contains(&query)),
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

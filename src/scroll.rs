//! Boundary scrolling with a quarter-viewport margin and no wrapping.
//!
//! With `h` visible rows, the cursor band is `h / 4 ..= h - h / 4 - 1`
//! (zero-based, inclusive). Downward movement reaches the bottom of that band
//! before scrolling; reversal traverses the band before scrolling upward.
//! At the list boundaries the cursor can enter the margins. A zero-height
//! viewport is treated as one row for state normalization.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoundaryScroll {
    pub selected: usize,
    pub offset: usize,
}

impl BoundaryScroll {
    /// Clamp stale state after filtering or resizing, retaining the offset when
    /// the selection is already inside the cursor band.
    pub fn normalize(&mut self, len: usize, height: usize) {
        if len == 0 {
            self.selected = 0;
            self.offset = 0;
            return;
        }
        self.selected = self.selected.min(len - 1);
        let height = height.max(1).min(len);
        let margin = height / 4;
        let bottom = height - margin - 1;
        let max_offset = len - height;
        self.offset = self.offset.min(max_offset);

        if self.selected < self.offset + margin {
            self.offset = self.selected.saturating_sub(margin);
        } else if self.selected > self.offset + bottom {
            self.offset = self.selected - bottom;
        }
        self.offset = self.offset.min(max_offset);
    }

    pub fn move_by(&mut self, delta: isize, len: usize, height: usize) {
        // Normalize first so a stale index after filtering is not the origin of
        // the next movement. Saturating arithmetic also handles isize::MIN.
        self.normalize(len, height);
        self.selected = self.selected.saturating_add_signed(delta);
        self.normalize(len, height);
    }

    /// Apply a viewport drag/wheel movement without pulling it back to the old selection.
    pub fn scroll_to(&mut self, offset: usize, len: usize, height: usize) {
        if len == 0 {
            *self = Self::default();
            return;
        }
        let height = height.max(1).min(len);
        let max_offset = len - height;
        self.offset = offset.min(max_offset);
        let margin = height / 4;
        let first = if self.offset == 0 {
            0
        } else {
            self.offset + margin
        };
        let last = if self.offset == max_offset {
            len - 1
        } else {
            self.offset + height - margin - 1
        };
        self.selected = self.selected.clamp(first, last);
    }

    pub fn select(&mut self, index: usize, len: usize, height: usize) {
        self.selected = index;
        self.normalize(len, height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_state(scroll: &BoundaryScroll, selected: usize, offset: usize) {
        assert_eq!((scroll.selected, scroll.offset), (selected, offset));
    }

    fn assert_invariants(scroll: &BoundaryScroll, len: usize, height: usize) {
        if len == 0 {
            assert_state(scroll, 0, 0);
            return;
        }
        let height = height.max(1).min(len);
        let margin = height / 4;
        assert!(scroll.selected < len);
        assert!(scroll.offset <= len - height);
        assert!(scroll.selected >= scroll.offset);
        assert!(scroll.selected - scroll.offset < height);
        if scroll.offset > 0 {
            assert!(scroll.selected - scroll.offset >= margin);
        }
        if scroll.offset < len - height {
            assert!(scroll.selected - scroll.offset < height - margin);
        }
    }

    #[test]
    fn exact_downward_sequence_reaches_three_quarters_then_scrolls_to_end() {
        let mut scroll = BoundaryScroll::default();
        // Eight rows leave two margin rows at each side: cursor rows 2..=5.
        let offsets = [
            0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 12, 12,
        ];
        for (selected, offset) in offsets.into_iter().enumerate() {
            if selected > 0 {
                scroll.move_by(1, 20, 8);
            }
            assert_state(&scroll, selected, offset);
        }
        scroll.move_by(1, 20, 8);
        assert_state(&scroll, 19, 12);
    }

    #[test]
    fn exact_upward_sequence_reaches_one_quarter_then_scrolls_to_start() {
        let mut scroll = BoundaryScroll::default();
        scroll.select(19, 20, 8);
        let offsets = [
            12, 12, 12, 12, 12, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 0,
        ];
        for (step, offset) in offsets.into_iter().enumerate() {
            if step > 0 {
                scroll.move_by(-1, 20, 8);
            }
            assert_state(&scroll, 19 - step, offset);
        }
        scroll.move_by(-1, 20, 8);
        assert_state(&scroll, 0, 0);
    }

    #[test]
    fn reversing_direction_crosses_the_band_without_scrolling() {
        let mut scroll = BoundaryScroll::default();
        scroll.select(12, 30, 8);
        assert_state(&scroll, 12, 7);
        for selected in (9..12).rev() {
            scroll.move_by(-1, 30, 8);
            assert_state(&scroll, selected, 7);
        }
        scroll.move_by(-1, 30, 8);
        assert_state(&scroll, 8, 6);
        for selected in 9..=11 {
            scroll.move_by(1, 30, 8);
            assert_state(&scroll, selected, 6);
        }
        scroll.move_by(1, 30, 8);
        assert_state(&scroll, 12, 7);
    }

    #[test]
    fn large_moves_and_selection_are_clamped_without_wrapping() {
        let mut scroll = BoundaryScroll::default();
        scroll.move_by(isize::MAX, 100, 12);
        assert_state(&scroll, 99, 88);
        scroll.move_by(isize::MIN, 100, 12);
        assert_state(&scroll, 0, 0);
        scroll.select(usize::MAX, 100, 12);
        assert_state(&scroll, 99, 88);
        scroll.select(50, 100, 12);
        assert_state(&scroll, 50, 47);
        scroll.select(0, 100, 12);
        assert_state(&scroll, 0, 0);
    }

    #[test]
    fn zero_and_one_row_viewports_follow_the_cursor() {
        for height in [0, 1] {
            let mut scroll = BoundaryScroll::default();
            for selected in 0..10 {
                scroll.select(selected, 10, height);
                assert_state(&scroll, selected, selected);
            }
            for selected in (0..9).rev() {
                scroll.move_by(-1, 10, height);
                assert_state(&scroll, selected, selected);
            }
        }
    }

    #[test]
    fn two_and_three_rows_have_no_rounded_up_margin() {
        for height in [2, 3] {
            let mut scroll = BoundaryScroll::default();
            for selected in 0usize..10 {
                scroll.select(selected, 10, height);
                assert_state(&scroll, selected, selected.saturating_sub(height - 1));
            }
            for selected in (0..9).rev() {
                scroll.move_by(-1, 10, height);
                assert_state(&scroll, selected, selected.min(10 - height));
            }
        }
    }

    #[test]
    fn four_rows_have_one_row_margins() {
        let mut scroll = BoundaryScroll::default();
        for (selected, offset) in [0, 0, 0, 1, 2, 3, 4, 5, 6, 6].into_iter().enumerate() {
            scroll.select(selected, 10, 4);
            assert_state(&scroll, selected, offset);
        }
    }

    #[test]
    fn empty_lists_always_reset_state() {
        for height in [0, 1, 8, usize::MAX] {
            let mut scroll = BoundaryScroll {
                selected: usize::MAX,
                offset: usize::MAX,
            };
            scroll.normalize(0, height);
            assert_state(&scroll, 0, 0);
            scroll.move_by(isize::MAX, 0, height);
            assert_state(&scroll, 0, 0);
            scroll.move_by(isize::MIN, 0, height);
            assert_state(&scroll, 0, 0);
            scroll.select(usize::MAX, 0, height);
            assert_state(&scroll, 0, 0);
        }
    }

    #[test]
    fn lists_fitting_the_viewport_never_scroll() {
        for len in 1..=12 {
            for height in [len, len + 1, usize::MAX] {
                let mut scroll = BoundaryScroll {
                    selected: usize::MAX,
                    offset: usize::MAX,
                };
                for selected in 0..len {
                    scroll.select(selected, len, height);
                    assert_state(&scroll, selected, 0);
                }
            }
        }
    }

    #[test]
    fn resizing_restores_margins_without_unnecessary_offset_changes() {
        let mut scroll = BoundaryScroll {
            selected: 12,
            offset: 7,
        };
        scroll.normalize(30, 8);
        assert_state(&scroll, 12, 7);
        scroll.normalize(30, 4);
        assert_state(&scroll, 12, 10);
        scroll.normalize(30, 12);
        assert_state(&scroll, 12, 9);
        scroll.normalize(30, 100);
        assert_state(&scroll, 12, 0);
        scroll.normalize(30, 0);
        assert_state(&scroll, 12, 12);
        scroll.normalize(30, 8);
        assert_state(&scroll, 12, 10);
    }

    #[test]
    fn filtering_clamps_selection_and_offset_before_next_movement() {
        let mut scroll = BoundaryScroll {
            selected: 90,
            offset: 85,
        };
        scroll.normalize(10, 8);
        assert_state(&scroll, 9, 2);
        scroll.normalize(3, 8);
        assert_state(&scroll, 2, 0);
        scroll.normalize(0, 8);
        assert_state(&scroll, 0, 0);
        scroll.normalize(20, 8);
        assert_state(&scroll, 0, 0);

        scroll.select(90, 100, 8);
        scroll.move_by(-1, 10, 8);
        assert_state(&scroll, 8, 2);
    }

    #[test]
    fn scroll_to_empty_lists_resets_stale_state() {
        for height in [0, 1, 8, usize::MAX] {
            for offset in [0, 10, usize::MAX] {
                let mut scroll = BoundaryScroll {
                    selected: usize::MAX,
                    offset: usize::MAX,
                };
                scroll.scroll_to(offset, 0, height);
                assert_state(&scroll, 0, 0);
            }
        }
    }

    #[test]
    fn scroll_to_tiny_viewports_use_the_full_visible_band() {
        for height in 0usize..=3 {
            let last = 4 + height.max(1) - 1;
            for (selected, expected) in [(0, 4), (usize::MAX, last)] {
                let mut scroll = BoundaryScroll {
                    selected,
                    offset: usize::MAX,
                };
                scroll.scroll_to(4, 10, height);
                assert_state(&scroll, expected, 4);
            }
            for selected in 4..=last {
                let mut scroll = BoundaryScroll {
                    selected,
                    offset: 0,
                };
                scroll.scroll_to(4, 10, height);
                assert_state(&scroll, selected, 4);
            }
        }
    }

    #[test]
    fn scroll_to_lists_fitting_the_viewport_clamps_only_stale_selection() {
        for len in 1..=12 {
            for height in [len, len + 1, usize::MAX] {
                for selected in (0..=len).chain(std::iter::once(usize::MAX)) {
                    let mut scroll = BoundaryScroll {
                        selected,
                        offset: usize::MAX,
                    };
                    scroll.scroll_to(usize::MAX, len, height);
                    assert_state(&scroll, selected.min(len - 1), 0);
                }
            }
        }
    }

    #[test]
    fn scroll_to_clamps_offsets_and_allows_selection_in_boundary_margins() {
        for (offset, selected, expected_selected, expected_offset) in [
            (0, 0, 0, 0),
            (0, 1, 1, 0),
            (0, 29, 5, 0),
            (22, 0, 24, 22),
            (22, 28, 28, 22),
            (22, 29, 29, 22),
            (23, 0, 24, 22),
            (usize::MAX, usize::MAX, 29, 22),
        ] {
            let mut scroll = BoundaryScroll {
                selected,
                offset: usize::MAX,
            };
            scroll.scroll_to(offset, 30, 8);
            assert_state(&scroll, expected_selected, expected_offset);
            assert_invariants(&scroll, 30, 8);
        }
    }

    #[test]
    fn scroll_to_preserves_selection_inside_the_quarter_band() {
        for offset in [1, 7, 21] {
            for selected in offset + 2..=offset + 5 {
                let mut scroll = BoundaryScroll {
                    selected,
                    offset: 0,
                };
                scroll.scroll_to(offset, 30, 8);
                assert_state(&scroll, selected, offset);
            }
        }
    }

    #[test]
    fn scroll_to_moves_offscreen_and_margin_selection_to_the_nearest_band_edge() {
        // Offset seven shows rows 7..=14, with the cursor band at 9..=12.
        for (selected, expected) in [
            (0, 9),
            (6, 9),
            (7, 9),
            (8, 9),
            (13, 12),
            (14, 12),
            (15, 12),
            (29, 12),
            (usize::MAX, 12),
        ] {
            let mut scroll = BoundaryScroll {
                selected,
                offset: 0,
            };
            scroll.scroll_to(7, 30, 8);
            assert_state(&scroll, expected, 7);
        }
    }

    #[test]
    fn scroll_to_is_stable_under_repeated_normalization() {
        for len in 0..=20 {
            for height in 0..=24 {
                for selected in 0..=24 {
                    for offset in 0..=24 {
                        let mut scroll = BoundaryScroll {
                            selected,
                            offset: usize::MAX,
                        };
                        scroll.scroll_to(offset, len, height);
                        assert_invariants(&scroll, len, height);
                        assert_eq!(scroll.offset, offset.min(len.saturating_sub(height.max(1))));
                        let dragged = scroll.clone();
                        scroll.normalize(len, height);
                        assert_eq!(scroll, dragged);
                        scroll.normalize(len, height);
                        assert_eq!(scroll, dragged);
                    }
                }
            }
        }
    }

    #[test]
    fn scroll_to_keyboard_moves_continue_from_the_dragged_selection() {
        for (selected, dragged, delta, states) in [
            (0, 9, 1, [(10, 7), (11, 7), (12, 7), (13, 8)]),
            (29, 12, -1, [(11, 7), (10, 7), (9, 7), (8, 6)]),
            (0, 9, -1, [(8, 6), (7, 5), (6, 4), (5, 3)]),
            (29, 12, 1, [(13, 8), (14, 9), (15, 10), (16, 11)]),
        ] {
            let mut scroll = BoundaryScroll::default();
            scroll.select(selected, 30, 8);
            scroll.scroll_to(7, 30, 8);
            assert_state(&scroll, dragged, 7);
            for (selected, offset) in states {
                scroll.move_by(delta, 30, 8);
                assert_state(&scroll, selected, offset);
            }
        }
    }

    #[test]
    fn maximum_usize_state_does_not_overflow() {
        for height in [0, 1, 4, 8, usize::MAX / 2, usize::MAX] {
            let mut scroll = BoundaryScroll {
                selected: usize::MAX,
                offset: usize::MAX,
            };
            scroll.normalize(usize::MAX, height);
            assert_invariants(&scroll, usize::MAX, height);
            scroll.move_by(isize::MAX, usize::MAX, height);
            assert_invariants(&scroll, usize::MAX, height);
            scroll.move_by(isize::MIN, usize::MAX, height);
            assert_invariants(&scroll, usize::MAX, height);
        }
    }

    #[test]
    fn exhaustive_normalization_is_idempotent_and_maintains_invariants() {
        for len in 0..=20 {
            for height in 0..=24 {
                for selected in 0..=24 {
                    for offset in 0..=24 {
                        let mut scroll = BoundaryScroll { selected, offset };
                        scroll.normalize(len, height);
                        assert_invariants(&scroll, len, height);
                        let normalized = scroll.clone();
                        scroll.normalize(len, height);
                        assert_eq!(scroll, normalized);
                    }
                }
            }
        }
    }

    #[test]
    fn repeated_full_traversals_for_every_small_viewport() {
        for len in 0..=32 {
            for height in 0..=36 {
                let mut scroll = BoundaryScroll::default();
                for _ in 0..3 {
                    for _ in 0..len + 2 {
                        scroll.move_by(1, len, height);
                        assert_invariants(&scroll, len, height);
                    }
                    assert_eq!(scroll.selected, len.saturating_sub(1));
                    for _ in 0..len + 2 {
                        scroll.move_by(-1, len, height);
                        assert_invariants(&scroll, len, height);
                    }
                    assert_state(&scroll, 0, 0);
                }
            }
        }
    }
}

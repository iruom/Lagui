//! Fixed-capacity UTF-8 text editing state with selection, composition and undo.

use crate::CapacityExceeded;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub cursor: usize,
}
impl Selection {
    pub fn ordered(self) -> core::ops::Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    pub fn is_empty(self) -> bool {
        self.anchor == self.cursor
    }
}

#[derive(Clone, Copy)]
struct Snapshot<const N: usize> {
    bytes: [u8; N],
    len: usize,
    selection: Selection,
}
impl<const N: usize> Default for Snapshot<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
            selection: Selection::default(),
        }
    }
}

/// Application-owned editor state. Clipboard integration is event-based through
/// `selected_text`, `cut`, and `paste`; the platform adapter owns the clipboard.
pub struct TextEdit<const N: usize, const HISTORY: usize = 32> {
    bytes: [u8; N],
    len: usize,
    selection: Selection,
    preedit: [u8; N],
    preedit_len: usize,
    preedit_selection: Selection,
    undo: [Snapshot<N>; HISTORY],
    undo_len: usize,
    redo: [Snapshot<N>; HISTORY],
    redo_len: usize,
}

impl<const N: usize, const HISTORY: usize> Default for TextEdit<N, HISTORY> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
            selection: Selection::default(),
            preedit: [0; N],
            preedit_len: 0,
            preedit_selection: Selection::default(),
            undo: [Snapshot::default(); HISTORY],
            undo_len: 0,
            redo: [Snapshot::default(); HISTORY],
            redo_len: 0,
        }
    }
}

impl<const N: usize, const HISTORY: usize> TextEdit<N, HISTORY> {
    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).expect("TextEdit preserves UTF-8")
    }
    pub fn selection(&self) -> Selection {
        self.selection
    }
    pub fn set_selection(&mut self, anchor: usize, cursor: usize) {
        self.selection = Selection {
            anchor: self.boundary(anchor),
            cursor: self.boundary(cursor),
        };
    }
    pub fn select_all(&mut self) {
        self.selection = Selection {
            anchor: 0,
            cursor: self.len,
        };
    }
    pub fn selected_text(&self) -> &str {
        let r = self.selection.ordered();
        &self.text()[r]
    }
    pub fn preedit(&self) -> &str {
        core::str::from_utf8(&self.preedit[..self.preedit_len]).expect("preedit UTF-8")
    }
    pub fn preedit_selection(&self) -> Selection {
        self.preedit_selection
    }
    pub fn set_text(&mut self, text: &str) -> Result<(), CapacityExceeded> {
        if text.len() > N {
            return Err(CapacityExceeded);
        }
        self.record();
        self.bytes[..text.len()].copy_from_slice(text.as_bytes());
        self.len = text.len();
        self.selection = Selection {
            anchor: self.len,
            cursor: self.len,
        };
        Ok(())
    }
    pub fn insert(&mut self, text: &str) -> Result<(), CapacityExceeded> {
        self.replace_selection(text, true)
    }
    pub fn paste(&mut self, text: &str) -> Result<(), CapacityExceeded> {
        self.insert(text)
    }
    pub fn cut(&mut self) -> bool {
        if self.selection.is_empty() {
            false
        } else {
            self.replace_selection("", true).is_ok()
        }
    }
    pub fn backspace(&mut self) -> bool {
        if self.selection.is_empty() && self.selection.cursor > 0 {
            let end = self.selection.cursor;
            let start = self.text()[..end]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.selection = Selection {
                anchor: start,
                cursor: end,
            };
        }
        !self.selection.is_empty() && self.replace_selection("", true).is_ok()
    }
    pub fn delete_forward(&mut self) -> bool {
        if self.selection.is_empty() && self.selection.cursor < self.len {
            let start = self.selection.cursor;
            let end = start
                + self.text()[start..]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0);
            self.selection = Selection {
                anchor: start,
                cursor: end,
            };
        }
        !self.selection.is_empty() && self.replace_selection("", true).is_ok()
    }
    pub fn move_cursor(&mut self, right: bool, extend: bool) {
        if !extend && !self.selection.is_empty() {
            let range = self.selection.ordered();
            let next = if right { range.end } else { range.start };
            self.selection = Selection {
                anchor: next,
                cursor: next,
            };
            return;
        }
        let old = self.selection.cursor;
        let next = if right {
            old + self.text()[old..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(0)
        } else {
            self.text()[..old]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0)
        };
        self.selection = Selection {
            anchor: if extend { self.selection.anchor } else { next },
            cursor: next,
        };
    }
    /// Moves to the preceding or following Unicode-whitespace-delimited word.
    pub fn move_word(&mut self, right: bool, extend: bool) {
        let text = self.text();
        let mut next = self.selection.cursor;
        if right {
            let origin = next;
            let mut seen_word = false;
            for (offset, ch) in text[origin..].char_indices() {
                if seen_word && ch.is_whitespace() {
                    next = origin + offset;
                    break;
                }
                seen_word |= !ch.is_whitespace();
                next = origin + offset + ch.len_utf8();
            }
        } else {
            let mut seen_word = false;
            for (offset, ch) in text[..next].char_indices().rev() {
                if seen_word && ch.is_whitespace() {
                    next = offset + ch.len_utf8();
                    break;
                }
                seen_word |= !ch.is_whitespace();
                next = offset;
            }
        }
        self.selection = Selection {
            anchor: if extend { self.selection.anchor } else { next },
            cursor: next,
        };
    }
    /// Moves to the start or end of the current logical line.
    pub fn move_line_edge(&mut self, end: bool, extend: bool) {
        let cursor = self.selection.cursor;
        let text = self.text();
        let next = if end {
            text[cursor..].find('\n').map_or(self.len, |i| cursor + i)
        } else {
            text[..cursor].rfind('\n').map_or(0, |i| i + 1)
        };
        self.selection = Selection {
            anchor: if extend { self.selection.anchor } else { next },
            cursor: next,
        };
    }
    /// Moves vertically by logical lines, retaining the UTF-8 byte column when possible.
    pub fn move_vertical(&mut self, down: bool, extend: bool) {
        let cursor = self.selection.cursor;
        let text = self.text();
        let line_start = text[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let column = cursor - line_start;
        let next = if down {
            match text[cursor..].find('\n') {
                Some(relative_end) => {
                    let start = cursor + relative_end + 1;
                    let end = text[start..].find('\n').map_or(self.len, |i| start + i);
                    self.boundary((start + column).min(end))
                }
                None => cursor,
            }
        } else if line_start == 0 {
            cursor
        } else {
            let previous_end = line_start - 1;
            let start = text[..previous_end].rfind('\n').map_or(0, |i| i + 1);
            self.boundary((start + column).min(previous_end))
        };
        self.selection = Selection {
            anchor: if extend { self.selection.anchor } else { next },
            cursor: next,
        };
    }
    pub fn line_count(&self) -> usize {
        self.text().bytes().filter(|byte| *byte == b'\n').count() + 1
    }
    pub fn set_preedit(
        &mut self,
        text: &str,
        selection: Option<(usize, usize)>,
    ) -> Result<(), CapacityExceeded> {
        if text.len() > N {
            return Err(CapacityExceeded);
        }
        self.preedit[..text.len()].copy_from_slice(text.as_bytes());
        self.preedit_len = text.len();
        let (a, c) = selection.unwrap_or((text.len(), text.len()));
        self.preedit_selection = Selection {
            anchor: a.min(text.len()),
            cursor: c.min(text.len()),
        };
        Ok(())
    }
    pub fn clear_preedit(&mut self) {
        self.preedit_len = 0;
        self.preedit_selection = Selection::default();
    }
    pub fn commit_preedit(&mut self) -> Result<(), CapacityExceeded> {
        let mut temp = [0u8; N];
        let len = self.preedit_len;
        temp[..len].copy_from_slice(&self.preedit[..len]);
        self.clear_preedit();
        let text = core::str::from_utf8(&temp[..len]).expect("preedit UTF-8");
        self.insert(text)
    }
    pub fn undo(&mut self) -> bool {
        if self.undo_len == 0 {
            return false;
        }
        self.push_redo();
        self.undo_len -= 1;
        self.restore(self.undo[self.undo_len]);
        true
    }
    pub fn redo(&mut self) -> bool {
        if self.redo_len == 0 {
            return false;
        }
        self.push_undo_without_clearing();
        self.redo_len -= 1;
        self.restore(self.redo[self.redo_len]);
        true
    }
    fn replace_selection(
        &mut self,
        replacement: &str,
        record: bool,
    ) -> Result<(), CapacityExceeded> {
        let r = self.selection.ordered();
        let new_len = self.len - (r.end - r.start) + replacement.len();
        if new_len > N {
            return Err(CapacityExceeded);
        }
        if record {
            self.record()
        }
        self.bytes
            .copy_within(r.end..self.len, r.start + replacement.len());
        self.bytes[r.start..r.start + replacement.len()].copy_from_slice(replacement.as_bytes());
        self.len = new_len;
        let cursor = r.start + replacement.len();
        self.selection = Selection {
            anchor: cursor,
            cursor,
        };
        Ok(())
    }
    fn boundary(&self, index: usize) -> usize {
        let mut i = index.min(self.len);
        while i > 0 && !self.text().is_char_boundary(i) {
            i -= 1
        }
        i
    }
    fn snapshot(&self) -> Snapshot<N> {
        Snapshot {
            bytes: self.bytes,
            len: self.len,
            selection: self.selection,
        }
    }
    fn restore(&mut self, s: Snapshot<N>) {
        self.bytes = s.bytes;
        self.len = s.len;
        self.selection = s.selection;
        self.clear_preedit()
    }
    fn record(&mut self) {
        self.push_undo_without_clearing();
        self.redo_len = 0
    }
    fn push_undo_without_clearing(&mut self) {
        if HISTORY == 0 {
            return;
        }
        if self.undo_len == HISTORY {
            self.undo.copy_within(1..HISTORY, 0);
            self.undo_len -= 1
        }
        self.undo[self.undo_len] = self.snapshot();
        self.undo_len += 1
    }
    fn push_redo(&mut self) {
        if HISTORY == 0 {
            return;
        }
        if self.redo_len == HISTORY {
            self.redo.copy_within(1..HISTORY, 0);
            self.redo_len -= 1
        }
        self.redo[self.redo_len] = self.snapshot();
        self.redo_len += 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_unicode_undo_redo() {
        let mut e = TextEdit::<32>::default();
        e.insert("a日本").unwrap();
        e.move_cursor(false, false);
        assert!(e.backspace());
        assert_eq!(e.text(), "a本");
        assert!(e.undo());
        assert_eq!(e.text(), "a日本");
        assert!(e.redo());
        assert_eq!(e.text(), "a本");
    }
    #[test]
    fn composition_commits_at_selection() {
        let mut e = TextEdit::<32>::default();
        e.insert("ab").unwrap();
        e.set_selection(1, 2);
        e.set_preedit("字", None).unwrap();
        e.commit_preedit().unwrap();
        assert_eq!(e.text(), "a字");
    }
    #[test]
    fn multiline_and_word_navigation_preserve_boundaries() {
        let mut e = TextEdit::<64>::default();
        e.insert("one two\n日本語\nlast").unwrap();
        e.set_selection(0, 0);
        e.move_word(true, false);
        assert_eq!(e.selection().cursor, 3);
        e.move_line_edge(true, false);
        assert_eq!(e.selection().cursor, 7);
        e.move_vertical(true, false);
        assert_eq!(&e.text()[e.selection().cursor..], "語\nlast");
        e.move_line_edge(false, false);
        assert_eq!(&e.text()[e.selection().cursor..], "日本語\nlast");
        assert_eq!(e.line_count(), 3);
    }
}

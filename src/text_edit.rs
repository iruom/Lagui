//! Unicode-aware editing sessions, with bounded and allocation-backed storage.
//! Committed application data and document undo remain owned by the host.
mod boundaries;
#[cfg(feature = "alloc")]
mod dynamic;
#[doc(hidden)]
pub mod storage;
use crate::CapacityExceeded;
use core::ops::Range;
#[cfg(feature = "alloc")]
pub use dynamic::TextEditor;
use storage::{FixedHistory, FixedText, History, TextStorage};
use unicode_segmentation::UnicodeSegmentation;

#[cfg(feature = "portable")]
pub(crate) fn floor_boundary(text: &str, index: usize) -> usize {
    boundaries::floor(text, index)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub cursor: usize,
}
impl Selection {
    pub fn ordered(self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    pub fn is_empty(self) -> bool {
        self.anchor == self.cursor
    }
}

/// Explicitly bounded, allocation-free editor. N is a byte limit and HISTORY
/// is an undo limit. For unbounded application input use `TextEditor` with alloc.
pub type TextEdit<const N: usize, const HISTORY: usize = 32> =
    Editor<FixedText<N>, FixedHistory<Edit<FixedText<N>>, HISTORY>>;

/// One reversible change. Allocating editors retain changed text only, rather
/// than copying the complete document on every keystroke.
#[doc(hidden)]
pub struct Edit<B> {
    start: usize,
    removed: B,
    inserted: B,
    before: Selection,
    after: Selection,
}

/// Common editing algorithm; storage policies do not define input behavior.
pub struct Editor<B: TextStorage, H: History<Edit<B>>> {
    text: B,
    selection: Selection,
    preedit: B,
    preedit_selection: Selection,
    undo: H,
    redo: H,
}
impl<B: TextStorage, H: History<Edit<B>>> Default for Editor<B, H> {
    fn default() -> Self {
        Self {
            text: B::default(),
            selection: Selection::default(),
            preedit: B::default(),
            preedit_selection: Selection::default(),
            undo: H::default(),
            redo: H::default(),
        }
    }
}
impl<B: TextStorage, H: History<Edit<B>>> Editor<B, H> {
    pub fn text(&self) -> &str {
        self.text.as_str()
    }
    pub fn selection(&self) -> Selection {
        self.selection
    }
    pub fn set_selection(&mut self, anchor: usize, cursor: usize) {
        self.selection = Selection {
            anchor: boundaries::floor(self.text(), anchor),
            cursor: boundaries::floor(self.text(), cursor),
        };
    }
    pub fn select_all(&mut self) {
        self.selection = Selection {
            anchor: 0,
            cursor: self.text().len(),
        };
    }
    pub fn selected_text(&self) -> &str {
        &self.text()[self.selection.ordered()]
    }
    pub fn preedit(&self) -> &str {
        self.preedit.as_str()
    }
    pub fn preedit_selection(&self) -> Selection {
        self.preedit_selection
    }
    pub fn set_text(&mut self, text: &str) -> Result<(), CapacityExceeded> {
        self.replace(0..self.text().len(), text)?;
        self.clear_preedit();
        Ok(())
    }
    pub fn insert(&mut self, text: &str) -> Result<(), CapacityExceeded> {
        self.replace(self.selection.ordered(), text)
    }
    pub fn paste(&mut self, text: &str) -> Result<(), CapacityExceeded> {
        self.insert(text)
    }
    pub fn cut(&mut self) -> bool {
        !self.selection.is_empty() && self.insert("").is_ok()
    }
    pub fn backspace(&mut self) -> bool {
        let mut range = self.selection.ordered();
        if range.is_empty() {
            range.start = boundaries::adjacent(self.text(), range.start, false);
        }
        !range.is_empty() && self.replace(range, "").is_ok()
    }
    pub fn delete_forward(&mut self) -> bool {
        let mut range = self.selection.ordered();
        if range.is_empty() {
            range.end = boundaries::adjacent(self.text(), range.end, true);
        }
        !range.is_empty() && self.replace(range, "").is_ok()
    }
    pub fn move_cursor(&mut self, right: bool, extend: bool) {
        let next = if !extend && !self.selection.is_empty() {
            let range = self.selection.ordered();
            if right { range.end } else { range.start }
        } else {
            boundaries::adjacent(self.text(), self.selection.cursor, right)
        };
        self.move_to(next, extend);
    }
    pub fn move_word(&mut self, right: bool, extend: bool) {
        self.move_to(
            boundaries::word(self.text(), self.selection.cursor, right),
            extend,
        );
    }
    pub fn move_line_edge(&mut self, end: bool, extend: bool) {
        let cursor = self.selection.cursor;
        let next = if end {
            self.text()[cursor..]
                .find('\n')
                .map_or(self.text().len(), |i| cursor + i)
        } else {
            self.text()[..cursor].rfind('\n').map_or(0, |i| i + 1)
        };
        self.move_to(next, extend);
    }
    /// Logical-line movement counts graphemes, never UTF-8 bytes. Wrapped/bidi
    /// visual caret positions belong to the text layout adapter.
    pub fn move_vertical(&mut self, down: bool, extend: bool) {
        let text = self.text();
        let cursor = self.selection.cursor;
        let start = text[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let column = text[start..cursor].graphemes(true).count();
        let target = if down {
            text[cursor..].find('\n').map(|i| {
                let start = cursor + i + 1;
                let end = boundaries::floor(
                    text,
                    text[start..].find('\n').map_or(text.len(), |i| start + i),
                );
                start..end
            })
        } else if start == 0 {
            None
        } else {
            let end = boundaries::floor(text, start - 1);
            Some(text[..end].rfind('\n').map_or(0, |i| i + 1)..end)
        };
        if let Some(range) = target {
            let next = text[range.clone()]
                .grapheme_indices(true)
                .nth(column)
                .map_or(range.end, |(i, _)| range.start + i);
            self.move_to(next, extend);
        }
    }
    pub fn line_count(&self) -> usize {
        self.text().bytes().filter(|b| *b == b'\n').count() + 1
    }
    pub fn set_preedit(
        &mut self,
        text: &str,
        selection: Option<(usize, usize)>,
    ) -> Result<(), CapacityExceeded> {
        let preedit = B::from_str(text)?;
        let (anchor, cursor) = selection.unwrap_or((text.len(), text.len()));
        self.preedit_selection = Selection {
            anchor: boundaries::char_boundary(text, anchor),
            cursor: boundaries::char_boundary(text, cursor),
        };
        self.preedit = preedit;
        Ok(())
    }
    pub fn clear_preedit(&mut self) {
        let len = self.preedit().len();
        self.preedit.replace(0..len, "");
        self.preedit_selection = Selection::default();
    }
    /// Failure preserves text, selection, composition and both history stacks.
    pub fn commit_preedit(&mut self) -> Result<(), CapacityExceeded> {
        let preedit = B::from_str(self.preedit())?;
        self.insert(preedit.as_str())?;
        self.clear_preedit();
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.pop() else {
            return false;
        };
        self.text.replace(
            edit.start..edit.start + edit.inserted.as_str().len(),
            edit.removed.as_str(),
        );
        self.selection = edit.before;
        self.redo.push(edit);
        self.clear_preedit();
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.redo.pop() else {
            return false;
        };
        self.text.replace(
            edit.start..edit.start + edit.removed.as_str().len(),
            edit.inserted.as_str(),
        );
        self.selection = edit.after;
        self.undo.push(edit);
        self.clear_preedit();
        true
    }
    fn move_to(&mut self, cursor: usize, extend: bool) {
        let cursor = boundaries::floor(self.text(), cursor);
        self.selection = Selection {
            anchor: if extend {
                self.selection.anchor
            } else {
                cursor
            },
            cursor,
        };
    }
    fn replace(&mut self, range: Range<usize>, replacement: &str) -> Result<(), CapacityExceeded> {
        let len = (self.text().len() - range.len())
            .checked_add(replacement.len())
            .ok_or(CapacityExceeded)?;
        if !self.text.accepts(len) {
            return Err(CapacityExceeded);
        }
        let cursor = range.start + replacement.len();
        if &self.text()[range.clone()] == replacement {
            self.move_to(boundaries::ceil(self.text(), cursor), false);
            return Ok(());
        }
        let removed = B::from_str(&self.text()[range.clone()])?;
        let inserted = B::from_str(replacement)?;
        let before = self.selection;
        self.text.replace(range.clone(), replacement);
        self.move_to(boundaries::ceil(self.text(), cursor), false);
        self.undo.push(Edit {
            start: range.start,
            removed,
            inserted,
            before,
            after: self.selection,
        });
        self.redo.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_treat_combining_marks_emoji_flags_and_crlf_as_graphemes() {
        for text in ["か\u{3099}", "e\u{301}", "👩‍💻", "👨‍👩‍👧‍👦", "🇯🇵", "\r\n"]
        {
            let mut editor = TextEdit::<128>::default();
            editor.insert(text).unwrap();
            editor.move_cursor(false, true);
            assert_eq!(editor.selected_text(), text);
            editor.set_selection(text.len(), text.len());
            assert!(editor.backspace());
            assert_eq!(editor.text(), "");
            assert!(editor.undo());
            assert_eq!(editor.text(), text);
            assert_eq!(editor.selection().cursor, text.len());
            editor.set_selection(0, 0);
            assert!(editor.delete_forward());
            assert_eq!(editor.text(), "");
        }
    }
    #[test]
    fn failed_composition_preserves_text_selection_preedit_and_history() {
        let mut editor = TextEdit::<4>::default();
        editor.insert("abcd").unwrap();
        editor.set_preedit("字", Some((1, usize::MAX))).unwrap();
        assert_eq!(
            editor.preedit_selection(),
            Selection {
                anchor: 0,
                cursor: 3
            }
        );
        assert_eq!(editor.commit_preedit(), Err(CapacityExceeded));
        assert_eq!(editor.text(), "abcd");
        assert_eq!(editor.selection().cursor, 4);
        assert_eq!(editor.preedit(), "字");
        assert!(editor.undo());
        assert_eq!(editor.text(), "");
        assert!(editor.redo());
        assert_eq!(editor.text(), "abcd");
    }
    #[test]
    fn no_op_does_not_consume_undo_or_clear_redo() {
        let mut editor = TextEdit::<32>::default();
        editor.insert("a").unwrap();
        editor.insert("b").unwrap();
        editor.undo();
        editor.insert("").unwrap();
        assert!(editor.redo());
        assert_eq!(editor.text(), "ab");
    }
    #[test]
    fn insertion_that_joins_a_cluster_keeps_the_caret_on_its_boundary() {
        let mut editor = TextEdit::<64>::default();
        editor.insert("👩💻").unwrap();
        editor.set_selection("👩".len(), "👩".len());
        editor.insert("\u{200d}").unwrap();
        assert_eq!(editor.text(), "👩‍💻");
        assert_eq!(editor.selection().cursor, editor.text().len());
        editor.undo();
        assert_eq!(editor.text(), "👩💻");
        assert_eq!(editor.selection().cursor, "👩".len());
    }
    #[test]
    #[cfg(feature = "alloc")]
    fn growable_editor_keeps_delta_history_beyond_old_limits() {
        let mut editor = TextEditor::from_text(alloc::string::String::from("文").repeat(10_000));
        for _ in 0..100 {
            editor.insert("字");
        }
        assert_eq!(editor.text().len(), 30_300);
        for _ in 0..100 {
            editor.undo();
        }
        assert_eq!(editor.text().len(), 30_000);
        for _ in 0..100 {
            editor.redo();
        }
        assert_eq!(editor.text().len(), 30_300);
        editor.set_preedit("👩‍💻", None);
        editor.commit_preedit();
        assert!(editor.text().ends_with("👩‍💻"));
        assert!(editor.preedit().is_empty());
    }
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
        assert_eq!(&e.text()[e.selection().cursor..], "\nlast");
        e.move_line_edge(false, false);
        assert_eq!(&e.text()[e.selection().cursor..], "日本語\nlast");
        assert_eq!(e.line_count(), 3);
    }
}

use super::{Edit, Editor, Selection};
use alloc::{string::String, vec::Vec};

/// Growable application-owned editing session. There are no byte, grapheme or
/// undo-count limits. History stores edits, not full-text snapshots. Hosts may
/// drop/recreate a session when editing ends; document undo is separate.
#[derive(Default)]
pub struct TextEditor {
    editor: Editor<String, Vec<Edit<String>>>,
}
impl TextEditor {
    pub fn from_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            editor: Editor {
                text,
                selection: Selection {
                    anchor: cursor,
                    cursor,
                },
                ..Editor::default()
            },
        }
    }
    pub fn text(&self) -> &str {
        self.editor.text()
    }
    pub fn selection(&self) -> Selection {
        self.editor.selection()
    }
    pub fn set_selection(&mut self, anchor: usize, cursor: usize) {
        self.editor.set_selection(anchor, cursor);
    }
    pub fn select_all(&mut self) {
        self.editor.select_all();
    }
    pub fn selected_text(&self) -> &str {
        self.editor.selected_text()
    }
    pub fn preedit(&self) -> &str {
        self.editor.preedit()
    }
    pub fn preedit_selection(&self) -> Selection {
        self.editor.preedit_selection()
    }
    pub fn set_text(&mut self, text: &str) {
        self.editor.set_text(text).expect("growable text");
    }
    pub fn insert(&mut self, text: &str) {
        self.editor.insert(text).expect("growable text");
    }
    pub fn paste(&mut self, text: &str) {
        self.insert(text);
    }
    pub fn cut(&mut self) -> bool {
        self.editor.cut()
    }
    pub fn backspace(&mut self) -> bool {
        self.editor.backspace()
    }
    pub fn delete_forward(&mut self) -> bool {
        self.editor.delete_forward()
    }
    pub fn move_cursor(&mut self, right: bool, extend: bool) {
        self.editor.move_cursor(right, extend);
    }
    pub fn move_word(&mut self, right: bool, extend: bool) {
        self.editor.move_word(right, extend);
    }
    pub fn move_line_edge(&mut self, end: bool, extend: bool) {
        self.editor.move_line_edge(end, extend);
    }
    pub fn move_vertical(&mut self, down: bool, extend: bool) {
        self.editor.move_vertical(down, extend);
    }
    pub fn line_count(&self) -> usize {
        self.editor.line_count()
    }
    pub fn set_preedit(&mut self, text: &str, selection: Option<(usize, usize)>) {
        self.editor
            .set_preedit(text, selection)
            .expect("growable composition");
    }
    pub fn clear_preedit(&mut self) {
        self.editor.clear_preedit();
    }
    pub fn commit_preedit(&mut self) {
        self.editor.commit_preedit().expect("growable composition");
    }
    pub fn undo(&mut self) {
        self.editor.undo();
    }
    pub fn redo(&mut self) {
        self.editor.redo();
    }
}

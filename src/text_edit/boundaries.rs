use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

pub(super) fn char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub(super) fn floor(text: &str, index: usize) -> usize {
    let index = char_boundary(text, index);
    let mut cursor = GraphemeCursor::new(index, text.len(), true);
    if cursor.is_boundary(text, 0).expect("complete text") {
        index
    } else {
        cursor
            .prev_boundary(text, 0)
            .expect("complete text")
            .unwrap_or(0)
    }
}

pub(super) fn ceil(text: &str, index: usize) -> usize {
    let index = char_boundary(text, index);
    let mut cursor = GraphemeCursor::new(index, text.len(), true);
    if cursor.is_boundary(text, 0).expect("complete text") {
        index
    } else {
        cursor
            .next_boundary(text, 0)
            .expect("complete text")
            .unwrap_or(text.len())
    }
}

pub(super) fn adjacent(text: &str, index: usize, right: bool) -> usize {
    let mut cursor = GraphemeCursor::new(index, text.len(), true);
    if right {
        cursor
            .next_boundary(text, 0)
            .expect("complete text")
            .unwrap_or(text.len())
    } else {
        cursor
            .prev_boundary(text, 0)
            .expect("complete text")
            .unwrap_or(0)
    }
}

pub(super) fn word(text: &str, index: usize, right: bool) -> usize {
    if right {
        text.unicode_word_indices()
            .find_map(|(start, word)| {
                let end = start + word.len();
                (end > index).then_some(end)
            })
            .unwrap_or(text.len())
    } else {
        text.unicode_word_indices()
            .rev()
            .find_map(|(start, _)| (start < index).then_some(start))
            .unwrap_or(0)
    }
}

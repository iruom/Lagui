use lagui::text_edit::TextEdit;

/// Pin the official data to the Unicode version implemented by the dependency.
/// Each case exercises public cursor movement in both directions, including
/// regional-indicator context, Hangul, Indic conjuncts and emoji ZWJ sequences.
#[test]
fn cursor_navigation_matches_unicode_16_grapheme_break_tests() {
    let mut cases = 0;
    for line in include_str!("assets/GraphemeBreakTest.txt").lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let mut text = String::new();
        let mut boundaries = Vec::new();
        for token in line.split_whitespace() {
            match token {
                "÷" => boundaries.push(text.len()),
                "×" => {}
                hex => text.push(char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap()),
            }
        }
        let mut editor = TextEdit::<2048, 0>::default();
        editor.insert(&text).unwrap();
        editor.set_selection(0, 0);
        for expected in boundaries.iter().skip(1) {
            editor.move_cursor(true, false);
            assert_eq!(editor.selection().cursor, *expected, "forward case: {line}");
        }
        for expected in boundaries.iter().rev().skip(1) {
            editor.move_cursor(false, false);
            assert_eq!(editor.selection().cursor, *expected, "reverse case: {line}");
        }
        cases += 1;
    }
    assert!(cases > 1000, "the full conformance fixture must run");
}

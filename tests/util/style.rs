use super::*;

#[test]
fn wrapping_loses_nothing_and_keeps_colors_whole() {
    let line = format!("\x1b[32m+\x1b[0m {}", "x".repeat(20));
    let rows = wrap(&line, 10);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| visible(row) <= 10));
    assert_eq!(strip_ansi(&rows.concat()), strip_ansi(&line));
    assert!(rows[0].starts_with("\x1b[32m+\x1b[0m"));
}

#[test]
fn a_short_line_is_one_row() {
    assert_eq!(wrap("short", 10), ["short"]);
    assert_eq!(wrap("", 10), [""]);
}

#[test]
fn cleaning_replaces_invisible_and_reordering_characters() {
    let tricks = "a\u{200b}b\u{202e}c\u{2060}d\u{feff}e\u{2028}f\u{e0041}g\u{61c}h\u{ad}i";
    assert_eq!(clean(tricks).matches('\u{fffd}').count(), 8);
    assert_eq!(clean("\x1b[31mred\r"), "\u{fffd}[31mred\u{fffd}");
    assert_eq!(clean("tab\tä汉"), "tab    ä汉");
}

#[test]
fn padding_ignores_color_codes() {
    let style = Style {
        terminal: true,
        color: true,
    };
    let painted = style.paint("ab", &[Paint::Red]);
    assert_eq!(visible(&pad(&painted, 5)), 5);
}

#[test]
fn wide_characters_count_twice() {
    assert_eq!(visible("汉字"), 4);
    assert_eq!(pad("汉字", 6), "汉字  ");
    let rows = wrap("~/汉字汉字汉字", 7);
    assert!(rows.iter().all(|row| visible(row) <= 7), "{rows:?}");
    assert_eq!(rows.concat(), "~/汉字汉字汉字");
}

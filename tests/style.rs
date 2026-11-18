use super::*;

#[test]
fn clipping_keeps_colors_and_counts_only_visible_characters() {
    let line = format!("\x1b[32m+\x1b[0m {}", "x".repeat(20));
    let clipped = clip(&line, 10);
    assert_eq!(visible(&clipped), 10);
    assert!(clipped.starts_with("\x1b[32m+\x1b[0m"));
    assert!(clipped.ends_with("…\x1b[0m"));
}

#[test]
fn a_short_line_is_left_alone() {
    assert_eq!(clip("short", 10), "short");
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
    let clipped = clip("~/汉字汉字汉字", 7);
    assert!(visible(&clipped) <= 7, "{clipped}");
}

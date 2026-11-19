use super::*;

#[test]
fn a_pref_line_reads_back_as_written_whatever_its_spacing() {
    assert_eq!(
        parse_line(&line("a.b", "\"x, y\"")),
        Some(("a.b", "\"x, y\""))
    );
    assert_eq!(parse_line("  user_pref( \"a\",3 ) ;"), Some(("a", "3")));
    assert_eq!(parse_line("// user_pref(\"a\", 3);"), None);
    assert_eq!(parse_line("user_pref(\"a\", 3)"), None);
}

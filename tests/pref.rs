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

#[test]
fn only_a_plain_literal_is_a_value() {
    for value in [
        "true",
        "false",
        "0",
        "-12",
        "\"\"",
        "\"http://a.b/c?d=1\"",
        "\"say \\\"hi\\\" \\\\ \"",
        "\"two\\nlines\\r\"",
    ] {
        assert_eq!(parse_line(&line("a", value)), Some(("a", value)), "{value}");
    }
    for value in [
        "1); user_pref(\"b\", 2",
        "\"x\"); user_pref(\"b\", \"y\"",
        "\"a\"b\"",
        "\"a\\\"",
        "\"a\\tb\"",
        "1.5",
        "-",
        "foo",
    ] {
        assert_eq!(parse_line(&line("a", value)), None, "{value}");
    }
    assert_eq!(
        parse_line("user_pref(\"a\", 1); user_pref(\"b\", \"http://evil\");"),
        None
    );
}

use super::*;

fn parse(args: &[&str]) -> Result<Flags> {
    Flags::parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
}

#[test]
fn flags_take_a_value_each_and_options_add_up() {
    let flags = parse(&[
        "--type", "server", "--option", "a", "--option", "b", "--layout", "beside",
    ])
    .unwrap();
    assert_eq!(flags.device_type.as_deref(), Some("server"));
    assert_eq!(flags.options, ["a", "b"]);
    assert_eq!(flags.layout, Some(Layout::Beside));
    assert_eq!(flags.only_on_the_iso(), Some("--layout"));
}

#[test]
fn flags_refuse_what_they_can_not_mean() {
    let error = |args: &[&str]| parse(args).err().unwrap().to_string();
    assert_eq!(error(&["--type"]), "--type needs a value");
    assert_eq!(
        error(&["--fleet", "--type", "server"]),
        "--fleet needs a value"
    );
    assert_eq!(
        error(&["--type", "a", "--type", "b"]),
        "--type is given twice"
    );
    assert_eq!(
        error(&["--type", "a", "--option", "x", "--option", "x"]),
        "--option x is given twice"
    );
    assert_eq!(error(&["--colour", "red"]), "install has no --colour");
    assert_eq!(error(&["--option", "a"]), "--option needs --type");
    assert_eq!(
        error(&["--layout", "half"]),
        "the layout is wipe or beside, not half"
    );
    assert_eq!(
        error(&["--remove-unlisted", "maybe"]),
        "--remove-unlisted is yes or no, not maybe"
    );
}

#[test]
fn a_hostname_follows_rfc_1123() {
    for name in ["box", "mini-pc", "a1", &"a".repeat(63)] {
        assert!(crate::system::host::check(name).is_ok(), "{name}");
    }
    for name in ["", "-help", "box-", "Box", "a_b", "a.b", &"a".repeat(64)] {
        assert!(crate::system::host::check(name).is_err(), "{name}");
    }
}

#[test]
fn a_time_zone_stays_inside_zoneinfo() {
    assert!(known_timezone("UTC"));
    for name in [
        "../../../etc/passwd",
        "/etc/passwd",
        "Europe/../UTC",
        "./UTC",
        "",
    ] {
        assert!(!known_timezone(name), "{name}");
    }
}

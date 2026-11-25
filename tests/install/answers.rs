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

#[test]
fn saved_answers_fill_only_what_the_flags_leave_open() {
    let saved = Saved {
        fleet: "git@github.com:me/fleet.git".into(),
        hostname: "laptop".into(),
        admin: "tag".into(),
        timezone: "Europe/Paris".into(),
        machine: Machine {
            device_type: "workstation".into(),
            options: vec!["windows".into()],
        },
    };
    let text = toml::to_string(&saved).unwrap();
    assert_eq!(toml::from_str::<Saved>(&text).unwrap(), saved);
    let flags = parse(&["--hostname", "desktop", "--type", "server"]).unwrap();
    let filled = saved.under(&flags);
    assert_eq!(filled.hostname.as_deref(), Some("desktop"));
    assert_eq!(filled.device_type.as_deref(), Some("server"));
    assert!(filled.options.is_empty());
    assert_eq!(filled.fleet.as_deref(), Some("git@github.com:me/fleet.git"));
    assert_eq!(filled.admin.as_deref(), Some("tag"));
    assert_eq!(filled.timezone.as_deref(), Some("Europe/Paris"));
}

#[test]
fn a_machine_the_fleet_lists_keeps_its_type_and_options() {
    let entry = Entry::new("server", &["b".to_owned(), "a".to_owned()]);
    let flags = |args: &[&str]| parse(args).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let error = |flags: &Flags| {
        listed(flags, &entry, dir.path(), "box")
            .err()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        error(&flags(&["--type", "desktop"])),
        "--type desktop: machines/box.toml says box is a server"
    );
    assert_eq!(
        error(&flags(&["--type", "server", "--option", "a"])),
        "--option: machines/box.toml lists other options for box"
    );
}

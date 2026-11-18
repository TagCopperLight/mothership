use super::*;
use crate::{
    fleet::{Fleet, System},
    ignore::Ignore,
};
use tempfile::TempDir;

const SYSTEM: &str = r#"# What a workstation has outside $HOME.
packages = ["base", "dev"]
built_packages = ["local"]

shell = "/usr/bin/bash"
groups = ["docker"]
services = [
  "NetworkManager.service",
  "fstrim.timer",
]

[[installers]]
name = "bun"
run = "curl -fsSL https://bun.sh/install | bash"
creates = "~/.bun"

[[installers]]
name = "uv"
run = "curl -LsSf https://astral.sh/uv/install.sh | sh"
creates = "~/.local/bin/uv"

[tools]
# nextest runs the tests
cargo = [
  "cargo-nextest",
  { name = "chromash", git = "https://example.com/chromash" },
  "sqlx-cli",
]
"#;

struct Checkout {
    dir: TempDir,
    fleet: Fleet,
}

impl Checkout {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let type_dir = dir.path().join("types/workstation");
        fs::create_dir_all(&type_dir).unwrap();
        fs::create_dir_all(dir.path().join("packages")).unwrap();
        fs::write(type_dir.join("system.toml"), SYSTEM).unwrap();
        fs::write(type_dir.join("ignore.txt"), "home ~/.cache\n").unwrap();
        fs::write(
            dir.path().join("packages/base.txt"),
            "vim\nfastfetch btop       # system info\nzip\n",
        )
        .unwrap();
        fs::write(dir.path().join("packages/dev.txt"), "rust\nuv  # python\n").unwrap();
        fs::write(dir.path().join("packages/local.txt"), "mytool\n").unwrap();
        let system: System = toml::from_str(SYSTEM).unwrap();
        let fleet = Fleet {
            dir: dir.path().to_path_buf(),
            type_dir: type_dir.clone(),
            system,
            ignore: Ignore::load(&type_dir.join("ignore.txt")).unwrap(),
        };
        Checkout { dir, fleet }
    }

    fn apply(&self, add: Add, answers: &'static str) -> bool {
        add.apply(&self.fleet, &mut Prompt::answering(answers))
            .unwrap()
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.dir.path().join(rel)).unwrap()
    }

    fn system(&self) -> String {
        self.read("types/workstation/system.toml")
    }
}

#[test]
fn lists_a_package_in_the_chosen_list() {
    let checkout = Checkout::new();
    assert!(checkout.apply(Add::ListPackage("htop".into()), "2"));
    assert_eq!(
        checkout.read("packages/dev.txt"),
        "rust\nuv  # python\nhtop\n"
    );
}

#[test]
fn listing_a_package_stops_when_no_list_is_chosen() {
    let checkout = Checkout::new();
    assert!(!checkout.apply(Add::ListPackage("htop".into()), ""));
    assert!(!checkout.apply(Add::ListPackage("htop".into()), "9"));
    assert_eq!(checkout.read("packages/dev.txt"), "rust\nuv  # python\n");
}

#[test]
fn unlisting_keeps_the_other_names_and_the_comment_in_place() {
    let checkout = Checkout::new();
    checkout.apply(Add::UnlistPackage("fastfetch".into()), "");
    assert_eq!(
        checkout.read("packages/base.txt"),
        "vim\nbtop                 # system info\nzip\n"
    );
}

#[test]
fn unlisting_the_only_name_drops_the_line_and_its_comment() {
    let checkout = Checkout::new();
    checkout.apply(Add::UnlistPackage("uv".into()), "");
    assert_eq!(checkout.read("packages/dev.txt"), "rust\n");
}

#[test]
fn unlisting_reaches_built_package_lists() {
    let checkout = Checkout::new();
    checkout.apply(Add::UnlistPackage("mytool".into()), "");
    assert_eq!(checkout.read("packages/local.txt"), "");
}

#[test]
fn unlisting_an_unknown_package_fails() {
    let checkout = Checkout::new();
    let add = Add::UnlistPackage("nope".into());
    assert!(
        add.apply(&checkout.fleet, &mut Prompt::answering(""))
            .is_err()
    );
}

#[test]
fn declares_in_the_style_of_the_list() {
    let checkout = Checkout::new();
    checkout.apply(
        Add::Declare {
            key: Key::Services,
            name: "sshd.service".into(),
        },
        "",
    );
    checkout.apply(
        Add::Declare {
            key: Key::Groups,
            name: "wheel".into(),
        },
        "",
    );
    let system = checkout.system();
    assert!(system.contains("groups = [\"docker\", \"wheel\"]\n"));
    assert!(system.contains("  \"fstrim.timer\",\n  \"sshd.service\",\n]"));
}

#[test]
fn declares_into_a_key_the_file_lacks() {
    let checkout = Checkout::new();
    checkout.apply(
        Add::Declare {
            key: Key::UserServices,
            name: "hyprwhspr.service".into(),
        },
        "",
    );
    checkout.apply(
        Add::Declare {
            key: Key::Uv,
            name: "ruff".into(),
        },
        "",
    );
    let system: System = toml::from_str(&checkout.system()).unwrap();
    assert_eq!(system.user_services, ["hyprwhspr.service"]);
    assert_eq!(system.tools.uv[0].name(), "ruff");
}

#[test]
fn undeclares_a_tool_by_name_or_by_table_and_keeps_the_comments() {
    let checkout = Checkout::new();
    for name in ["cargo-nextest", "chromash"] {
        checkout.apply(
            Add::Undeclare {
                key: Key::Cargo,
                name: name.into(),
            },
            "",
        );
    }
    let system = checkout.system();
    assert!(system.contains("# What a workstation has outside $HOME."));
    assert!(system.contains("# nextest runs the tests\ncargo = [\n  \"sqlx-cli\",\n]"));
}

#[test]
fn undeclaring_what_is_not_declared_fails() {
    let checkout = Checkout::new();
    let add = Add::Undeclare {
        key: Key::Groups,
        name: "wheel".into(),
    };
    assert!(
        add.apply(&checkout.fleet, &mut Prompt::answering(""))
            .is_err()
    );
}

#[test]
fn removes_one_installer() {
    let checkout = Checkout::new();
    checkout.apply(Add::UndeclareInstaller("bun".into()), "");
    let system: System = toml::from_str(&checkout.system()).unwrap();
    let names: Vec<&str> = system.installers.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["uv"]);
}

#[test]
fn sets_the_shell() {
    let checkout = Checkout::new();
    checkout.apply(Add::Shell("/usr/bin/fish".into()), "");
    let system: System = toml::from_str(&checkout.system()).unwrap();
    assert_eq!(system.shell.as_deref(), Some("/usr/bin/fish"));
}

#[test]
fn tracking_a_folder_stops_unless_confirmed() {
    let checkout = Checkout::new();
    let folder = checkout.dir.path().join("app");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("config"), "").unwrap();
    assert!(!checkout.apply(Add::Track(folder), "n"));
}

#[test]
fn appends_ignore_lines_the_ignore_list_then_matches() {
    let checkout = Checkout::new();
    let item = crate::scan::Item {
        kind: crate::scan::Kind::Home,
        subject: "~/.config/odd [name]*".into(),
        detail: String::new(),
        secret: false,
        add: None,
        revert: None,
        preview: crate::preview::Preview::None,
    };
    ignore(&checkout.fleet, &crate::scan::ignore_line(&item)).unwrap();
    let ignore = Ignore::load(&checkout.fleet.ignore_path()).unwrap();
    assert!(ignore.matches(&item));
    let other = crate::scan::Item {
        subject: "~/.config/odd n".into(),
        ..item
    };
    assert!(!ignore.matches(&other));
}

#[test]
fn copies_files_and_symlinks_into_etc() {
    let checkout = Checkout::new();
    let machine = checkout.dir.path().join("machine");
    fs::create_dir(&machine).unwrap();
    fs::write(machine.join("locale.conf"), "LANG=en_US.UTF-8\n").unwrap();
    unix::fs::symlink("/run/resolv.conf", machine.join("resolv.conf")).unwrap();
    let etc = checkout.fleet.type_dir.join("etc");

    let fleet = &checkout.fleet;
    copy_etc(
        fleet,
        Path::new("sub/locale.conf"),
        &machine.join("locale.conf"),
    )
    .unwrap();
    copy_etc(
        fleet,
        Path::new("resolv.conf"),
        &machine.join("resolv.conf"),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(etc.join("sub/locale.conf")).unwrap(),
        "LANG=en_US.UTF-8\n"
    );
    assert_eq!(
        fs::read_link(etc.join("resolv.conf")).unwrap(),
        Path::new("/run/resolv.conf")
    );
    assert!(copy_etc(fleet, Path::new("machine"), &machine).is_err());
    assert!(!checkout.system().contains("etc_modes"));
}

#[test]
fn copying_keeps_a_mode_git_cannot_in_etc_modes() {
    let checkout = Checkout::new();
    let machine = checkout.dir.path().join("wheel");
    fs::write(&machine, "%wheel ALL=(ALL:ALL) ALL\n").unwrap();
    fs::set_permissions(&machine, fs::Permissions::from_mode(0o640)).unwrap();
    let rel = Path::new("sudoers.d/wheel");
    copy_etc(&checkout.fleet, rel, &machine).unwrap();
    assert!(
        checkout
            .system()
            .ends_with("\n[etc_modes]\n\"sudoers.d/wheel\" = \"640\"\n")
    );
    let copied = checkout.fleet.etc_dir().join(rel);
    let mode = fs::metadata(&copied).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o644);

    fs::set_permissions(&machine, fs::Permissions::from_mode(0o644)).unwrap();
    copy_etc(&checkout.fleet, rel, &machine).unwrap();
    assert!(!checkout.system().contains("sudoers.d/wheel"));
}

#[test]
fn modes_parse_as_octal_and_reject_anything_else() {
    let parse = |text: &str| toml::from_str::<System>(&format!("etc_modes = {{ a = \"{text}\" }}"));
    assert_eq!(parse("440").unwrap().etc_modes[Path::new("a")], Mode(0o440));
    assert!(parse("abc").is_err());
    assert!(parse("99").is_err());
    assert!(parse("77777").is_err());
}

#[test]
fn removing_from_etc_drops_the_folders_left_empty() {
    let checkout = Checkout::new();
    let etc = checkout.fleet.type_dir.join("etc");
    fs::create_dir_all(etc.join("a/b")).unwrap();
    fs::write(etc.join("a/b/file"), "").unwrap();
    fs::write(etc.join("kept"), "").unwrap();
    remove_etc(&checkout.fleet, &etc.join("a/b/file")).unwrap();
    assert!(!etc.join("a").exists());
    assert!(etc.join("kept").exists());
}

#[test]
fn writes_prefs_into_user_js() {
    let checkout = Checkout::new();
    let user_js = checkout.dir.path().join("user.js");
    fs::write(&user_js, "user_pref(\"a\", 1);\nuser_pref(\"ab\", 2);\n").unwrap();
    set_pref(&user_js, "a", "3").unwrap();
    set_pref(&user_js, "c", "\"text\"").unwrap();
    assert_eq!(
        fs::read_to_string(&user_js).unwrap(),
        "user_pref(\"a\", 3);\nuser_pref(\"ab\", 2);\nuser_pref(\"c\", \"text\");\n"
    );
}

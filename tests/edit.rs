use super::*;
use crate::{
    fleet::{Fleet, System},
    hardware::Hardware,
    ignore::Ignore,
};
use std::collections::BTreeSet;
use tempfile::TempDir;

const SYSTEM: &str = r#"# What a workstation has outside $HOME.
packages = [
  # Core
  "vim",
  "btop",  # system info
  "zip",   # archives
]
built_packages = ["mytool"]

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

[hardware.battery]
packages = ["power-profiles-daemon"]
services = ["power-profiles-daemon.service"]
etc = ["battery.conf"]

[options.dual-boot]
packages = ["os-prober"]
etc = ["grub.d/dual"]
"#;

struct Checkout {
    dir: TempDir,
    fleet: Fleet,
}

impl Checkout {
    /// A machine with a battery, without dual boot.
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let type_dir = dir.path().join("types/workstation");
        fs::create_dir_all(type_dir.join("etc/grub.d")).unwrap();
        fs::write(type_dir.join("etc/battery.conf"), "").unwrap();
        fs::write(type_dir.join("etc/grub.d/dual"), "").unwrap();
        fs::write(type_dir.join("system.toml"), SYSTEM).unwrap();
        fs::write(type_dir.join("ignore.txt"), "home ~/.cache\n").unwrap();
        let hardware = BTreeSet::from([Hardware::Battery]);
        let fleet = Fleet::new(dir.path().to_path_buf(), "workstation", &[], &hardware).unwrap();
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
fn lists_a_package_last_and_leaves_the_comments_on_their_lines() {
    let checkout = Checkout::new();
    // ListPackage asks pacman which key; the e2e scenarios cover that part.
    checkout.apply(
        Add::Declare {
            key: Key::Packages,
            name: "htop".into(),
        },
        "",
    );
    assert!(
        checkout
            .system()
            .contains("  \"zip\",   # archives\n  \"htop\",\n]\nbuilt_packages")
    );
}

#[test]
fn unlisting_drops_the_line_and_its_comment_but_not_the_heading() {
    let checkout = Checkout::new();
    checkout.apply(Add::UnlistPackage("btop".into()), "");
    assert!(
        checkout
            .system()
            .contains("packages = [\n  # Core\n  \"vim\",\n  \"zip\",   # archives\n]")
    );
    checkout.apply(Add::UnlistPackage("vim".into()), "");
    assert!(
        checkout
            .system()
            .contains("packages = [\n  # Core\n  \"zip\",   # archives\n]")
    );
}

#[test]
fn unlisting_the_last_keeps_the_comment_of_the_one_before() {
    let checkout = Checkout::new();
    checkout.apply(Add::UnlistPackage("zip".into()), "");
    assert!(checkout.system().contains("  \"btop\",  # system info\n]"));
}

#[test]
fn unlisting_reaches_built_packages_and_the_machines_blocks() {
    let checkout = Checkout::new();
    checkout.apply(Add::UnlistPackage("mytool".into()), "");
    checkout.apply(Add::UnlistPackage("power-profiles-daemon".into()), "");
    checkout.apply(
        Add::Undeclare {
            key: Key::Services,
            name: "power-profiles-daemon.service".into(),
        },
        "",
    );
    let system = checkout.system();
    assert!(system.contains("built_packages = []\n"));
    assert!(system.contains("[hardware.battery]\npackages = []\nservices = []\n"));
}

#[test]
fn unlisting_an_unknown_package_or_one_of_an_option_off_here_fails() {
    let checkout = Checkout::new();
    for name in ["nope", "os-prober"] {
        let add = Add::UnlistPackage(name.into());
        assert!(
            add.apply(&checkout.fleet, &mut Prompt::answering(""))
                .is_err()
        );
    }
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
            key: Key::Tools(Manager::Uv),
            name: "ruff".into(),
        },
        "",
    );
    let system: System = toml::from_str(&checkout.system()).unwrap();
    assert_eq!(system.user_services, ["hyprwhspr.service"]);
    assert_eq!(system.tools.of(Manager::Uv)[0].name(), "ruff");
}

#[test]
fn undeclares_a_tool_by_name_or_by_table_and_keeps_the_comments() {
    let checkout = Checkout::new();
    for name in ["cargo-nextest", "chromash"] {
        checkout.apply(
            Add::Undeclare {
                key: Key::Tools(Manager::Cargo),
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
    assert!(!checkout.apply(
        Add::Track {
            path: folder,
            encrypt: false
        },
        "n"
    ));
}

#[test]
fn appends_ignore_lines_the_ignore_list_then_matches() {
    let checkout = Checkout::new();
    let item = crate::scan::Item {
        kind: crate::scan::Kind::Home,
        subject: "~/.config/odd [name]*".into(),
        detail: String::new(),
        secret: crate::scan::Secret::No,
        add: None,
        no_add: None,
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
        false,
    )
    .unwrap();
    copy_etc(
        fleet,
        Path::new("resolv.conf"),
        &machine.join("resolv.conf"),
        false,
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
    assert!(copy_etc(fleet, Path::new("machine"), &machine, false).is_err());
    assert!(!checkout.system().contains("etc_modes"));
}

#[test]
fn copying_keeps_a_mode_git_cannot_in_etc_modes() {
    let checkout = Checkout::new();
    let machine = checkout.dir.path().join("wheel");
    fs::write(&machine, "%wheel ALL=(ALL:ALL) ALL\n").unwrap();
    fs::set_permissions(&machine, fs::Permissions::from_mode(0o640)).unwrap();
    let rel = Path::new("sudoers.d/wheel");
    copy_etc(&checkout.fleet, rel, &machine, false).unwrap();
    assert!(
        checkout
            .system()
            .ends_with("\n[etc_modes]\n\"sudoers.d/wheel\" = \"640\"\n")
    );
    let copied = checkout.fleet.etc_dir().join(rel);
    let mode = fs::metadata(&copied).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o644);

    fs::set_permissions(&machine, fs::Permissions::from_mode(0o644)).unwrap();
    copy_etc(&checkout.fleet, rel, &machine, false).unwrap();
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
    remove_etc(&checkout.fleet, Path::new("a/b/file")).unwrap();
    assert!(!etc.join("a").exists());
    assert!(etc.join("kept").exists());
}

#[test]
fn removing_a_blocks_file_drops_it_from_the_block() {
    let checkout = Checkout::new();
    remove_etc(&checkout.fleet, Path::new("battery.conf")).unwrap();
    assert!(checkout.system().contains("[hardware.battery]\npackages = [\"power-profiles-daemon\"]\nservices = [\"power-profiles-daemon.service\"]\netc = []\n"));
}

#[test]
fn copying_over_the_file_of_an_option_off_here_fails() {
    let checkout = Checkout::new();
    let machine = checkout.dir.path().join("dual");
    fs::write(&machine, "mine").unwrap();
    assert!(copy_etc(&checkout.fleet, Path::new("grub.d/dual"), &machine, false).is_err());
    assert_eq!(checkout.read("types/workstation/etc/grub.d/dual"), "");
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

#[test]
fn declaring_a_tool_writes_its_source_and_reads_back_the_same() {
    let checkout = Checkout::new();
    let moved = Tool::Git(crate::fleet::GitTool {
        name: "chromash".into(),
        git: "https://github.com/me/Chromash".into(),
        branch: None,
    });
    let new = Tool::Git(crate::fleet::GitTool {
        name: "new".into(),
        git: "https://example.com/new".into(),
        branch: Some("dev".into()),
    });
    for tool in [&moved, &new] {
        checkout.apply(
            Add::DeclareTool {
                manager: Manager::Cargo,
                tool: tool.clone(),
            },
            "",
        );
    }
    let system: System = toml::from_str(&checkout.system()).unwrap();
    let cargo = system.tools.of(Manager::Cargo);
    assert_eq!(cargo.iter().filter(|t| t.name() == "chromash").count(), 1);
    assert_eq!(system.tools.find(Manager::Cargo, "chromash"), Some(&moved));
    assert_eq!(system.tools.find(Manager::Cargo, "new"), Some(&new));
}

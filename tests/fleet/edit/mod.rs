use super::*;
use crate::{
    fleet::{Fleet, Mode, Repo, System, UserConfig},
    sync::ignore::Ignore,
    sync::kind::Level,
    sync::preview::{Preview, Seen},
    system::hardware::Hardware,
};
use std::{
    fs,
    os::unix::{self, fs::PermissionsExt},
};
use tempfile::TempDir;

const SYSTEM: &str = r#"# What a workstation has outside $HOME.
packages = [
  # Core
  "vim",
  "btop",  # system info
  "zip",   # archives
]
built_packages = ["mytool"]
services = [
  "NetworkManager.service",
  "fstrim.timer",
]

[hardware.laptop]
detect = "battery"
packages = ["power-profiles-daemon"]
services = ["power-profiles-daemon.service"]
etc = ["battery.conf"]

[options.dual-boot]
packages = ["os-prober"]
etc = ["grub.d/dual"]
"#;

const USER: &str = r#"# What I have outside chezmoi.
shell = "/usr/bin/bash"
groups = ["docker"]

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

fn laptop() -> Hardware {
    Hardware {
        battery: true,
        ..Hardware::default()
    }
}

struct Checkout {
    dir: TempDir,
    fleet: Fleet,
}

impl Checkout {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        crate::support::name_recovery(dir.path());
        let type_dir = dir.path().join("types/workstation");
        fs::create_dir_all(type_dir.join("etc/grub.d")).unwrap();
        fs::write(type_dir.join("etc/battery.conf"), "").unwrap();
        fs::write(type_dir.join("etc/grub.d/dual"), "").unwrap();
        fs::write(type_dir.join(crate::fleet::SYSTEM_TOML), SYSTEM).unwrap();
        fs::write(type_dir.join("ignore.txt"), "package htop\n").unwrap();
        fs::create_dir_all(type_dir.join("users/me")).unwrap();
        fs::write(type_dir.join("users/me/user.toml"), USER).unwrap();
        let hardware = laptop();
        let repo = Repo::new(dir.path().to_path_buf(), "workstation", &[], &hardware).unwrap();
        let mut me = crate::support::current_user();
        me.home = dir.path().to_path_buf();
        let fleet = Fleet::of(repo, &me);
        Checkout { dir, fleet }
    }

    fn apply(&self, add: Add, answers: &'static str) -> bool {
        self.try_apply(add, answers).unwrap()
    }

    fn try_apply(&self, add: Add, answers: &'static str) -> Result<bool> {
        self.try_apply_seen(add, answers, &Seen::default())
    }

    fn try_apply_seen(&self, add: Add, answers: &'static str, seen: &Seen) -> Result<bool> {
        let me = &self.fleet.members[0];
        add.apply(&self.fleet, Some(me), &mut Prompt::answering(answers), seen)
    }

    fn shown_home(&self, path: &Path) -> Seen {
        let seen = Seen::default();
        let user = self.fleet.members[0].user().clone();
        let path = path.to_path_buf();
        Preview::Home { path, user }.show(&seen, false);
        seen
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.dir.path().join(rel)).unwrap()
    }

    fn system(&self) -> String {
        self.read("types/workstation/system.toml")
    }

    fn user(&self) -> String {
        self.read("types/workstation/users/me/user.toml")
    }
}

#[test]
fn lists_a_package_last_and_leaves_the_comments_on_their_lines() {
    let checkout = Checkout::new();
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
    assert!(
        system.contains("[hardware.laptop]\ndetect = \"battery\"\npackages = []\nservices = []\n")
    );
}

#[test]
fn unlisting_an_unknown_package_or_one_of_an_option_off_here_fails() {
    let checkout = Checkout::new();
    for name in ["nope", "os-prober"] {
        assert!(
            checkout
                .try_apply(Add::UnlistPackage(name.into()), "")
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
    assert!(
        checkout
            .user()
            .contains("groups = [\"docker\", \"wheel\"]\n")
    );
    assert!(
        checkout
            .system()
            .contains("  \"fstrim.timer\",\n  \"sshd.service\",\n]")
    );
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
    let user: UserConfig = toml::from_str(&checkout.user()).unwrap();
    assert_eq!(user.user_services, ["hyprwhspr.service"]);
    assert_eq!(user.tools.of(Manager::Uv)[0].name(), "ruff");
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
    let user = checkout.user();
    assert!(user.contains("# What I have outside chezmoi."));
    assert!(user.contains("# nextest runs the tests\ncargo = [\n  \"sqlx-cli\",\n]"));
}

#[test]
fn undeclaring_what_is_not_declared_fails() {
    let checkout = Checkout::new();
    let add = Add::Undeclare {
        key: Key::Groups,
        name: "wheel".into(),
    };
    assert!(checkout.try_apply(add, "").is_err());
}

#[test]
fn removes_one_installer() {
    let checkout = Checkout::new();
    checkout.apply(Add::UndeclareInstaller("bun".into()), "");
    let user: UserConfig = toml::from_str(&checkout.user()).unwrap();
    let names: Vec<&str> = user.installers.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["uv"]);
}

#[test]
fn sets_the_shell() {
    let checkout = Checkout::new();
    checkout.apply(Add::Shell("/usr/bin/fish".into()), "");
    let user: UserConfig = toml::from_str(&checkout.user()).unwrap();
    assert_eq!(user.shell.as_deref(), Some("/usr/bin/fish"));
}

#[test]
fn sets_and_drops_linger() {
    let checkout = Checkout::new();
    checkout.apply(Add::Linger(true), "");
    assert!(checkout.user().contains("linger = true\n"));
    checkout.apply(Add::Linger(false), "");
    assert!(!checkout.user().contains("linger"));
}

fn shown(path: &Path) -> Seen {
    let seen = Seen::default();
    Preview::Contents(path.to_path_buf()).show(&seen, false);
    seen
}

#[test]
fn tracking_a_folder_stops_unless_confirmed() {
    let checkout = Checkout::new();
    let folder = checkout.dir.path().join("app");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("config"), "").unwrap();
    let seen = checkout.shown_home(&folder);
    let add = Add::Track {
        path: folder,
        storage: Storage::Clear,
    };
    assert!(!checkout.try_apply_seen(add, "n", &seen).unwrap());
}

#[test]
fn a_folder_in_clear_is_refused_when_it_holds_files_only_the_owner_reads() {
    let checkout = Checkout::new();
    let folder = checkout.dir.path().join("app");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("config"), "").unwrap();
    fs::write(folder.join("token"), "").unwrap();
    fs::set_permissions(folder.join("token"), fs::Permissions::from_mode(0o600)).unwrap();
    let seen = checkout.shown_home(&folder);
    let add = Add::Track {
        path: folder,
        storage: Storage::Clear,
    };
    let error = checkout.try_apply_seen(add, "y", &seen).unwrap_err();
    assert!(format!("{error:#}").contains("token"));
    assert!(!format!("{error:#}").contains("config"));
}

#[test]
fn a_home_add_needs_what_was_shown_unchanged() {
    let checkout = Checkout::new();
    let folder = checkout.dir.path().join("app");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("config"), "one").unwrap();
    let add = || Add::Track {
        path: folder.clone(),
        storage: Storage::Clear,
    };
    let unseen = checkout.try_apply_seen(add(), "n", &Seen::default());
    assert!(format!("{:#}", unseen.unwrap_err()).contains("changed since it was shown"));
    let seen = checkout.shown_home(&folder);
    fs::write(folder.join("config"), "two").unwrap();
    let changed = checkout.try_apply_seen(add(), "n", &seen);
    assert!(format!("{:#}", changed.unwrap_err()).contains("changed since it was shown"));
}

#[test]
fn appends_ignore_lines_the_ignore_list_then_matches() {
    let checkout = Checkout::new();
    let mut item = crate::sync::item::Item::new(
        crate::sync::kind::Kind::Home,
        crate::sync::item::Change::Added,
        "~/.config/odd [name]*",
        "new",
    );
    item.member = Some(0);
    let me = &checkout.fleet.members[0];
    ignore(&checkout.fleet, Some(me), &item.ignore_line()).unwrap();
    let ignore = Ignore::load(&me.ignore_path(), Level::User).unwrap();
    assert!(ignore.matches(item.kind, &item.subject));
    let other = crate::sync::item::Item {
        subject: "~/.config/odd n".into(),
        ..item
    };
    assert!(!ignore.matches(other.kind, &other.subject));
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
    etc::copy(
        fleet,
        Path::new("sub/locale.conf"),
        &machine.join("locale.conf"),
        Storage::Clear,
        &shown(&machine.join("locale.conf")),
    )
    .unwrap();
    etc::copy(
        fleet,
        Path::new("resolv.conf"),
        &machine.join("resolv.conf"),
        Storage::Clear,
        &shown(&machine.join("resolv.conf")),
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
    assert!(
        etc::copy(
            fleet,
            Path::new("machine"),
            &machine,
            Storage::Clear,
            &shown(&machine)
        )
        .is_err()
    );
    assert!(!checkout.system().contains("etc_modes"));
}

#[test]
fn an_etc_copy_refuses_what_was_not_shown_or_changed_since() {
    let checkout = Checkout::new();
    let machine = checkout.dir.path().join("hosts");
    fs::write(&machine, "one\n").unwrap();
    let rel = Path::new("hosts");
    let copy = |seen: &Seen| etc::copy(&checkout.fleet, rel, &machine, Storage::Clear, seen);
    let error = copy(&Seen::default()).unwrap_err();
    assert!(format!("{error:#}").contains("changed since it was shown"));
    let seen = shown(&machine);
    fs::write(&machine, "two\n").unwrap();
    assert!(copy(&seen).is_err());
    assert!(!checkout.fleet.etc_dir().join(rel).exists());
    copy(&shown(&machine)).unwrap();
    assert_eq!(checkout.read("types/workstation/etc/hosts"), "two\n");
}

#[test]
fn copying_keeps_a_mode_git_cannot_in_etc_modes() {
    let checkout = Checkout::new();
    let machine = checkout.dir.path().join("wheel");
    fs::write(&machine, "%wheel ALL=(ALL:ALL) ALL\n").unwrap();
    fs::set_permissions(&machine, fs::Permissions::from_mode(0o640)).unwrap();
    let rel = Path::new("sudoers.d/wheel");
    etc::copy(
        &checkout.fleet,
        rel,
        &machine,
        Storage::Clear,
        &shown(&machine),
    )
    .unwrap();
    assert!(
        checkout
            .system()
            .ends_with("\n[etc_modes]\n\"sudoers.d/wheel\" = \"640\"\n")
    );
    let copied = checkout.fleet.etc_dir().join(rel);
    let mode = fs::metadata(&copied).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o644);

    fs::set_permissions(&machine, fs::Permissions::from_mode(0o644)).unwrap();
    etc::copy(
        &checkout.fleet,
        rel,
        &machine,
        Storage::Clear,
        &shown(&machine),
    )
    .unwrap();
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
    etc::remove(&checkout.fleet, Path::new("a/b/file")).unwrap();
    assert!(!etc.join("a").exists());
    assert!(etc.join("kept").exists());
}

#[test]
fn removing_a_blocks_file_drops_it_from_the_block() {
    let checkout = Checkout::new();
    etc::remove(&checkout.fleet, Path::new("battery.conf")).unwrap();
    assert!(checkout.system().contains("[hardware.laptop]\ndetect = \"battery\"\npackages = [\"power-profiles-daemon\"]\nservices = [\"power-profiles-daemon.service\"]\netc = []\n"));
}

#[test]
fn copying_over_the_file_of_an_option_off_here_fails() {
    let checkout = Checkout::new();
    let machine = checkout.dir.path().join("dual");
    fs::write(&machine, "mine").unwrap();
    assert!(
        etc::copy(
            &checkout.fleet,
            Path::new("grub.d/dual"),
            &machine,
            Storage::Clear,
            &shown(&machine)
        )
        .is_err()
    );
    assert_eq!(checkout.read("types/workstation/etc/grub.d/dual"), "");
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
    let user: UserConfig = toml::from_str(&checkout.user()).unwrap();
    let cargo = user.tools.of(Manager::Cargo);
    assert_eq!(cargo.iter().filter(|t| t.name() == "chromash").count(), 1);
    assert_eq!(user.tools.find(Manager::Cargo, "chromash"), Some(&moved));
    assert_eq!(user.tools.find(Manager::Cargo, "new"), Some(&new));
}

#[test]
fn listing_a_user_gives_them_an_empty_folder_in_the_type() {
    let checkout = Checkout::new();
    assert!(!checkout.apply(Add::ListUser("dev".into()), "n"));
    assert!(
        !checkout
            .dir
            .path()
            .join("types/workstation/users/dev")
            .exists()
    );
    assert!(checkout.apply(Add::ListUser("dev".into()), "y"));
    for empty in ["user.toml", "recipients.txt", "home/.keep"] {
        assert_eq!(
            checkout.read(&format!("types/workstation/users/dev/{empty}")),
            ""
        );
    }
    let repo = Repo::new(
        checkout.dir.path().to_path_buf(),
        "workstation",
        &[],
        &laptop(),
    )
    .unwrap();
    let names: Vec<&str> = repo.profiles.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["dev", "me"]);
    let error = checkout.try_apply(Add::ListUser("dev".into()), "y");
    assert!(error.unwrap_err().to_string().contains("is there already"));
}

#[test]
fn review_writes_mothership_s_recipes_workflow_into_the_fleet() {
    let checkout = Checkout::new();
    assert!(checkout.apply(Add::WriteWorkflow, ""));
    assert_eq!(
        checkout.read(crate::services::WORKFLOW_PATH),
        crate::services::WORKFLOW
    );
}

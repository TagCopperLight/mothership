# mothership

mothership keeps Linux machines in line with a git repo, the fleet repo, that describes device types such as "workstation". Each machine runs one type. [chezmoi](https://chezmoi.io) writes the files in `$HOME`; mothership handles packages, `/etc`, services, groups and the login shell.

You need Arch, `chezmoi`, `cargo`, `uv` and `dconf`, with the fleet repo as chezmoi's source.

## Commands

- `mothership scan` lists every difference between the machine and its type.
- `mothership review` asks about each difference: **add** it to the fleet, **ignore** it for good, put it off a week (**later**), or skip it this time (**pass**). It edits the fleet checkout and never commits. Under sudo it can also **revert** the machine to the fleet.
- `sudo mothership apply` makes the machine follow the fleet. It shows its plan and asks first.

Paths that look like secrets get a `[secret]` mark, and review won't add them. Review adds a tool only when it comes from the registry or from git, with at most a branch: an editable install, a local folder or a tag waits until the fleet can describe it.

## The fleet repo

```text
packages/<list>.txt        package names, # starts a comment
types/<type>/system.toml   what the type has outside $HOME
types/<type>/etc/          files for /etc
types/<type>/ignore.txt    what the scan never reports
```

The type is the `profile` value in the machine's chezmoi data.

```toml
packages = ["base", "desktop"]        # lists in packages/
built_packages = ["local"]            # built elsewhere: checked, never installed
aur_helper = "yay"                    # installs what pacman's repos lack
shell = "/usr/bin/fish"
groups = ["docker"]
services = ["NetworkManager.service"]
user_services = []

[[installers]]                        # run as you when `creates` is missing
name = "bun"
run = "curl -fsSL https://bun.sh/install | bash"
creates = "~/.bun"

[etc_modes]                           # default 644, or 755 if executable
"sudoers.d/wheel" = "440"

[tools]                               # cargo, uv and bun, installed as you
cargo = ["cargo-nextest", { name = "mytool", git = "https://github.com/me/mytool", branch = "dev" }]
```

`ignore.txt` holds `<kind> <glob>` lines, such as `home ~/.cache` or `package linux-firmware-*`. `*` stays in one folder, `**` crosses folders.

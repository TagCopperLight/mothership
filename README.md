# mothership

mothership keeps a set of Linux machines in line with one git repo that describes them. This repo is called the fleet repo below. The fleet repo describes device types, such as "workstation" or "server", rather than physical machines. Each machine runs one type.

It builds on [chezmoi](https://chezmoi.io), which already writes files in `$HOME`. mothership covers the rest of the machine: packages, tools installed outside the package manager, `/etc`, services, groups and login shell.

## `mothership scan`

The scan compares the machine with its device type and prints every difference, grouped by kind:

```text
Packages (2)
  htop                     installed, not listed
  ripgrep                  listed, not installed

Untracked config in ~ (1)
  ~/.config/newapp         not in the fleet

/etc (1)
  /etc/pacman.conf         changed from the package's version

System services (1)
  sshd.service             enabled, not declared

5 differences, 31 ignored by ~/.local/share/chezmoi/types/workstation/ignore.txt
```

It checks:

- **Packages**: explicitly installed packages missing from the lists, and listed packages that aren't installed.
- **Tools**: `cargo install`, `uv tool` and `bun` globals, plus tools from install scripts.
- **chezmoi files**: tracked files edited on the machine, or changed in the repo and not applied yet.
- **Untracked config in `~`**: any hidden file or folder nothing accounts for. A new app's folder shows up as one line.
- **`/etc`**: files the device type declares, package config files edited since install, and files no package owns.
- **Services**: system and user units enabled by hand. Units a package enables by default don't count.
- **Account**: login shell and groups.
- **Settings stored as data**: dconf sections, and browser prefs that differ from a `user.js` chezmoi manages.

A path that looks like a secret (SSH keys, names with `key` or `token` in them, files holding a private key) gets a `[secret]` mark, so it doesn't end up in the repo in clear.

### Running it

You need Arch, `chezmoi`, `cargo`, `uv` and `dconf` on the machine, and the fleet repo set up as chezmoi's source.

```sh
cargo run -- scan
```

It runs as your user. Parts of `/etc` that only root can read show up as "unreadable without root".

## The fleet repo

mothership reads the fleet repo from chezmoi's source directory (`chezmoi source-path`). The machine's device type is the `profile` value in its chezmoi data, which `.chezmoi.toml.tmpl` can ask for once with `promptChoiceOnce`.

```text
packages/<list>.txt        package names, one or more per line, # starts a comment
types/<type>/system.toml   what the type has outside $HOME
types/<type>/etc/          files for /etc, laid out as in /etc
types/<type>/ignore.txt    what the scan never reports for this type
```

`system.toml`:

```toml
packages = ["base", "desktop"]        # lists in packages/
built_packages = ["local"]            # packages built elsewhere: checked, not installed
shell = "/usr/bin/fish"
groups = ["docker"]
services = ["NetworkManager.service"]
user_services = []

[[installers]]                        # tools installed by a script
name = "bun"
run = "curl -fsSL https://bun.sh/install | bash"
creates = "~/.bun"                    # the scan checks this path exists

[tools]
cargo = ["cargo-nextest", { name = "mytool", git = "https://github.com/me/mytool" }]
uv = []
bun = []
```

`ignore.txt` holds one `<kind> <glob>` per line, matched against what the scan prints. `*` stays inside one folder and `**` crosses folders. The kinds are `package`, `tool`, `file`, `home`, `etc`, `service`, `user-service`, `account`, `dconf` and `pref`.

```text
home ~/.cache
home ~/.*_history
package linux-firmware-*
```

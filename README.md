# mothership

mothership sets up machines from a git repository, and tells you when a machine drifts away from it. That repository is called the fleet.

The fleet doesn't describe each machine. It describes device types, such as "workstation", and every machine belongs to one type. Machines of the same type end up identical, even when their hardware differs.

[chezmoi](https://chezmoi.io) writes the files in your home folder. mothership takes care of everything else:

- the packages, with pacman and an AUR helper on Arch, or with apt on Debian and Ubuntu;
- the files in `/etc`;
- system and user services;
- each user's account: groups, login shell, and whether their services run without a login;
- the tools each user installs with cargo, uv and bun.

A type can have several users. Each gets their own part of the type, their own secrets and their own ignore list, and mothership creates a user the machine lacks. Every command runs with `sudo`, since it reads every user's home.

## The three commands

`sudo mothership apply` brings in the changes made to the fleet since the last apply: it installs or removes packages and tools, writes the files and turns services on or off. It shows you the plan first and waits for your approval. Apply never undoes a change you made on the machine itself. It lists the files you edited, leaves them alone, and leaves your other changes to review.

`sudo mothership scan` lists everything that differs between the machine and its type, in three parts:

- **Changed on this machine**: what you did by hand, such as a package you installed, a file you edited in `/etc`, a new folder in `~/.config` or a service you enabled. Review sorts these.
- **Coming from the fleet**: what changed in the fleet since the last apply. Apply brings these.
- **Needs attention**: a secret the fleet holds in clear, or a file scan can't read.

Each line starts with what happened to the machine: `+` it gained something, `~` something was edited, `-` something is gone. On the fleet's side, the sign shows what apply will do. Scan also says when the fleet has changes nobody committed or pushed yet.

`sudo mothership review` goes through the changes made on this machine one at a time and asks what to do with each. In a terminal, one key answers, with no Enter:

| Answer | What happens |
|---|---|
| add | The difference goes into the type, so every machine of the type gets it. |
| ignore | Scan stops reporting it. |
| later | Review hides it for a week. |
| pass | Review skips it this time only. |
| revert | The machine goes back to what the type says. |

Review changes the machine's copy of the fleet but doesn't commit anything. You look at the changes, then commit and push them yourself.

## Secrets

The fleet can hold secrets, such as SSH keys and Wi-Fi passwords, because it encrypts them with [age](https://age-encryption.org).

A machine has a key for root and one for each user. Each type keeps a list of the keys allowed to open the secrets in `/etc`, and each of its users a list for the secrets in their home. A key missing from a list leaves those secrets alone.

mothership treats a file as a secret when only its owner can read it. Review then shows the file's path but never its content, and adds it encrypted. When it guesses wrong, you can answer **secret** or **not secret**.

| Command | What it does |
|---|---|
| `sudo mothership key` | Creates root's key and each user's when they have none, and prints their public halves. |
| `sudo mothership trust <type>/<root\|user> <name> <key>` | Adds a key to a list, then encrypts that list's secrets again so the key can open them. It works on any type of the fleet. |
| `sudo mothership retire <type>/<root\|user> <name>` | Removes a key from a list. It also prints the secrets that key could open, because git history still holds their old versions, so you should change them. |

## Setting up a machine

mothership runs on Arch Linux, Debian and Ubuntu. It needs `chezmoi`, plus `cargo`, `uv` and `dconf` when a user's part uses them. Build it from this repository with `cargo build --release`, on Debian for a Debian machine, since a binary built on Arch needs a newer C library.

On Debian and Ubuntu, the installer leaves the whole base system marked as installed by hand, so the first scan would list hundreds of packages. Mark them all as dependencies once with `apt-mark auto`, then mark the type's packages and built packages with `apt-mark manual`, and remove the rest with `apt-get autoremove --purge`.

The fleet lives in `/var/lib/mothership/fleet`. Whoever owns that folder is the machine's admin: they commit and push from it, and every file mothership writes there goes to them. The file `/etc/mothership/machine.toml` tells mothership the machine's type:

```toml
type = "workstation"
options = ["dual-boot"]
```

Each user's chezmoi must read the fleet and encrypt with age, using the user's key and list. The fleet's `.chezmoi.toml.tmpl` writes this config, and apply runs `chezmoi init` for a user whose config doesn't match:

```toml
sourceDir = "/var/lib/mothership/fleet"
encryption = "age"

[age]
useBuiltin = true
identity = "~/.config/mothership/key.txt"
recipientsFile = "/var/lib/mothership/fleet/types/workstation/users/tag/recipients.txt"

[data]
user = "tag"
```

## Writing a fleet

Each type has a folder in the fleet:

```text
types/workstation/
├── system.toml           packages and services
├── etc/                  files copied into /etc
├── recipients.txt        the keys allowed to open the secrets in /etc
├── ignore.txt            differences outside the homes scan should never report
├── not-secret.txt        files in /etc that look secret but aren't
└── users/tag/
    ├── user.toml         groups, shell, linger, user services and tools
    ├── secrets/          encrypted values for chezmoi templates
    ├── recipients.txt    the keys allowed to open tag's secrets
    ├── ignore.txt        differences in tag's home scan should never report
    └── not-secret.txt    files in tag's home that look secret but aren't
```

### system.toml and user.toml

```toml
packages = ["base", "linux", "git"]
aur_helper = "yay"
services = ["NetworkManager.service"]
```

Package names are the ones the machine's package manager uses. `aur_helper` only matters on Arch. A package no repository offers, such as one you built yourself, goes in `built_packages`: scan stops reporting it, and apply never installs it.

Apply writes the package manager's config (`/etc/apt` on Debian, `pacman.conf` and `pacman.d` on Arch) before it installs anything, so a repository the fleet adds is ready for the packages that come from it.

```toml
shell = "/usr/bin/fish"
groups = ["docker"]
linger = true
user_services = []

[tools]
cargo = ["cargo-nextest"]
```

`linger` keeps the user's services running while nobody is logged in.

Files in `etc/` get the mode 644, or 755 when they're executable. You can set another mode for a file in an `[etc_modes]` table.

### Hardware and options

mothership detects the CPU and GPU vendor, a battery, and a few devices. A `[hardware.*]` block adds what a piece of hardware needs:

```toml
[hardware.cpu-intel]
packages = ["intel-ucode"]
```

A machine can turn on options in its `machine.toml`, and an `[options.*]` block says what each option adds:

```toml
[options.dual-boot]
packages = ["os-prober"]
etc = ["default/grub"]
```

When a machine has hardware that its type doesn't mention, mothership stops and names the missing block. An empty block is enough to say the hardware needs nothing.

### Ignoring things

Each line of an `ignore.txt` names a kind of difference and a pattern. The type's list takes `package`, `etc` and `service`, and each user's list the rest:

```text
home ~/.cache
package linux-firmware-*
```

Each line of `not-secret.txt` is a pattern, such as `~/.config/zed/*`. In both files, `*` matches inside one folder and `**` matches across folders.

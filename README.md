# mothership

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/wordmark-dark.svg">
    <img src="assets/wordmark.svg" alt="mothership" width="360">
  </picture>
</p>

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

mothership runs on Arch Linux, Debian and Ubuntu, on x86_64. Each release publishes a static binary and its checksum, so a new machine downloads it instead of building it:

```sh
base=https://github.com/TagCopperLight/mothership/releases/latest/download
curl -fLO $base/mothership-x86_64 -fLO $base/SHA256SUMS
sha256sum -c SHA256SUMS && install -m 755 mothership-x86_64 mothership
```

`sudo ./mothership install` then asks for the fleet's repository, the machine's type and options, and its hostname. For a private fleet on GitHub, it logs in with `gh`, whose login can read every repository you can, and saves that login for the admin. The admin is the user who ran `sudo`, or the one `--admin` names. Every answer also comes as a flag, for cloud-init:

```sh
./mothership install --fleet https://github.com/me/fleet --type server --hostname box --admin ubuntu
```

Install creates the users the type lists, then the machine's keys. A list with no key yet takes this machine's. For each list that already holds keys, it prints the `trust` command to run on the machines named in it. You run them, commit and push, and answer **d**: install pulls the fleet. You can also type the recovery key instead, the key named `recovery` in the lists, and install trusts the new keys itself. That step always waits for an answer, even when flags gave every other one. A machine reinstalled under the same name replaces its old key. Then install applies the fleet. When a step fails while the network is down, install waits for it and runs the step again.

On Debian and Ubuntu, the installer leaves the whole base system marked as installed by hand. After the apply, install marks every package the type doesn't list as a dependency, and offers to remove the ones nothing needs. So the type lists what the machine can't go without: the kernel, `openssh-server`, git and chezmoi.

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

### From the Arch ISO

On a workstation, start the Arch ISO in UEFI mode, with Secure Boot in Setup Mode (its keys cleared in the firmware setup) and a TPM 2.0. Install checks all three before it touches the disk. New Secure Boot keys make Windows' BitLocker ask for its recovery key, so suspend BitLocker first, or have that key at hand. Then it asks for the time zone, the disk, and whether to erase the whole disk (`wipe`) or use its largest free space next to Windows (`beside`). It changes the disk only once you type the disk's name.

The root partition is encrypted with LUKS, and systemd finds it by its partition type, so no file names it. With `wipe`, the boot partition holds systemd-boot and the kernel. With `beside`, systemd-boot goes in Windows' EFI partition, and the kernel in a new boot partition.

Install then runs inside the new system: it asks which of the type's users is the admin and their password, applies the fleet, builds the kernel image, and signs it and systemd-boot with new Secure Boot keys, which keep Microsoft's for Windows. The type lists `linux` and `sbctl`, and holds `/etc/kernel/cmdline`. Its mkinitcpio preset builds that image as a UKI in `/boot/EFI/Linux`, with the `systemd` and `sd-encrypt` hooks.

At the next start, you type the disk's passphrase. Install finishes on the console: it checks Secure Boot is on, has the TPM open the disk from then on, and applies what needs a running system, such as user services. The TPM opens the disk only while Secure Boot runs with this machine's keys, and before any other root partition was opened, so a partition someone adds can't ask for the key.

When install stops on the ISO after it opened the disk, running it again goes on from there, and pulls what the fleet gained since.

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

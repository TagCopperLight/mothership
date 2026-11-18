# mothership

mothership keeps Linux machines in line with a git repo, the fleet repo, that describes device types such as "workstation". Each machine runs one type. [chezmoi](https://chezmoi.io) writes the files in `$HOME`; mothership handles packages, `/etc`, services, groups and the login shell.

You need Arch, `chezmoi`, `cargo`, `uv` and `dconf`, with the fleet repo as chezmoi's source. chezmoi's config encrypts with age, for the recipients of the machine's type:

```toml
encryption = "age"
[age]
useBuiltin = true
identity = "~/.config/mothership/key.txt"
recipientsFile = "~/.local/share/chezmoi/types/workstation/recipients.txt"
```

## Commands

- `mothership scan` lists every difference between the machine and its type.
- `mothership review` asks about each difference: **add** it to the fleet, **ignore** it for good, put it off a week (**later**), or skip it this time (**pass**). It edits the fleet checkout and never commits. Under sudo it can also **revert** the machine to the fleet.
- `sudo mothership apply` makes the machine follow the fleet. It shows its plan and asks first.
- `mothership key` makes the machine's key when it has none, and shows its public half.
- `mothership trust <name> <key>` adds a machine's key to the type's recipients and encrypts every secret of the type again. `mothership retire <name>` removes one, and lists the secrets to change, since git history keeps what it could open.

A file only its owner can read, or a folder holding one, gets a `[secret]` mark: review never shows it and adds it encrypted. For a secret the mark misses, review's **secret** answer adds the file encrypted, and for a file the mark gets wrong, **not secret** writes it into `not-secret.txt`. A secret the fleet holds in clear shows up too. A machine whose key isn't among the recipients leaves the secrets alone.

Review offers to declare a service, group or shell only when the fleet's packages, with their dependencies, or its files provide it, and puts a package found in neither pacman's repos nor the AUR into `built_packages`. It adds a tool only when it comes from the registry or from git, with at most a branch: an editable install, a local folder or a tag waits until the fleet can describe it.

## The fleet repo

```text
types/<type>/system.toml      what the type has outside $HOME
types/<type>/etc/             files for /etc, secrets encrypted as <file>.age
types/<type>/recipients.txt   the age keys that open the type's secrets, each under a `# name`
types/<type>/ignore.txt       what the scan never reports
types/<type>/not-secret.txt   paths that look secret but aren't, one glob per line
```

The machine names its type and options in `/etc/mothership/machine.toml`:

```toml
type = "workstation"
options = ["dual-boot"]
```

The type's `system.toml`:

```toml
packages = ["base", "linux"]
built_packages = ["mydriver"]         # built elsewhere: checked, never installed
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

[hardware.cpu-intel]                  # on machines with an Intel CPU
packages = ["intel-ucode"]

[options.dual-boot]                   # on machines whose machine.toml turns it on
packages = ["os-prober"]
etc = ["default/grub"]                # files of etc/ only these machines get
```

A block takes `packages`, `built_packages`, `services` and `etc`. mothership finds `cpu-intel`, `cpu-amd`, `gpu-intel`, `gpu-amd`, `gpu-nvidia`, `battery` and `gxfp5130` (a fingerprint sensor), and stops when the machine has one its type has no block for, even an empty one.

`ignore.txt` holds `<kind> <glob>` lines, such as `home ~/.cache` or `package linux-firmware-*`, and `not-secret.txt` holds globs such as `~/.config/zed/*`. `*` stays in one folder, `**` crosses folders.

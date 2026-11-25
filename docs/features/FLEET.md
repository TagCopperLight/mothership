# Fleet

The fleet is the git repo that describes every machine: packages, files, users, system and user services, containers and secrets. Part of [MOTHERSHIP](../MOTHERSHIP.md).

## Requirements

1. The fleet describes kinds of machine, not single machines, so two machines of one kind can't drift apart.
2. A machine finds its own hardware: a swapped part gets its driver without a fleet edit, when its type knows that part.
3. A machine has several users, each with their own home, groups, user services and tools. A file several homes use exists once.
4. Everything installed outside the package manager is in the fleet, when the fleet can rebuild it.
5. Removing a user from a type never deletes their user or their files on the machines: scan reports the untracked user instead.
6. The fleet lists every machine, so a reinstalled machine gets back its type and options without anyone typing them.

## Design

```
fleet/
├── mothership.toml        settings for the whole fleet
├── machines/<host>.toml   each machine: its type, options and public keys (below)
├── shared/                files several homes link
│   └── .secrets/          secrets several homes or types link (SECRETS)
└── types/<type>/
    ├── system.toml        packages, system services, options
    ├── etc/               files for /etc, encrypted ones as .age
    ├── ignore.txt         differences scan stays quiet about, outside homes
    ├── services/<name>/   containers (SERVICES)
    └── users/<name>/      one folder per user (below)
```

Each user of a type has a folder:

```
types/<type>/users/<name>/
├── home/                  the home, as chezmoi sources
│   └── .secrets/          the user's encrypted secrets, read by templates
├── user.toml              groups, shell, user services, tools, ...
└── ignore.txt             differences scan stays quiet about, in this home
```

### Types and options

A type holds everything its machines share, down to the tools users install outside the package manager. Each machine's file in `machines/` names its type and options, and the machine itself stores only its name, in `/etc/mothership/machine.toml`. While a new machine waits for `trust`, its `machine.toml` also holds the type and options install chose, and `trust` writes them into its file. An option is a choice that differs between machines of one type, such as adding Windows to the boot menu.

### Machines

Each machine has one file, `machines/<host>.toml`, holding everything the fleet knows about it:

```toml
type = "db-server"
options = []
signs = ["types/db-server"]       # the folders its key may sign (TRUST), or "everything"

[keys]
signing = "ssh-ed25519 AAAA…"
age = "age1…"                     # root's
probe = "ssh-ed25519 AAAA…"       # the probe's, on the machine that runs it (PROBE)

[users.alice]
age = "age1…"
ssh = "ssh-ed25519 AAAA…"
```

Mothership builds every list from these files: the keys allowed to sign, who opens each secret ([SECRETS](SECRETS.md)), and the SSH keys `ssh_from` gives a home ([TRUST](TRUST.md)). A machine's name lives in this one file, so `trust` writes it and `retire` deletes it. A signer may change a machine's file when it may sign that machine's type, and signs at least what the file lets the machine sign.

### Users

A type lists its users. Mothership creates a listed user the machine lacks, and scan reports user accounts the type doesn't list, leaving out the system's own, such as `postgres`.

On each machine, one user owns the machine's clone of the fleet, `/var/lib/mothership/fleet`, commits from it and has sudo. Other users can live without sudo, such as an account for AI agents.

Apply writes each user's chezmoi config, pointing at their home in root's checked copy of the fleet, and runs chezmoi as that user: the user reads the fleet but can't change it. Review runs chezmoi as root, on a copy of the user's state, to bring a file back into the fleet. A home gets a file from `shared/` by linking it, one link per file. Review asks before changing a shared file, since every home linking it gets the change.

### Hardware

Mothership detects the hardware on every scan and apply. Each hardware block of a type says what to look for, a CPU vendor, a GPU vendor, an ACPI id or a battery, and what it brings: packages, system services and `/etc` files. Examples: the microcode matching the CPU's vendor, the GPU's driver, power profiles when a battery exists, or a fingerprint reader's driver when its ACPI id shows up. Hardware that no block names brings nothing.

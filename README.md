<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/wordmark-dark.svg">
    <img src="assets/wordmark.svg" alt="mothership" width="360">
  </picture>
</p>

mothership sets up machines from a git repository, the fleet, and tells you when a machine drifts away from it. The fleet describes device types, such as "workstation", and every machine of a type ends up identical. It runs on Arch, Debian and Ubuntu.

[chezmoi](https://chezmoi.io) writes the files in each user's home. mothership handles the rest: packages, `/etc`, services, user accounts, and tools installed with cargo, uv and bun.

## Commands

All of them run with `sudo`.

| Command | What it does |
|---|---|
| `apply` | Brings the fleet's changes since the last apply, after showing the plan. It never undoes a change made on the machine. |
| `scan` | Lists what differs: what changed on the machine, and what's coming from the fleet. |
| `review` | Goes through the machine's changes one by one: **add** it to the fleet, **revert** it, **ignore** it, or decide **later**. Nothing changes until its last question: it then commits and pushes the answers, signed, and reverts. |
| `commit <message>` | Signs and pushes your own edits in the fleet's folder. |
| `key` | Creates the machine's keys and prints them. |
| `recovery` | Gives a new fleet its recovery key, which opens every secret, or replaces it and encrypts every secret again. |
| `trust <host> <type> <name>=<key>...` | Lets a new machine's keys open their lists' secrets and sign for the fleet, and prints the code to type on it. |
| `retire <host>` | Removes a machine's keys, and prints the secrets you should change. |
| `secret <service> <name>` | Reads a value and encrypts it into the service's secrets, for the type's root list and the recovery key. `<service>` may be `<type>/<service>`. |
| `install` | Sets up a new machine from the fleet. |

A machine applies only commits signed by a key `signers.txt` lists, or by the recovery key, and only what is committed. Every command that changes the fleet shows the commit and asks before it signs and pushes.

Secrets are encrypted with [age](https://age-encryption.org). Root and each user have their own key, and their own list of keys allowed to open their secrets. Every secret also opens with the fleet's recovery key, which you keep on paper. Review treats a file other users can't read as a secret, and adds it encrypted.

## Setting up a machine

Download the latest release and check it:

```sh
base=https://github.com/TagCopperLight/mothership/releases/latest/download
curl -fLO $base/mothership-x86_64 -fLO $base/SHA256SUMS
sha256sum -c SHA256SUMS && install -m 755 mothership-x86_64 mothership
```

`sudo ./mothership install` asks for the fleet's SSH URL, the type, its options and the hostname. Each answer can also be a flag (`--fleet`, `--type`, `--option`, `--hostname`, `--admin`, and `--remove-unlisted yes|no` on Debian); with `--type`, only the options given with `--option` turn on. Passwords and keys are always typed. Install creates the admin's SSH key and waits until GitHub lets it in, then prints the `trust` command to run on a machine that signs for the fleet and waits for the code it prints, or takes the recovery key, and applies the fleet.

On Debian and Ubuntu, install offers to remove every package the type doesn't list, so the type lists what the machine needs: the kernel, `openssh-server`, `sudo`, git and chezmoi.

From the Arch ISO, install also partitions and encrypts the disk:

- Start the ISO in UEFI mode, with Secure Boot in Setup Mode and a TPM 2.0.
- New Secure Boot keys make BitLocker ask for its recovery key: suspend it first.
- The type lists `linux`, `sbctl` and `tpm2-tss`, holds `/etc/kernel/cmdline`, and has a mkinitcpio preset that builds a UKI with the `systemd` and `sd-encrypt` hooks, plus `sd-vconsole` when `vconsole.conf` sets a keymap. Install checks this before it touches the disk.
- At the next start, type the disk's passphrase once. After that, the TPM opens the disk.

## Writing a fleet

```text
mothership.toml           the recovery key, and the mothership release every machine runs
signers.txt               the keys whose commits machines apply, each with the folders it may change
shared/                   files and secrets several homes get, linked from each home or root's secrets
types/workstation/
├── system.toml           packages, services, hardware and options
├── etc/                  files copied into /etc
├── recipients.txt        keys that open the secrets in /etc
├── ignore.txt            differences scan never reports
├── services/gripsou/     a service: its containers, files and secrets
└── users/tag/
    ├── home/             tag's home on a workstation, as chezmoi source files
    │   └── .secrets/     secrets templates read, never written to the home
    ├── user.toml         groups, shell, linger, user services, tools and ssh_from
    ├── recipients.txt    keys that open the secrets of tag's home
    ├── ssh-keys.txt      tag's SSH public key on each workstation
    └── ignore.txt
```

A home links each shared file it wants, one link per file, and only the homes linking a file get it: scan lists who gets each one. An encrypted file in a type's `etc/` or `services/` can link a shared secret too, which root then opens. A template reads a secret with `{{ include ".secrets/api-key.age" | decrypt }}`, and can read `.profile` (the type) and `.user`. With `ssh_from = ["workstation", "server/dev"]` in `user.toml`, `.authorized_keys` lists the SSH keys of every workstation user and of dev on every server, for an `authorized_keys.tmpl` to write.

```toml
packages = ["base", "linux", "git"]
services = ["NetworkManager.service"]

[hardware.cpu-intel]
packages = ["intel-ucode"]

[options.encrypted]
packages = ["sbctl"]
etc = ["kernel/cmdline"]
```

mothership detects the hardware, and stops when the type has no block for a piece the machine has. A machine turns options on in `/etc/mothership/machine.toml`.

Each line of `ignore.txt` is a kind and a pattern, such as `home ~/.cache` or `package linux-firmware-*`.

## Services

On a server, the account `services` runs each service with its own rootless Docker, and keeps its data in `/srv/<name>/`. A service folder holds a `compose.yaml` without `image:` lines, the files it mounts, `*.tmpl` templates that read `{{ secret "name" }}`, and `secrets/*.age`, encrypted for the type's `recipients.txt`, which containers read in `/run/secrets/`. Private images on ghcr.io need `services/ghcr-token.age`, a classic token with only `read:packages`, from which mothership writes Docker's login file. `service.toml` pins each container's image:

```toml
[images.app]
image = "ghcr.io/me/app"
version = "1.4.2"
digest = "sha256:…"
repo = "me/app"
```

A small image of your own, such as Caddy with a plugin, can be a recipe: a `Dockerfile` in the service's folder that names its version with `LABEL org.opencontainers.image.version="2.8.4"`, and an image with `recipe = true` and no digest yet. Review then writes the workflow that builds recipes into the fleet, and the probe pins each build made from a commit the machines accepted.

When an update fails its health check, the machine restores the data it copied before the update and starts the versions and files that ran then. A service with bulky data that updates never change names the part to copy, such as `snapshot = ["db"]`. `[probe] host = "minipc"` in `mothership.toml` makes that machine follow new patches and minors of every image, waiting 2 days for public ones, and push them as commits that only change versions and digests. `trust` gives the probe's key that right. A machine runs a pin the probe set only once the registry agrees with it, and install asks you when the publisher has moved that tag since.

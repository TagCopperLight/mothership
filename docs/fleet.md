# Writing a fleet

```text
mothership.toml           the recovery key, which install adds, and the mothership release every machine runs
machines/desk.toml        a machine's type, options, the folders it may sign, and its public keys
shared/                   files and secrets several homes get, linked from each home or root's secrets
types/workstation/
├── system.toml           packages, services, hardware and options
├── etc/                  files copied into /etc
├── ignore.txt            differences scan never reports
├── services/app/         a service: its containers, files and secrets
└── users/alice/
    ├── home/             alice's home on a workstation, as chezmoi source files
    │   └── .secrets/     secrets templates read, never written to the home
    ├── user.toml         groups, shell, linger, user services, tools and ssh_from
    └── ignore.txt
```

A home links each shared file it wants, one link per file, and only the homes linking a file get it. Scan reports a shared file no home links. An encrypted file in a type's `etc/` or `services/` can link a shared secret too, which root then opens. A template reads a secret with `{{ include ".secrets/api-key.age" | decrypt }}`, and then needs `private_` in its name. It can read `.profile` (the type) and `.user`. With `ssh_from = ["workstation", "server/dev"]` in `user.toml`, `.authorized_keys` lists the SSH keys of every workstation user and of dev on every server, for an `authorized_keys.tmpl` to write.

```toml
packages = ["base", "linux", "git"]
services = ["NetworkManager.service"]

[hardware.intel]
detect = { cpu = "GenuineIntel" }
packages = ["intel-ucode"]

[options.encrypted]
packages = ["sbctl"]
etc = ["kernel/cmdline"]
```

A hardware block turns on when the machine has what its `detect` names: a CPU vendor (`cpu`), a GPU's PCI vendor id (`gpu`), an ACPI id (`acpi`) or `"battery"`. A machine turns options on in its file:

```toml
type = "workstation"
options = ["encrypted"]
signs = "everything"      # or folders, such as ["types/workstation"]

[keys]
signing = "ssh-ed25519 AAAA…"
age = "age1…"             # root's

[users.alice]
age = "age1…"
ssh = "ssh-ed25519 AAAA…"
```

`trust` writes this file. The machine itself keeps only its name, in `/etc/mothership/machine.toml`.

Each line of `ignore.txt` is a kind and a pattern, such as `home ~/.cache` or `package linux-firmware-*`.

## Installing

On Debian and Ubuntu, install offers to remove every package the type doesn't list, so the type lists what the machine needs: the kernel, `openssh-server`, `sudo`, git and chezmoi.

From the Arch ISO, install also partitions and encrypts the disk:

- Start the ISO in UEFI mode, with Secure Boot in Setup Mode and a TPM 2.0.
- New Secure Boot keys make BitLocker ask for its recovery key: suspend it first.
- The type lists `linux`, `sbctl` and `tpm2-tss`, holds `/etc/kernel/cmdline`, and has a mkinitcpio preset that builds a UKI with the `systemd` and `sd-encrypt` hooks, plus `sd-vconsole` when `vconsole.conf` sets a keymap. Install checks this before it touches the disk.
- At the next start, type the disk's passphrase once. After that, the TPM opens the disk.

## Services

On a server, the account `services` runs each service with its own rootless Docker, and keeps its data in `/srv/<name>/`. A service folder holds a `compose.yaml` without `image:` lines, the files it mounts, `*.tmpl` templates that read `{{ secret "name" }}`, and `secrets/*.age`, encrypted for the type's root keys, which containers read in `/run/secrets/`. Private images on ghcr.io need `services/ghcr-token.age`, a classic token with only `read:packages`, from which mothership writes Docker's login file. `service.toml` pins each container's image:

```toml
[images.app]
image = "ghcr.io/me/app"
version = "1.4.2"
digest = "sha256:…"
repo = "me/app"
ports.public = ["443/tcp"]
ports.tailscale0 = ["53/udp"]
callers = ["gateway/caddy"]
```

Each port names the network it faces: `public` (every address), `local` (the machine itself) or a network interface, whose addresses the service binds when the interface comes up. `callers` names the containers of other services that may call this one, as `<service>/<container>`: they find it as `<container>.<service>`, such as `app.web` for the container `app` of a service `web`, and no other container reaches it. When a type drops its last service, apply removes the `services` account and everything it set up, and keeps `/srv`.

A small image of your own, such as Caddy with a plugin, can be a recipe: a `Dockerfile` in the service's folder that names its version with `LABEL org.opencontainers.image.version="2.8.4"`, and an image with `recipe = true` and no digest yet. Review then writes the workflow that builds recipes into the fleet, and the probe pins each build made from a commit the machines accepted.

When an update fails its health check, the machine restores the data it copied before the update and starts the versions and files that ran then. A service with bulky data that updates never change names the part to copy, such as `snapshot = ["db"]`. `[probe] host = "server"` in `mothership.toml` makes that machine follow new patches and minors of every image, waiting 2 days for public ones, and push them as commits that only change versions and digests. `trust` gives the probe's key that right. A machine runs a pin the probe set only once the registry agrees with it, and install asks you when the publisher has moved that tag since.

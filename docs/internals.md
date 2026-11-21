# Internals

## Install and boot

- `sbctl enroll-keys --tpm-eventlog` finds no event log inside the chroot.
- zram shows in lsblk as a disk, with no `/sys/block/<name>/device`.
- GitHub refuses SSH to a key it doesn't list, even for a public repository, so install waits until the admin's key is on GitHub.
- gh run with a cleared environment still finds its keyring login, as long as `XDG_RUNTIME_DIR` is set.
- On a fresh Arch install, the admin's sudoers file comes from the fleet's first apply.

## Packages

- pacman writes `%REASON%` only for dependencies.
- Asking pacman or dpkg for edited config files checks every file of every package (about 30 s).
- dpkg lists no package for the `/etc` files ucf keeps, the alternatives it picks, or the `rc*.d` links.
- dpkg writes `newconffile` for a config file it has no hash of yet, and doesn't mark folders. Some packages list files under `/bin` or `/lib`, which link into `/usr`.
- `apt-cache madison` lists only what the sources offer. `apt-cache pkgnames` also counts installed packages.

## Services

- `systemctl list-unit-files` takes half a second.
- A link named like its target is `systemctl link`, not an alias.

## chezmoi

- `chezmoi status` fails on a file it can't decrypt.
- Without `encryption` set, chezmoi keeps `.age` in target names.
- chezmoi follows a link to a file in its source folder, but not a link to a folder. `re-add` replaces a linked file with a copy.
- Root's chezmoi works on a copy of the user's `chezmoistate.boltdb` and writes it back, so a later edit shows as made on the machine and apply leaves it alone.
- chezmoi as root follows the user's links unless the home is a copy bound over it in a private mount namespace (`unshare`).

## git

- `git fetch --upload-pack` works for a folder on the same machine, so root fetches from the admin's checkout without running git in it.
- Debian's git ignores `-c safe.directory` in the `upload-pack` it starts, so the admin fetches root's commits from a bundle.
- A signed commit's payload is the commit object without its `gpgsig` header and the lines continuing it.

## Other tools

- `getent` also asks systemd, which stalls while it lists unit files.
- cargo records a branch as `?branch=`, uv as `?rev=`.
- bun refuses a name before a git source.

## Containers

- Rootless Docker's own network under slirp4netns is 10.0.2.0/24, so it can't reach a host at 10.0.2.2: the services VM test gives QEMU another range.
- Debian ships `dockerd-rootless.sh` in `/usr/share/docker.io/contrib/`, outside `PATH`.
- Inside a rootless container, a host file owned by root shows as `nobody`, and the `services` group shows as root's group. A service's folder is `root:services 750`, so its files can stay readable by everyone inside it.
- Files that containers create in `/srv/<name>/` belong to subordinate ids, which `services` itself can't read. The snapshot runs `cp` inside rootless Docker's user namespace (`nsenter -U -m -t` the daemon's pid).
- `ssh` refuses to run for a user without a name, and systemd's temporary users get one only through nss-systemd.
- systemd hands a credential to a temporary user as a file that user owns, not root.

# Internals

## Install and boot

- `sbctl enroll-keys --tpm-eventlog` finds no event log inside the chroot.
- zram shows in lsblk as a disk, with no `/sys/block/<name>/device`.
- `gh auth token` works offline, `gh auth status` doesn't.
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

## Other tools

- `getent` also asks systemd, which stalls while it lists unit files.
- cargo records a branch as `?branch=`, uv as `?rev=`.
- bun refuses a name before a git source.

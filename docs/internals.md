# Internals.

## Install and boot

- `arch-chroot` shares the ISO's `/run` with the new system, so `/run/archiso` shows inside it. The ISO sets `MOTHERSHIP_FROM_ISO` for the inner run, and that decides.
- Without `/etc/kernel/cmdline`, mkinitcpio puts the running kernel's command line in the UKI. In the chroot, that's the ISO's.
- The TPM key is bound to PCR 7 (Secure Boot state) and to PCR 15 being zero. systemd extends PCR 15 when it opens a root, so a fake root partition someone adds can't unseal this disk's key.
- systemd's gpt-auto finds the partitions by type code (ESP `ef00`, XBOOTLDR `ea00`, root `8304`) and opens the root as `root`. No fstab or crypttab names them.
- `sbctl enroll-keys --microsoft` also enrolls Microsoft's third-party CA, which signs graphics cards' option ROMs. `--tpm-eventlog` finds no event log inside the chroot.
- `beside` puts the kernel in its own XBOOTLDR partition, since Windows' EFI partition is often too small.
- The passphrase is typed with the console keymap at each boot, so the ISO loads the type's keymap before asking for it.
- A disk name ending in a digit takes a `p` before the partition number: `nvme0n1p2`.
- zram shows in lsblk as a disk, with no `/sys/block/<name>/device`.
- `gh auth token` works offline, `gh auth status` doesn't.
- git fails the same way for a private repository and a dropped network, so a fleet only counts as private once github.com answers.
- Debian's installer names the machine in `/etc/hosts` (`127.0.1.1`), and sudo warns on every run when that name is gone.

## Packages

- pacman writes `%REASON%` only for dependencies.
- Asking pacman or dpkg for edited config files checks every file of every package (about 30 s), so scan reads their databases directly.
- Debian's installer marks the whole base system as installed by hand.
- dpkg lists no package for the `/etc` files ucf keeps, the alternatives it picks, or the `rc*.d` links.
- dpkg writes `newconffile` for a config file it has no hash of yet, and doesn't mark folders. Some packages list files under `/bin` or `/lib`, which link into `/usr`.
- apt installs Recommends and keeps them installed, so they count as dependencies.
- An epoch keeps its colon in the name of the archive apt downloaded.
- `apt-cache madison` lists only what the sources offer. `apt-cache pkgnames` also counts installed packages.

## Services

- `systemctl list-unit-files` takes half a second, so scan reads the unit folders.
- A link in a `.wants` folder is named after the unit it enables: for a template, the instance (`getty@tty1.service`).
- A link named like its target is `systemctl link`, not an alias. A link to `/dev/null` is a mask, which the `/etc` check reports.
- Presets: the first rule that names a unit decides. A rule on a template enables the instances it lists, or the template's `DefaultInstance`.
- `systemctl --user --machine user@` needs the user's manager, which only runs after a login or with linger, so apply starts `user@<uid>.service` first.

## chezmoi

- `chezmoi status` fails on a file it can't decrypt, so a user whose key isn't in their list is scanned with `--exclude encrypted`.
- Without `encryption` set, chezmoi keeps `.age` in target names.
- In `status`, the first column says what changed on the machine since chezmoi last wrote the file, the second what its apply would do.
- chezmoi follows a link to a file in its source folder, but not a link to a folder. `re-add` replaces a linked file with a copy.

## Other tools

- `getent` also asks systemd, which stalls while it lists unit files, so users are read from `/etc/passwd`.
- Firefox-family browsers write UI changes to `prefs.js`, and `user.js` overrides them at each start.
- cargo records a branch as `?branch=`, uv as `?rev=`.
- bun refuses a name before a git source. Its installer puts it in `~/.bun/bin`, outside a login shell's PATH.
- A wireless mouse or headset shows up as a battery, with the scope `Device`.
- Rust ignores SIGPIPE, so main restores the default, or `mothership scan | head` would panic.

# Internals.

## Install and boot

- `arch-chroot` shares the ISO's `/run` with the new system, so `/run/archiso` shows inside it. The ISO sets `MOTHERSHIP_FROM_ISO` for the inner run, and that decides.
- Without `/etc/kernel/cmdline`, mkinitcpio puts the running kernel's command line in the UKI. In the chroot, that's the ISO's.
- The TPM key is bound to PCR 7 (Secure Boot state) and to PCR 15 being zero. systemd extends PCR 15 when it opens a root, so a fake root partition someone adds can't unseal this disk's key.
- Install enrols the TPM only if PCR 15 already holds a measurement, since a root named with `root=` or `rd.luks.*` isn't measured.
- At the first start, a wait for the network ends after five minutes, so the service fails and logins stay possible. It stays enabled and runs again at the next start.
- The passwordless pacman rule exists only around each AUR helper run, so code running as the admin at other times can't use it. A hard kill can leave it behind, so every command that takes the lock removes it first.
- PCR 7 lets every UKI this machine ever signed unseal the key, and a dbx update changes it. A signed PCR 11 policy (ukify with a PCR key, `--tpm2-public-key`) survives dbx updates but still trusts old UKIs; revoking them takes systemd's pcrlock.
- archiso names its device on the kernel command line (`archisodevice`, `archisolabel`, `archisosearchuuid`), which still holds after `copytoram` unmounts it, so install doesn't read the mounts. An isohybrid stick carries the ISO's filesystem on the whole disk.
- systemd's gpt-auto finds the partitions by type code (ESP `ef00`, XBOOTLDR `ea00`, root `8304`) and opens the root as `root`. No fstab or crypttab names them.
- `sbctl enroll-keys --microsoft` also enrolls Microsoft's third-party CA, which signs graphics cards' option ROMs. `--tpm-eventlog` finds no event log inside the chroot.
- `beside` puts the kernel in its own XBOOTLDR partition, since Windows' EFI partition is often too small.
- The passphrase is typed with the console keymap at each boot, so the ISO loads the type's keymap before asking for it.
- A disk name ending in a digit takes a `p` before the partition number: `nvme0n1p2`.
- zram shows in lsblk as a disk, with no `/sys/block/<name>/device`.
- `gh auth token` works offline, `gh auth status` doesn't.
- GitHub asks for a login both for a private repository and for one that doesn't exist, so with prompts off git says `terminal prompts disabled` for either.
- Debian's installer names the machine in `/etc/hosts` (`127.0.1.1`), and sudo warns on every run when that name is gone.

## Packages

- pacman writes `%REASON%` only for dependencies.
- The record keeps listed names as the fleet wrote them and scans see real ones, so scan refuses a listed package that only something provides and a listed unit that is an alias, and apply fails when a package it installed doesn't exist under the listed name (a group, a virtual name or a version).
- Asking pacman or dpkg for edited config files checks every file of every package (about 30 s), so scan reads their databases directly.
- Debian's installer marks the whole base system as installed by hand.
- dpkg lists no package for the `/etc` files ucf keeps, the alternatives it picks, or the `rc*.d` links.
- dpkg writes `newconffile` for a config file it has no hash of yet, and doesn't mark folders. Some packages list files under `/bin` or `/lib`, which link into `/usr`.
- apt installs Recommends and keeps them installed, so they count as dependencies.
- An epoch keeps its colon in the name of the archive apt downloaded.
- `apt-cache madison` lists only what the sources offer. `apt-cache pkgnames` also counts installed packages.
- `apt-get purge` runs the packages' purge scripts, which delete state such as openssh-server's host keys. Uninstall runs `apt-get remove`, then deletes the unedited config files itself.
- dpkg takes a missing config file for one the admin deleted, and doesn't bring it back on a reinstall without `--force-confmiss`.
- A diverted file holds the diverting package's version, and the package it came from installed its own at the diversion's target.

## Services

- `systemctl list-unit-files` takes half a second, so scan reads the unit folders.
- A link in a `.wants` folder is named after the unit it enables: for a template, the instance (`getty@tty1.service`).
- A link named like its target is `systemctl link`, not an alias. A link to `/dev/null` is a mask, which the `/etc` check reports.
- Presets: the first rule that names a unit decides. A rule on a template enables the instances it lists, or the template's `DefaultInstance`. systemd matches the patterns with `fnmatch`, which has no `{a,b}`.
- A link in `/etc/systemd/user` enables a unit for every user, so scan reports it as an `/etc` file.
- `systemctl --user --machine user@` needs the user's manager, which only runs after a login or with linger, so apply starts `user@<uid>.service` first.

## chezmoi

- `chezmoi status` fails on a file it can't decrypt, so a user whose key isn't in their list is scanned with `--exclude encrypted`.
- Without `encryption` set, chezmoi keeps `.age` in target names.
- In `status`, the first column says what changed on the machine since chezmoi last wrote the file, the second what its apply would do.
- Root's chezmoi works on a copy of the user's `chezmoistate.boltdb` and writes it back: after an add, the state records the file, so a later edit shows as made on the machine and apply leaves it alone.
- chezmoi adds `private_` only when neither group nor others can read the file, and asks before an add would drop it, so an encrypted add chmods the files to owner-only first, on the machine (with the user's rights) and in the snapshot, so chezmoi's state matches the machine.
- chezmoi follows a link to a file in its source folder, but not a link to a folder. `re-add` replaces a linked file with a copy.

## Other tools

- `getent` also asks systemd, which stalls while it lists unit files, so users are read from `/etc/passwd`.
- Firefox-family browsers write UI changes to `prefs.js`, and `user.js` overrides them at each start.
- cargo records a branch as `?branch=`, uv as `?rev=`.
- bun refuses a name before a git source. Its installer puts it in `~/.bun/bin`, outside a login shell's PATH.
- A wireless mouse or headset shows up as a battery, with the scope `Device`.
- Rust ignores SIGPIPE, so main restores the default, or `mothership scan | head` would panic.

## Code

- The fleet folder's owner is the admin, so every command asks `sudo -l -U` whether they can run a shell as root. On a fresh Arch install their sudoers file comes from the fleet's first apply, so install asks after its apply.
- `tests/` holds unit tests that each module includes with `#[path]`, not standalone test crates, so Cargo's `autotests` is off.
- Root reads a user's files with that user's rights: a thread sets its filesystem uid, gid and groups (they are per thread in Linux, and `setfsuid` drops the file capabilities), so a link a user swaps in leads only where the user could read anyway.
- chezmoi runs as root only on such a copy of the home, bound over the home in a private mount namespace (`unshare`), so it can't follow a user's links. Docker needs `--cap-add SYS_ADMIN` for that.
- Every add that copies machine content records a fingerprint of what its preview showed (the snapshot's digest, or a hash of the `/etc` file) and refuses when it is missing or different. A revert writes the fleet's or the package's version, so it needs none.

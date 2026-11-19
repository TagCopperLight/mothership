# Apply stops before installing when the AUR has no helper

The fleet lists `zeta`, which only the AUR has, but names no AUR helper.

```console
$ cd /var/lib/mothership/fleet &&
> sed -i '/^aur_helper/d' types/workstation/system.toml &&
> sed -i 's/^  "gamma",$/  "gamma",\n  "zeta",/' types/workstation/system.toml &&
> git commit -qam "List zeta without an AUR helper"
```

Apply fails before it installs anything, and changes nothing else.

```console
$ echo y | sudo mothership apply
Coming from the fleet · 4
  Packages
    + alpha            install
    + gamma            install
    + zeta             install
  Files
    + /etc/fleet.conf  create

Apply [y/N]? y
Error: zeta not in pacman's repos, and system.toml sets no aur_helper
[exit 1]
$ pacman -Qq alpha
error: package 'alpha' was not found
[exit 1]
$ cat /etc/fleet.conf
cat: /etc/fleet.conf: No such file or directory
[exit 1]
```

## The helper it names isn't installed

The fleet names `paru`, which the machine lacks.

```console
$ cd /var/lib/mothership/fleet &&
> sed -i 's/^packages = \[$/aur_helper = "paru"\n&/' types/workstation/system.toml &&
> git commit -qam "Name paru as the AUR helper"
```

Apply fails the same way, before pacman installs anything.

```console
$ echo y | sudo mothership apply
...
Apply [y/N]? y
Error: system.toml names paru as the AUR helper, but me has no paru
[exit 1]
$ pacman -Qq alpha
error: package 'alpha' was not found
[exit 1]
```

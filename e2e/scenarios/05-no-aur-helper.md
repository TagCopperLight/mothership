# Apply stops before installing when the AUR has no helper

The fleet lists `zeta`, which only the AUR has, but names no AUR helper.

```console
$ cd ~/.local/share/chezmoi &&
> sed -i '/^aur_helper/d' types/workstation/system.toml &&
> echo zeta >> packages/apps.txt &&
> git commit -qam "List zeta without an AUR helper"
```

Apply fails before it installs anything, and changes nothing else.

```console
$ echo y | sudo mothership apply
Packages (3)
  alpha  install
  gamma  install
  zeta   install

/etc (1)
  /etc/fleet.conf  write the fleet's version

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

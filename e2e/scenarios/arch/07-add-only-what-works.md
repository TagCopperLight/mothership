# Review offers to add only what a new machine would get

The machine follows its fleet. Then I change it in ways a new machine of the type couldn't
repeat from the fleet alone, and one way it could:

- I install `omega`, a package I built here, and `zeta` from the AUR.
- I write a service of my own into `/etc` and enable it.
- I make a group of my own and join it.

```console
$ echo y | sudo mothership apply &>/dev/null
$ sudo pacman -U --noconfirm /local/omega-1-1-any.pkg.tar.zst &>/dev/null
$ yay -S --needed zeta &>/dev/null
$ printf '[Service]\nExecStart=/bin/true\n[Install]\nWantedBy=multi-user.target\n' |
> sudo tee /etc/systemd/system/mine.service >/dev/null &&
> sudo mkdir -p /etc/systemd/system/multi-user.target.wants &&
> sudo ln -s /etc/systemd/system/mine.service /etc/systemd/system/multi-user.target.wants/
$ sudo groupadd mine && sudo usermod -aG mine me
```

`zeta` is in the AUR, so review lists it like any package. `omega` is in neither pacman's
repos nor the AUR, so it goes into the packages apply checks but never installs. The service's unit file and the group come from nothing the fleet holds, so review
doesn't offer to add them.

```console
$ printf 'a\na\np\np\np\ny\n' | sudo mothership review
[1/5] Packages
    + omega  installed  An app built on this machine
  a  add         add it to `packages` in system.toml
  r  revert      uninstall
  i ignore · l later · p pass · q quit
  › a
  It is in neither pacman's repos nor the AUR: yay -Si omega failed: -> No AUR package found for omega
  It goes into `built_packages`.
[2/5] Packages
    + zeta  installed  An app only the AUR has
  a  add         add it to `packages` in system.toml
  r  revert      uninstall
  i ignore · l later · p pass · q quit
  › a
[3/5] Files
    + /etc/systemd/system/mine.service  new
...
  › p
[4/5] Services
    + mine.service  enabled
  r  revert      disable and stop
  can't add: its unit file comes from outside the fleet
  i ignore · l later · p pass · q quit
  › p
[5/5] Account
    + group mine  joined
  r  revert      leave
  can't add: no package the fleet lists creates this group
  i ignore · l later · p pass · q quit
  › p
...
```

Only the packages reached the fleet.

```console
$ grep -E 'zeta|omega' /var/lib/mothership/fleet/types/workstation/system.toml
  "zeta",
built_packages = ["omega"]
$ git -C /var/lib/mothership/fleet show --name-only --format= HEAD
types/workstation/system.toml
```

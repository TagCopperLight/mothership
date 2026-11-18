# Review offers to add only what a new machine would get

The machine follows its fleet. Then I change it in ways a new machine of the type couldn't
repeat from the fleet alone, and one way it could:

- I install `omega`, a package I built here, and `zeta` from the AUR.
- I write a service of my own into `/etc` and enable it.
- I make a group of my own and join it.

```console
$ echo y | sudo mothership apply &>/dev/null
$ sudo pacman -U --noconfirm /local/omega-1-1-any.pkg.tar.zst &>/dev/null
$ fake-aur -S --needed zeta &>/dev/null
$ printf '[Service]\nExecStart=/bin/true\n[Install]\nWantedBy=multi-user.target\n' |
> sudo tee /etc/systemd/system/mine.service >/dev/null &&
> sudo mkdir -p /etc/systemd/system/multi-user.target.wants &&
> sudo ln -s /etc/systemd/system/mine.service /etc/systemd/system/multi-user.target.wants/
$ sudo groupadd mine && sudo usermod -aG mine me
```

`zeta` is in the AUR, so review lists it like any package. `omega` is in neither pacman's
repos nor the AUR, and this fleet has no list of built packages, so the add fails and review
asks again. The service's unit file and the group come from nothing the fleet holds, so review
doesn't offer to add them.

```console
$ printf 'a\n2\na\np\np\np\np\nq\n' | sudo mothership review
\[1/\d+\] Packages (re)
zeta  installed, not listed
    An app only the AUR has
  add: list it in one of the type's package lists
  revert: uninstall
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? a
    1. base
    2. apps
  Which list (number, empty to cancel)? 2
\[2/\d+\] Packages (re)
omega  installed, not listed
    An app built on this machine
  add: list it in one of the type's package lists
  revert: uninstall
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? a
  It is in neither pacman's repos nor the AUR: fake-aur -Si omega failed: -> No AUR package found for omega
  failed: only a `built_packages` list can hold it, and system.toml has none
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? p
\[3/\d+\] /etc (re)
/etc/systemd/system/mine.service  from no package
...
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? p
\[4/\d+\] System services (re)
mine.service  enabled, not declared
...
  its unit file comes from outside the fleet, so it can't be added yet
  revert: disable and stop
  (r)evert (i)gnore (l)ater (p)ass (q)uit? p
\[5/\d+\] Account (re)
group mine  member, not declared
  no package the fleet lists creates this group, so it can't be added yet
  revert: leave
  (r)evert (i)gnore (l)ater (p)ass (q)uit? p
...
```

Only `zeta` reached the fleet.

```console
$ cat ~/.local/share/chezmoi/packages/apps.txt
...
zeta
$ git -C ~/.local/share/chezmoi status --short
 M packages/apps.txt
```

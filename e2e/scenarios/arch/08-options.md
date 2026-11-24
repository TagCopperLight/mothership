# An option brings its block, and takes it back

The machine follows its fleet. Then I turn on the `extra` option in the machine's file in
`machines/`, whose block lists `beta` and `/etc/option.conf`.

```console
$ echo y | sudo mothership apply &>/dev/null
$ cd /var/lib/mothership/fleet &&
> sed -i '1a options = ["extra"]' machines/machine.toml &&
> echo y | sudo mothership commit "Turn on extra" >/dev/null
$ echo y | sudo mothership apply
Coming from the fleet · 2
  Packages
    + beta              install
  Files
    + /etc/option.conf  create

Apply [y/N]? y
...
$ cat /etc/option.conf
option = yes
```

Turning it off again removes both.

```console
$ cd /var/lib/mothership/fleet &&
> sed -i '/^options/d' machines/machine.toml &&
> echo y | sudo mothership commit "Turn off extra" >/dev/null
$ echo y | sudo mothership apply
Coming from the fleet · 2
  Packages
    - beta              uninstall  An app the fleet never lists
  Files
    - /etc/option.conf  delete

Apply [y/N]? y
...
$ pacman -Qq beta
error: package 'beta' was not found
[exit 1]
$ ls /etc/option.conf
ls: cannot access '/etc/option.conf': No such file or directory
[exit 2]
```

An option the type doesn't offer stops everything.

```console
$ cd /var/lib/mothership/fleet &&
> sed -i '1a options = ["nope"]' machines/machine.toml &&
> echo y | sudo mothership commit "Turn on nope" >/dev/null
$ sudo mothership scan
Error: the machine turns on nope, and /var/lib/mothership/current/types/workstation/system.toml has no [options.nope]
[exit 1]
```

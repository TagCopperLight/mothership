# An option brings its block, and takes it back

The machine follows its fleet. Then I turn on the `extra` option, whose block lists `beta` and
`/etc/option.conf`.

```console
$ echo y | sudo mothership apply &>/dev/null
$ printf 'type = "workstation"\noptions = ["extra"]\n' |
> sudo tee /etc/mothership/machine.toml >/dev/null
$ echo y | sudo mothership apply
Packages (1)
  beta  install

/etc (1)
  /etc/option.conf  write the fleet's version

Apply [y/N]? y
...
$ cat /etc/option.conf
option = yes
```

Turning it off again removes both.

```console
$ printf 'type = "workstation"\n' | sudo tee /etc/mothership/machine.toml >/dev/null
$ echo y | sudo mothership apply
Packages (1)
  beta  uninstall

/etc (1)
  /etc/option.conf  delete

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
$ printf 'type = "workstation"\noptions = ["nope"]\n' |
> sudo tee /etc/mothership/machine.toml >/dev/null
$ sudo mothership scan
Error: /etc/mothership/machine.toml turns on nope, and /var/lib/mothership/fleet/types/workstation/system.toml has no [options.nope]
[exit 1]
```

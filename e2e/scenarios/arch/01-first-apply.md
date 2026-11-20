# A new machine follows its fleet

The machine was just set up: the fleet repo sits in `/var/lib/mothership/fleet`, owned by me,
and my chezmoi wrote my files from it, but nothing else ran. The fleet lists `alpha` and `gamma`
and holds `/etc/fleet.conf`.

mothership reads every user's home, so it needs root.

```console
$ mothership scan
Error: mothership reads every user's home: run it with sudo
[exit 1]
$ mothership --help | head -2
usage: sudo mothership <command>
  scan                          show what differs from the fleet
$ mothership scna
Error: mothership has no command scna: `mothership --help` lists them
[exit 1]
```

The scan finds what the fleet wants and the machine lacks. Nothing was ever applied here, so it
all comes from the fleet, and apply brings it.

```console
$ sudo mothership scan
Coming from the fleet · 3
  Packages
    + alpha            install
    + gamma            install
  Files
    + /etc/fleet.conf  create

3 to apply      sudo mothership apply
\d+ ignored (re)
```

Apply shows its plan and asks first.

```console
$ echo n | sudo mothership apply
Coming from the fleet · 3
  Packages
    + alpha            install
    + gamma            install
  Files
    + /etc/fleet.conf  create

Apply [y/N]? n
$ pacman -Qq alpha
error: package 'alpha' was not found
[exit 1]
```

Once I say yes, it installs the packages and writes the file.

```console
$ echo y | sudo mothership apply
Coming from the fleet · 3
  Packages
    + alpha            install
    + gamma            install
  Files
    + /etc/fleet.conf  create

Apply [y/N]? y
...
$ pacman -Qq alpha gamma
alpha
gamma
$ stat -c '%a %U %n' /etc/fleet.conf
644 root /etc/fleet.conf
$ cat /etc/fleet.conf
managed = yes
$ cat /var/log/systemctl.log
daemon-reload
```

The machine now matches its fleet, so a second apply has nothing to do.

```console
$ sudo mothership scan
✓ Nothing differs from the workstation type
\d+ ignored (re)
$ echo y | sudo mothership apply
Nothing to apply.
```

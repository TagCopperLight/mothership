# A new machine follows its fleet

The machine was just set up: the fleet repo sits in `/var/lib/mothership/fleet`, owned by me,
and my chezmoi wrote my files from it, but nothing else ran. The fleet lists `alpha` and `gamma`
and holds `/etc/fleet.conf`.

mothership reads every user's home, so it needs root.

```console
$ mothership scan
Error: mothership reads every user's home: run it with sudo
[exit 1]
```

The scan finds what the fleet wants and the machine lacks.

```console
$ sudo mothership scan
Packages (2)
  alpha  listed, not installed
  gamma  listed, not installed

/etc (1)
  /etc/fleet.conf  missing, the fleet has it

3 differences, \d+ ignored (re)
```

Apply shows its plan and asks first.

```console
$ echo n | sudo mothership apply
Packages (2)
  alpha  install
  gamma  install

/etc (1)
  /etc/fleet.conf  write the fleet's version

Apply [y/N]? n
$ pacman -Qq alpha
error: package 'alpha' was not found
[exit 1]
```

Once I say yes, it installs the packages and writes the file.

```console
$ echo y | sudo mothership apply
Packages (2)
  alpha  install
  gamma  install

/etc (1)
  /etc/fleet.conf  write the fleet's version

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
0 differences, \d+ ignored (re)
$ echo y | sudo mothership apply
Nothing to apply.
```

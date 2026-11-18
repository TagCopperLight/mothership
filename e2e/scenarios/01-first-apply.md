# A new machine follows its fleet

The machine was just set up: the fleet repo is chezmoi's source and chezmoi wrote the home
files, but nothing else ran. The fleet lists `alpha` and `gamma` and holds `/etc/fleet.conf`.

The scan finds what the fleet wants and the machine lacks. Without root, it can't read the
sudoers folder.

```console
$ mothership scan
Packages (2)
  alpha  listed, not installed
  gamma  listed, not installed

/etc (3)
  /etc/fleet.conf       missing, the fleet has it
  /etc/sudoers.d/wheel  unreadable without root
  /etc/sudoers.d        unreadable without root

5 differences, \d+ ignored by ~/\.local/share/chezmoi/types/workstation/ignore\.txt (re)
```

Without root, apply only shows its plan.

```console
$ mothership apply
Packages (2)
  alpha  install
  gamma  install

/etc (1)
  /etc/fleet.conf  write the fleet's version

2 other differences wait for `mothership review`.
Run it with sudo to apply. Without root, unreadable /etc files are left out.
$ pacman -Qq alpha
error: package 'alpha' was not found
[exit 1]
```

With sudo, it asks, then installs the packages and writes the file.

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
$ mothership scan
/etc (2)
  /etc/sudoers.d/wheel  unreadable without root
  /etc/sudoers.d        unreadable without root

2 differences, \d+ ignored by ~/\.local/share/chezmoi/types/workstation/ignore\.txt (re)
$ echo y | sudo mothership apply
Nothing to apply.
```

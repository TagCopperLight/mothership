# Apply follows the fleet's changes and leaves the machine's own

The machine follows its fleet.

```console
$ echo y | sudo mothership apply &>/dev/null
```

Then I change the machine by hand: I install `beta`, remove `gamma`, and edit `/etc/fleet.conf`.

```console
$ sudo pacman -S --noconfirm beta &>/dev/null
$ sudo pacman -R --noconfirm gamma &>/dev/null
$ echo "mine = yes" | sudo tee -a /etc/fleet.conf >/dev/null
```

Meanwhile the fleet changes:

- It lists `epsilon` and `zeta`, and drops `alpha`. `epsilon` needs `alpha`, and `zeta` comes
  from the AUR.
- It changes `/etc/fleet.conf` and adds `/etc/new.conf`.
- It moves the login shell to `/usr/bin/bash` and adds the `video` group.

```console
$ cd /var/lib/mothership/fleet &&
> sed -i 's/^  "alpha",$/  "epsilon",\n  "zeta",/' types/workstation/system.toml &&
> printf 'managed = yes\nlevel = 2\n' > types/workstation/etc/fleet.conf &&
> printf 'new = yes\n' > types/workstation/etc/new.conf &&
> sed -i 's|^shell = .*|shell = "/usr/bin/bash"|; s|^groups = .*|groups = ["wheel", "video"]|' types/workstation/users/me/user.toml &&
> git add -A && git commit -S -qm "Change the workstation"
```

Apply follows the fleet's changes. It leaves the file I edited, and the packages I installed
and removed by hand wait for review.

```console
$ echo y | sudo mothership apply
Coming from the fleet · 6
  Packages
    - alpha            uninstall                           A library other packages need
    + epsilon          install
    + zeta             install
  Files
    + /etc/new.conf    create
  Account
    ~ shell /bin/bash  make /usr/bin/bash the login shell
    + group video      join

Left alone, since they were edited on this machine:
  Files
    ~ /etc/fleet.conf  edited

3 changed on this machine, for sudo mothership review
Apply [y/N]? y
...
```

`alpha` stays, as a dependency, because `epsilon` needs it. `gamma` stays removed and `beta`
stays installed.

```console
$ pacman -Qq alpha beta epsilon zeta
alpha
beta
epsilon
zeta
$ pacman -Qq gamma
error: package 'gamma' was not found
[exit 1]
$ pacman -Qi alpha | grep Reason
Install Reason : Installed as a dependency for another package
$ cat /etc/new.conf /etc/fleet.conf
new = yes
managed = yes
mine = yes
$ getent passwd me | cut -d: -f7
/usr/bin/bash
$ id -nG me | tr ' ' '\n' | sort
me
video
wheel
```

A second apply keeps leaving my changes alone.

```console
$ echo y | sudo mothership apply
Left alone, since they were edited on this machine:
  Files
    ~ /etc/fleet.conf  edited

3 changed on this machine, for sudo mothership review
Nothing to apply.
```

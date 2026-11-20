# A Debian machine follows its fleet with apt

The machine runs Debian, so mothership reads dpkg's database and installs with apt. The fleet
lists `alpha` and `gamma` and holds `/etc/fleet.conf`.

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
$ echo y | sudo mothership apply
Coming from the fleet · 3
  Packages
    + alpha            install
    + gamma            install
  Files
    + /etc/fleet.conf  create

Apply [y/N]? y
...
$ dpkg-query -W -f '${Package} ${db:Status-Abbrev}\n' alpha gamma
alpha ii
gamma ii
$ sudo mothership scan
✓ Nothing differs from the server type
\d+ ignored (re)
```

I install `beta` from apt's sources and `omega`, which I built, and I edit the config file
`gamma` ships. Scan tells which package owns the file.

```console
$ sudo apt-get install -y beta > /dev/null 2>&1
$ sudo dpkg -i /local/omega_1_all.deb > /dev/null
$ echo "volume = 11" | sudo tee /etc/gamma.conf > /dev/null
$ sudo mothership scan
Changed on this machine · 3
  Packages
    + beta             installed  An app the fleet never lists
    + omega            installed  An app built on this machine
  Files
    ~ /etc/gamma.conf  edited     owned by gamma

3 to review     sudo mothership review
\d+ ignored (re)
```

Review adds both packages. apt can't install `omega`, so it goes into `built_packages`. The
file's diff and its revert both come from the package apt keeps in its cache.

```console
$ printf 'a\na\nr\ny\n' | sudo mothership review
[1/3] Packages
    + beta  installed  An app the fleet never lists
  a  add         add it to `packages` in system.toml
  r  revert      uninstall
  i ignore · l later · p pass · q quit
  › a
[2/3] Packages
    + omega  installed  An app built on this machine
  a  add         add it to `packages` in system.toml
  r  revert      uninstall
  i ignore · l later · p pass · q quit
  › a
  It is in none of apt's sources.
  It goes into `built_packages`.
[3/3] Files
    ~ /etc/gamma.conf  edited  owned by gamma
      @@ -1 +1 @@
      -volume = 5
      +volume = 11
  a  add         copy it into the type's etc/
  s  secret      copy it into the type's etc/, encrypted
  r  revert      restore the package's version
  i ignore · l later · p pass · q quit
  › r
2 added, 1 reverted
Review on machine: 2 added:
  M types/server/system.toml
Then on this machine:
  /etc/gamma.conf: restore the package's version
Commit, push and revert [y/N]? y
Committed [0-9a-f]{12} Review on machine: 2 added (re)
$ cat /etc/gamma.conf
volume = 5
$ git -C /var/lib/mothership/fleet show -U0 HEAD | grep '^[+-] '
+  "beta",
$ grep built_packages /var/lib/mothership/fleet/types/server/system.toml
built_packages = ["omega"]
```

The fleet drops `alpha` and `gamma`. Apply removes `gamma` and deletes its unedited config
file, without running its purge script, and keeps `alpha`, which `epsilon` needs, as a
dependency.

```console
$ sudo apt-get install -y epsilon > /dev/null 2>&1
$ cd /var/lib/mothership/fleet &&
> sed -i '/"alpha",/d; /"gamma",/d' types/server/system.toml
> echo y | sudo mothership commit "Drop alpha and gamma" >/dev/null
$ echo y | sudo mothership apply
Coming from the fleet · 2
  Packages
    - alpha  uninstall  A library other packages need
    - gamma  uninstall  An app with a config file

1 changed on this machine, for sudo mothership review
Apply [y/N]? y
...
$ dpkg-query -W -f '${Package} ${db:Status-Abbrev}\n' alpha epsilon
alpha ii
epsilon ii
$ apt-mark showauto alpha
alpha
$ dpkg-query -W -f '${db:Status-Abbrev}\n' gamma
rc
$ ls /etc/gamma.conf
ls: cannot access '/etc/gamma.conf': No such file or directory
[exit 2]
```

When the fleet lists `gamma` again, apply brings its config file back with it.

```console
$ cd /var/lib/mothership/fleet &&
> git revert --no-commit HEAD && echo y | sudo mothership commit "List alpha and gamma again" >/dev/null
$ echo y | sudo mothership apply > /dev/null
$ cat /etc/gamma.conf
volume = 5
```

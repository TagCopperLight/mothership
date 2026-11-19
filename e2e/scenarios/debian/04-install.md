# Install sets up a server from its fleet

The machine comes with Debian and nothing of mothership: no fleet and no `machine.toml`. I
installed `beta` by hand, as Debian's installer would leave a package the fleet never lists.

```console
$ git clone -q --bare /var/lib/mothership/fleet ~/fleet.git
$ sudo rm -r /var/lib/mothership/fleet /etc/mothership
$ sudo apt-get install -y beta > /dev/null 2>&1
```

Install clones the fleet, writes the machine's type and hostname, creates the keys and trusts
them, since the type's lists are empty. Then it applies the fleet, and offers to remove what the
fleet's packages don't need.

```console
$ echo y | sudo mothership install --fleet /home/me/fleet.git --type server --hostname box
...
box opens the 0 secrets of server/root now. Commit and push the fleet.
box opens the 0 secrets of server/me now. Commit and push the fleet.
Coming from the fleet · 3
  Packages
    + alpha            install
    + gamma            install
  Files
    + /etc/fleet.conf  create

1 changed on this machine, for sudo mothership review
...
The fleet's packages don't need these 1:
  beta
Remove them [y/N]? y
...
✓ Nothing differs from the server type
The fleet has 2 changes not committed  git -C /var/lib/mothership/fleet status
\d+ ignored (re)
$ cat /etc/mothership/machine.toml /etc/hostname
type = "server"
box
$ dpkg-query -W -f '${Package} ${db:Status-Abbrev}\n' alpha gamma
alpha ii
gamma ii
$ dpkg-query -W -f '${db:Status-Abbrev}\n' beta
...
[exit 1]
```

The two key lists are new, so I commit and push them. Running install again then finds
everything done.

```console
$ cd /var/lib/mothership/fleet && git add -A &&
> git -c user.name=Me -c user.email=me@example.com commit -qm "Trust box" && git push -q
$ sudo mothership install
...
✓ Nothing differs from the server type
\d+ ignored (re)
```

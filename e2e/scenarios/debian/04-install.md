# Install sets up a server from its fleet

The machine comes with Debian and nothing of mothership: no fleet, no keys and no `machine.toml`.
I installed `beta` by hand, as Debian's installer would leave a package the fleet never lists.

```console
$ git clone -q --bare /var/lib/mothership/fleet ~/fleet.git
$ sudo rm -r /var/lib/mothership /etc/mothership
$ sudo apt-get install -y beta > /dev/null 2>&1
```

No machine that signs for the fleet is at hand, so I type the recovery key. Install trusts the
fleet from the newest commit the recovery key signed, so it stops on a fleet where it signed
none.

```console
$ git clone -q --bare ~/fleet.git ~/unsigned.git &&
> git -C ~/unsigned.git update-ref refs/heads/main "$(git -C ~/unsigned.git commit-tree -m 'The fleet' 'main^{tree}')"
$ printf 'r\n%s\n' "$(grep AGE-SECRET ~/recovery.txt)" |
> sudo mothership install --admin me --fleet /home/me/unsigned.git --type server --hostname box 2>&1 | tail -1
Error: no commit on the fleet's main is signed by the recovery key, so nothing tells which commits to trust: trust this machine from one that signs for the fleet
$ sudo rm -r /var/lib/mothership /etc/mothership
```

Someone with my git login pushed a commit. Install clones the fleet, writes the machine's name,
type and hostname, and creates the keys. It refuses that commit, and I drop it. Install trusts
its keys itself and signs that with the recovery key. Then it applies the fleet, and offers to
remove what the fleet's packages don't need.

```console
$ git clone -q ~/fleet.git ~/work && cd ~/work && echo evil > types/server/backdoor &&
> git add -A && git -c user.name=Thief -c user.email=thief@example.com commit -qm "Add a backdoor" && git push -q
$ printf 'r\n%s\nd\n\ny\ny\n' "$(grep AGE-SECRET ~/recovery.txt)" |
> sudo mothership install --admin me --fleet /home/me/fleet.git --type server --hostname box
...
Run this on a machine that signs for the fleet:
  sudo mothership trust box server root=age1[a-z0-9]+ me=age1[a-z0-9]+ 'me=ssh-ed25519 [A-Za-z0-9+/]+' 'signing=ssh-ed25519 [A-Za-z0-9+/]+' (re)
c type the code it prints · r type the recovery key › r
Recovery key: 
main holds 1 commit after [0-9a-f]{12}, the last one this machine accepts: (re)
  [0-9a-f]{12} Thief, \d{4}-\d\d-\d\d: Add a backdoor (re)
They change:
  A types/server/backdoor
v vouch for them, signing on top · d drop them from main · n change nothing › d
box may change: Enter for types/server, * for everything, or folders separated by spaces › 
box opens the 0 secrets of server/root now.
box opens the 0 secrets of server/me now.
server/me on box has an SSH key in the fleet now.
box signs for the fleet now.
Trust box as server:
  A machines/box.toml
Commit and push [y/N]? y
Committed [0-9a-f]{12} Trust box as server (re)
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
\d+ ignored (re)
$ cat /etc/mothership/machine.toml /etc/hostname
name = "box"
box
$ dpkg-query -W -f '${Package} ${db:Status-Abbrev}\n' alpha gamma
alpha ii
gamma ii
$ dpkg-query -W -f '${db:Status-Abbrev}\n' beta
...
[exit 1]
$ git -C /home/me/fleet.git log --format=%s
Trust box as server
The fleet
```

Running install again finds everything done.

```console
$ sudo mothership install
...
✓ Nothing differs from the server type
\d+ ignored (re)
```

The fleet now has `machines/box.toml`, so a reinstall of `box` takes the type from it. A `--type`
that says otherwise stops install before it touches anything, naming the file.

```console
$ sudo rm -r /var/lib/mothership /etc/mothership
$ sudo mothership install --admin me --fleet /home/me/fleet.git --type desktop --hostname box 2>&1 | tail -1
Error: --type desktop: machines/box.toml says box is a server
```

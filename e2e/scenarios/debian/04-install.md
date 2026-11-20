# Install sets up a server from its fleet

The machine comes with Debian and nothing of mothership: no fleet, no keys and no `machine.toml`.
I installed `beta` by hand, as Debian's installer would leave a package the fleet never lists.

```console
$ git clone -q --bare /var/lib/mothership/fleet ~/fleet.git
$ sudo rm -r /var/lib/mothership /etc/mothership
$ sudo apt-get install -y beta > /dev/null 2>&1
```

Install clones the fleet, writes the machine's type and hostname, and creates the keys. No
machine that signs for the fleet is at hand, so I type the recovery key: install shows the
fleet's newest commits, and once I trust them, it trusts its keys itself and signs that with the
recovery key. Then it applies the fleet, and offers to remove what the fleet's packages don't
need.

```console
$ printf 'r\n%s\ny\nt\ny\ny\n' "$(grep AGE-SECRET ~/recovery.txt)" |
> sudo mothership install --fleet /home/me/fleet.git --type server --hostname box
...
Run this on a machine that signs for the fleet:
  sudo mothership trust box server root=age1[a-z0-9]+ me=age1[a-z0-9]+ 'signing=ssh-ed25519 [A-Za-z0-9+/]+' (re)
c type the code it prints · r type the recovery key › r
Recovery key: 
The fleet's newest commits:
  [0-9a-f]{12} Me: The fleet  \(the fleet's first commit\) (re)
Trust the fleet as it is [y/N]? y
box opens the 0 secrets of server/root now.
box opens the 0 secrets of server/me now.
box signs: t only types/server · e everything › t
box signs for the fleet now.
Trust box as server:
  M signers.txt
  A types/server/recipients.txt
  A types/server/users/me/recipients.txt
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
type = "server"
box
$ dpkg-query -W -f '${Package} ${db:Status-Abbrev}\n' alpha gamma
alpha ii
gamma ii
$ dpkg-query -W -f '${db:Status-Abbrev}\n' beta
...
[exit 1]
$ git -C /home/me/fleet.git log -1 --format=%s
Trust box as server
```

Running install again finds everything done.

```console
$ sudo mothership install
...
✓ Nothing differs from the server type
\d+ ignored (re)
```

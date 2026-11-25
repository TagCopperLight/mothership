# Install sets up a new fleet's first machine

I wrote a new fleet: its types, but no machines and no `mothership.toml`. The machine has nothing
of mothership yet.

```console
$ mkdir ~/new && git -C /var/lib/mothership/fleet archive HEAD types | tar -x -C ~/new &&
> git -C ~/new init -q -b main && git -C ~/new add -A &&
> git -C ~/new -c user.name=Me -c user.email=me@example.com commit -q -m "A new fleet" &&
> git clone -q --bare ~/new ~/fleet.git
$ sudo rm -r /var/lib/mothership /etc/mothership
```

No machine signs for the fleet, so install goes straight to the recovery key. The fleet has none,
so install makes one, and names it in the commit that trusts the machine, signed with it.

```console
$ printf 'n\ny\n\ny\n' |
> sudo mothership install --admin me --fleet /home/me/fleet.git --type workstation --hostname box &> ~/out
$ sed -n '/^No machine/,/^Committed/p' ~/out
No machine signs for the fleet yet, so the recovery key lets this one in.
The fleet has no recovery key yet:
n make a new one · t type the one you have › n
The recovery key opens every secret, signs any change, and nothing else keeps it. Write it on paper and in your password manager:
  AGE-SECRET-KEY-[0-9A-Z]+ (re)
The fleet's newest commits:
  [0-9a-f]{12} Me: A new fleet  \(the fleet's first commit\) (re)
Trust the fleet as it is [y/N]? y
...
Trust box as workstation:
  A machines/box.toml
  A mothership.toml
Commit and push [y/N]? y
Committed [0-9a-f]{12} Trust box as workstation (re)
```

The fleet's copy on the machine has nothing left to commit, and the pushed fleet names the key.

```console
$ git -C /var/lib/mothership/fleet status --short
$ git -C ~/fleet.git show main:mothership.toml | grep -c '^age = "age1'
1
$ sudo mothership apply < /dev/null > /dev/null
```

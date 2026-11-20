# Review adds a group a listed package's install script creates

Debian packages create their groups in their install scripts. `lambda`'s creates the `lambda`
group. I install it, add it to the fleet, then join its group and one I make by hand.

```console
$ echo y | sudo mothership apply > /dev/null 2>&1
$ sudo apt-get install -y lambda > /dev/null 2>&1
$ printf 'a\n' | sudo mothership review > /dev/null
$ sudo groupadd mine && sudo gpasswd -a me lambda > /dev/null && sudo gpasswd -a me mine > /dev/null
```

A new machine of the type would get `lambda`'s group from the package, so review offers to add
it. Nothing the fleet lists creates `mine`.

```console
$ printf 'a\np\n' | sudo mothership review
[1/2] Account
    + group lambda  joined
  a  add         add it to `groups` in user.toml
  r  revert      leave
  i ignore · l later · p pass · q quit
  › a
[2/2] Account
    + group mine  joined
  r  revert      leave
  can't add: no package the fleet lists creates this group
  i ignore · l later · p pass · q quit
  › p
1 added, 1 passed
Check the changes with git -C /var/lib/mothership/fleet diff, then commit.
$ grep groups /var/lib/mothership/fleet/types/server/users/me/user.toml
groups = ["sudo", "lambda"]
```

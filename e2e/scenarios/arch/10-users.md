# Each user of the type gets their own home folder

The machine follows its fleet. Then the fleet adds a second user, `dev`, who keeps their
services running without a login, gets a tool from an installer, and has an ignore list of
their own. `.gitconfig` moves to `shared/`, and both home folders link it.

```console
$ echo y | sudo mothership apply &>/dev/null
$ cd /var/lib/mothership/fleet && mkdir -p types/workstation/users/dev/home shared &&
> git mv types/workstation/users/me/home/dot_gitconfig shared/dot_gitconfig &&
> ln -s ../../../../../shared/dot_gitconfig types/workstation/users/me/home/dot_gitconfig &&
> ln -s ../../../../../shared/dot_gitconfig types/workstation/users/dev/home/dot_gitconfig &&
> printf 'linger = true\n\n[[installers]]\nname = "greeter"\nrun = "mkdir ~/.greeter && echo hi > ~/.greeter/hi"\ncreates = "~/.greeter"\n' > types/workstation/users/dev/user.toml &&
> printf 'home ~/.cargo\nhome ~/.bash{rc,_profile,_logout}\n' > types/workstation/users/dev/ignore.txt &&
> git add -A && git commit -S -qm "Add dev"
$ sudo mothership scan
Coming from the fleet · 1
  Users
    + dev  create it

1 to apply      sudo mothership apply
The fleet has 1 commit not pushed  git -C /var/lib/mothership/fleet status
\d+ ignored (re)
```

Apply creates `dev`, then goes round again for what the new account brings: dev's chezmoi
config, linger and the installer, then dev's files. dev gets a key too.

```console
$ printf 'y\ny\ny\n' | sudo mothership apply
Coming from the fleet · 1
  Users
    + dev  create it

Apply [y/N]? y
Coming from the fleet · 3
  Tools · dev
    + greeter  run its installer   ~/.greeter is missing
  Files · dev
    + chezmoi  write its config    its config isn't the one mothership writes
  Account · dev
    + linger   turn it on          user services run without a login

Apply [y/N]? y
Coming from the fleet · 1
  Files · dev
    + ~/.gitconfig  create

Apply [y/N]? y
$ ls /var/lib/systemd/linger
dev
$ sudo stat -c '%U %a %n' /home/dev/.greeter/hi /home/dev/.gitconfig /home/dev/.config/mothership/key.txt
dev 644 /home/dev/.greeter/hi
dev 644 /home/dev/.gitconfig
dev 600 /home/dev/.config/mothership/key.txt
$ sudo grep sourceDir /home/dev/.config/chezmoi/chezmoi.toml
sourceDir = "/var/lib/mothership/current/types/workstation/users/dev/home"
$ sudo mothership scan
✓ Nothing differs from the workstation type
The fleet has 1 commit not pushed  git -C /var/lib/mothership/fleet status
\d+ ignored (re)
```

`dev` can't write the fleet, which I own. Review adds dev's new config to dev's own folder: the
files it writes into the fleet are mine, and dev's chezmoi state stays dev's.

```console
$ sudo -u dev bash -c 'mkdir -p ~/.config/devapp && echo "theme = light" > ~/.config/devapp/conf'
$ printf 'a\ny\n' | sudo mothership review
[1/1] Files · dev
    + ~/.config/devapp  new  1 file
      ── conf
      theme = light
  a  add         add it to chezmoi
  s  secret      add it to chezmoi, encrypted
  i ignore · l later · p pass · q quit
  › a
  That adds 1 file. Go on [y/N]? y
1 added
Committed [0-9a-f]{12} Review on machine: 1 added (re)
$ cd /var/lib/mothership/fleet && find types -name conf
types/workstation/users/dev/home/dot_config/devapp/conf
$ find /var/lib/mothership/fleet -not -user me | wc -l
0
$ sudo stat -c %U /home/dev/.config/chezmoi/chezmoistate.boltdb
dev
$ sudo mothership scan
✓ Nothing differs from the workstation type
\d+ ignored (re)
```

When dev changes the shared `.gitconfig`, review says it reaches me too, then writes the change
into `shared/`, and the link stays. Apply then brings it to me.

```console
$ sudo -u dev bash -c 'echo "[user]" >> ~/.gitconfig'
$ printf 'a\ny\n' | sudo mothership review
[1/1] Files · dev
    ~ ~/.gitconfig  edited
      @@ -1,3 +1,4 @@
       [user]
           name = Me
           email = me@example.com
      +[user]
  a  add         copy this machine's version into chezmoi
  s  secret      copy this machine's version into chezmoi, encrypted
  r  revert      write the fleet's version
  i ignore · l later · p pass · q quit
  › a
  shared/dot_gitconfig is shared: dev's version reaches workstation/me too.
  Go on [y/N]? y
1 added
Committed [0-9a-f]{12} Review on machine: 1 added (re)
$ cd /var/lib/mothership/fleet && git show --name-only --format= HEAD && test -L types/workstation/users/dev/home/dot_gitconfig && tail -1 shared/dot_gitconfig
shared/dot_gitconfig
[user]
$ echo y | sudo mothership apply
Coming from the fleet · 1
  Files · me
    ~ ~/.gitconfig  update

Apply [y/N]? y
$ tail -1 ~/.gitconfig
[user]
```

`dev` has a key of their own, next to root's and mine. From this machine I can trust a key for a
user of another type, whose secrets I list from the fleet's rules for that type.

```console
$ sudo mothership key | sed 's/age1[a-z0-9]*/<key>/g'
root     <key>
dev      <key>
me       <key>
signing  ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBiaU3yvKl1XfZP1NPwoqvUpk/NKEmD0N2A069AmOCrB
Run this on a machine that signs for the fleet:
  sudo mothership trust machine workstation root=<key> dev=<key> me=<key>
$ cd /var/lib/mothership/fleet && mkdir -p types/server/etc types/server/users/dev &&
> touch types/server/system.toml types/server/users/dev/user.toml types/server/users/dev/recipients.txt &&
> git add -A && git commit -S -qm "Add a server type"
$ sudo mothership trust laptop server dev="$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
laptop opens the 0 secrets of server/dev now.
Committed [0-9a-f]{12} Trust laptop as server (re)
On laptop, type this code: [0-9a-f]{12} (re)
$ sed 's/age1[a-z0-9]*/<key>/' /var/lib/mothership/fleet/types/server/users/dev/recipients.txt
# laptop
<key>
```

A secret both homes use lives once in `shared/.secrets/`, and each home links it. Trust re-encrypts
it for the lists of both homes, and asks first. My key also opens dev's list under another name,
`devbox`: retiring `devbox` leaves the secret open to my key through my own home.

```console
$ sudo mothership trust laptop workstation me="$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
laptop opens the 0 secrets of workstation/me now.
Committed [0-9a-f]{12} Trust laptop as workstation (re)
On laptop, type this code: [0-9a-f]{12} (re)
$ cd /var/lib/mothership/fleet && mkdir -p shared/.secrets types/workstation/users/{me,dev}/home/.secrets &&
> echo "SHARED-TOKEN" | chezmoi encrypt > shared/.secrets/token.age &&
> for user in me dev; do ln -s ../../../../../../shared/.secrets/token.age types/workstation/users/$user/home/.secrets/token.age; done &&
> echo '{{ include ".secrets/token.age" | decrypt | trim }}' > shared/dot_token.tmpl &&
> ln -s ../../../../../shared/dot_token.tmpl types/workstation/users/dev/home/dot_token.tmpl &&
> git add -A && git commit -S -qm "Share a token"
$ echo y | sudo mothership trust minipc workstation dev="$(sudo grep -o 'age1.*' /home/dev/.config/mothership/key.txt)" 
Re-encrypting for workstation/dev:
  .secrets/token.age  workstation/dev, workstation/me
Go on [y/N]? y
minipc opens the 1 secrets of workstation/dev now.
Committed [0-9a-f]{12} Trust minipc as workstation (re)
On minipc, type this code: [0-9a-f]{12} (re)
$ echo n | sudo mothership trust devbox workstation dev="$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
Re-encrypting for workstation/dev:
  .secrets/token.age  workstation/dev, workstation/me
Go on [y/N]? n
Nothing changed.
$ echo y | sudo mothership trust devbox workstation dev="$(grep -o 'age1.*' ~/.config/mothership/key.txt)" | tail -3
devbox opens the 1 secrets of workstation/dev now.
Committed [0-9a-f]{12} Trust devbox as workstation (re)
On devbox, type this code: [0-9a-f]{12} (re)
$ echo y | sudo mothership retire devbox
Re-encrypting for workstation/dev:
  .secrets/token.age  workstation/dev, workstation/me
Go on [y/N]? y
devbox is gone from workstation/dev.
git history keeps the versions devbox opens, and that machine may keep copies, so change these:
  .secrets/token.age
devbox's key still opens these, through another home that shares them:
  .secrets/token.age  workstation/me
Committed [0-9a-f]{12} Retire devbox (re)
$ echo y | sudo mothership apply
Coming from the fleet · 1
  Files · dev
    + ~/.token  create

Apply [y/N]? y
$ sudo cat /home/dev/.token
SHARED-TOKEN
$ test -e ~/.token || test -e ~/.secrets || test -e /home/dev/.secrets || echo "nothing else"
nothing else
```

Scan shows accounts the type doesn't list. Adding one lists the user with an empty home, which
the next scan treats like any other user. Ignoring one writes a `user` line in the type's
`ignore.txt`.

```console
$ sudo useradd -m -s /bin/bash guest && sudo useradd -m -s /bin/bash visitor
$ sudo mothership scan
Changed on this machine · 2
  Users
    + guest    not in the fleet
    + visitor  not in the fleet

2 to review     sudo mothership review
\d+ ignored (re)
$ printf 'a\ny\ni\n' | sudo mothership review
[1/2] Users
    + guest  not in the fleet
  a  add         list the user in the type, with an empty home
  i ignore · l later · p pass · q quit
  › a
  mothership then manages guest's account, services and files here. Go on [y/N]? y
[2/2] Users
    + visitor  not in the fleet
  a  add         list the user in the type, with an empty home
  i ignore · l later · p pass · q quit
  › i
1 added, 1 ignored
Committed [0-9a-f]{12} Review on machine: 1 added, 1 ignored (re)
$ cd /var/lib/mothership/fleet && git show --name-only --format= HEAD && tail -1 types/workstation/ignore.txt
types/workstation/ignore.txt
types/workstation/users/guest/home/.keep
types/workstation/users/guest/recipients.txt
types/workstation/users/guest/user.toml
user visitor
$ sudo mothership scan | head -4
Coming from the fleet · 1
  Files · guest
    + chezmoi  write its config  its config isn't the one mothership writes

```

A file in `shared/` that no home links needs attention: it reaches nobody. Ignoring it writes a
`shared` line in the type's `ignore.txt`.

```console
$ cd /var/lib/mothership/fleet && touch shared/dot_unused && git add -A && git commit -S -qm "Leave a file unlinked"
$ sudo mothership scan | head -4
Needs attention · 1
  Shared files
    + dot_unused  linked by no home  link it from a home, or delete it

$ printf 'i\n' | sudo mothership review | tail -3
1 ignored
1 coming from the fleet, for sudo mothership apply
Committed [0-9a-f]{12} Review on machine: 1 ignored (re)
$ tail -1 /var/lib/mothership/fleet/types/workstation/ignore.txt
shared dot_unused
```

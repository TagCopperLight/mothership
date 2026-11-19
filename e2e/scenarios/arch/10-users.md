# Each user of the type gets their own home folder

The machine follows its fleet. Then the fleet adds a second user, `dev`, who keeps their
services running without a login, gets a tool from an installer, and has an ignore list of
their own. `.gitconfig` moves to `shared/`, and both home folders link it.

```console
$ echo y | sudo mothership apply &>/dev/null
$ cd /var/lib/mothership/fleet && mkdir -p types/workstation/users/dev homes/dev shared &&
> git mv homes/me/dot_gitconfig shared/dot_gitconfig &&
> ln -s ../../shared/dot_gitconfig homes/me/dot_gitconfig &&
> ln -s ../../shared/dot_gitconfig homes/dev/dot_gitconfig &&
> printf 'linger = true\n\n[[installers]]\nname = "greeter"\nrun = "mkdir ~/.greeter && echo hi > ~/.greeter/hi"\ncreates = "~/.greeter"\n' > types/workstation/users/dev/user.toml &&
> printf 'home ~/.cargo\nhome ~/.bash{rc,_profile,_logout}\n' > types/workstation/users/dev/ignore.txt &&
> git add -A && git commit -qm "Add dev"
$ sudo mothership scan
Coming from the fleet · 1
  Account
    + user dev  create it

1 to apply      sudo mothership apply
The fleet has 1 commit not pushed  git -C /var/lib/mothership/fleet status
\d+ ignored (re)
```

Apply creates `dev`, then goes round again for what the new account brings: dev's chezmoi
config, linger and the installer, then dev's files.

```console
$ printf 'y\ny\ny\n' | sudo mothership apply
Coming from the fleet · 1
  Account
    + user dev  create it

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
$ sudo stat -c '%U %n' /home/dev/.greeter/hi /home/dev/.gitconfig
dev /home/dev/.greeter/hi
dev /home/dev/.gitconfig
$ sudo grep sourceDir /home/dev/.config/chezmoi/chezmoi.toml
sourceDir = "/var/lib/mothership/fleet/homes/dev"
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
      conf
  a  add         add it to chezmoi
  s  secret      add it to chezmoi, encrypted
  i ignore · l later · p pass · q quit
  › a
  That adds 1 file. Go on [y/N]? y
1 added
Check the changes with git -C /var/lib/mothership/fleet diff, then commit.
$ cd /var/lib/mothership/fleet && git add -A && git commit -qm "Add devapp" && find homes -name conf
homes/dev/dot_config/devapp/conf
$ find /var/lib/mothership/fleet -not -user me | wc -l
0
$ sudo stat -c %U /home/dev/.config/chezmoi/chezmoistate.boltdb
dev
$ sudo mothership scan
✓ Nothing differs from the workstation type
The fleet has 2 commits not pushed  git -C /var/lib/mothership/fleet status
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
  shared/dot_gitconfig is shared: dev's version reaches me too.
  Go on [y/N]? y
1 added
Check the changes with git -C /var/lib/mothership/fleet diff, then commit.
$ cd /var/lib/mothership/fleet && git status --short && test -L homes/dev/dot_gitconfig && tail -1 shared/dot_gitconfig
 M shared/dot_gitconfig
[user]
$ cd /var/lib/mothership/fleet && git add -A && git commit -qm "Share the change"
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
$ sudo mothership key | sed 's/age1[a-z0-9]*/<key>/'
root    <key>
dev     <key>
me      <key>
Add them from a machine that opens the secrets, or here for the first one:
  sudo mothership trust workstation/root <name> <key>
  sudo mothership trust workstation/dev <name> <key>
  sudo mothership trust workstation/me <name> <key>
$ cd /var/lib/mothership/fleet && mkdir -p types/server/etc types/server/users/dev &&
> touch types/server/system.toml types/server/users/dev/user.toml
$ sudo mothership trust server/dev laptop "$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
laptop opens the 0 secrets of server/dev now. Commit and push the fleet.
$ sed 's/age1[a-z0-9]*/<key>/' /var/lib/mothership/fleet/types/server/users/dev/recipients.txt
# laptop
<key>
```

# Each user of the type gets their own part

The machine follows its fleet. Then the fleet adds a second user, `dev`, who keeps their
services running without a login, gets a tool from an installer, and has an ignore list of
their own.

```console
$ echo y | sudo mothership apply &>/dev/null
$ cd /var/lib/mothership/fleet && mkdir -p types/workstation/users/dev &&
> printf 'linger = true\n\n[[installers]]\nname = "greeter"\nrun = "mkdir ~/.greeter && echo hi > ~/.greeter/hi"\ncreates = "~/.greeter"\n' > types/workstation/users/dev/user.toml &&
> printf 'home ~/.cargo\nhome ~/.bash_logout\n' > types/workstation/users/dev/ignore.txt &&
> git add -A && git commit -qm "Add dev"
$ sudo mothership scan
Account, dev (1)
  user dev  declared, missing

1 differences, \d+ ignored (re)
```

Apply creates `dev`, then goes round again for what the new account brings: dev's chezmoi set
up from the fleet's config template, linger and the installer, then dev's files.

```console
$ printf 'y\ny\ny\n' | sudo mothership apply
Account, dev (1)
  user dev  create it

Apply [y/N]? y
Tools outside pacman, dev (1)
  greeter  run its installer

Files chezmoi tracks, dev (1)
  chezmoi  set it up from the fleet's config template

Account, dev (1)
  linger  turn it on

Apply [y/N]? y
Files chezmoi tracks, dev (3)
  ~/.bash_profile  write the fleet's version
  ~/.bashrc        write the fleet's version
  ~/.gitconfig     write the fleet's version

Apply [y/N]? y
$ ls /var/lib/systemd/linger
dev
$ sudo stat -c '%U %n' /home/dev/.greeter/hi /home/dev/.gitconfig
dev /home/dev/.greeter/hi
dev /home/dev/.gitconfig
$ sudo grep sourceDir /home/dev/.config/chezmoi/chezmoi.toml
sourceDir = "/var/lib/mothership/fleet"
$ sudo mothership scan
0 differences, \d+ ignored (re)
```

`dev` can't write the fleet, which I own. Review adds dev's new config all the same: the files
it writes into the fleet are mine, and dev's chezmoi state stays dev's.

```console
$ sudo -u dev bash -c 'mkdir -p ~/.config/devapp && echo "theme = light" > ~/.config/devapp/conf'
$ printf 'a\ny\n' | sudo mothership review
[1/1] Untracked config in ~, dev
~/.config/devapp  not in the fleet
    folder with 1 files
    conf
  add: add it to chezmoi
  (a)dd (s)ecret (i)gnore (l)ater (p)ass (q)uit? a
  That adds 1 files. Go on [y/N]? y
1 added, 0 reverted, 0 ignored, 0 put off for later, 0 passed
Check the changes with `git -C /var/lib/mothership/fleet diff`, then commit.
$ find /var/lib/mothership/fleet -not -user me | wc -l
0
$ sudo stat -c %U /home/dev/.config/chezmoi/chezmoistate.boltdb
dev
```

The fleet's chezmoi rules know each user's name, so the config stays dev's. When dev edits it,
apply leaves the edit for review.

```console
$ cd /var/lib/mothership/fleet &&
> printf '{{ if ne .user "dev" }}\n.config/devapp\n{{ end }}\n' >> .chezmoiignore &&
> git add -A && git commit -qm "Keep devapp for dev"
$ sudo mothership scan
0 differences, \d+ ignored (re)
$ sudo -u dev bash -c 'echo "theme = dark" > ~/.config/devapp/conf'
$ echo y | sudo mothership apply
Left alone, since they were edited on this machine:
  dev: ~/.config/devapp/conf

Nothing to apply.
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

# Review sorts what changed on the machine

The machine follows its fleet. Then I change it by hand:

- I install `beta` and remove `gamma`.
- I edit `/etc/fleet.conf` and add a proxy script that only root and its group can read.
- I edit `~/.gitconfig`, which chezmoi tracks.
- I add config for a new app, and a folder of tokens.
- I join the `audio` group.

```console
$ echo y | sudo mothership apply &>/dev/null
$ sudo pacman -S --noconfirm beta &>/dev/null
$ sudo pacman -R --noconfirm gamma &>/dev/null
$ echo "mine = yes" | sudo tee -a /etc/fleet.conf >/dev/null
$ echo "export HTTP_PROXY=http://proxy:3128" | sudo install -m 664 /dev/stdin /etc/profile.d/proxy.sh
$ printf '[core]\n\teditor = vim\n' >> ~/.gitconfig
$ mkdir -p ~/.config/newapp ~/.config/tokens &&
> echo "theme = dark" > ~/.config/newapp/settings.ini &&
> echo "abc123" > ~/.config/tokens/api && chmod 600 ~/.config/tokens/api
$ sudo usermod -aG audio me
```

Review goes through what changed on the machine, grouped like scan shows it. I add `beta` to
the fleet's packages, add the new app's config, ignore the tokens and pass on the rest. Review never shows a secret's contents. Then I quit.

```console
$ printf 'a\np\np\na\ny\ni\np\np\nq\ny\n' | sudo mothership review
[1/8] Packages
    + beta  installed  An app the fleet never lists
  a  add         add it to `packages` in system.toml
  r  revert      uninstall
  i ignore · p pass · q quit
  › a
[2/8] Packages
    - gamma  uninstalled
  a  add         remove it from system.toml
  r  revert      install
  i ignore · p pass · q quit
  › p
[3/8] Files
    + /etc/profile.d/proxy.sh  new
      export HTTP_PROXY=http://proxy:3128
  a  add         copy it into the type's etc/
  s  secret      copy it into the type's etc/, encrypted
  r  revert      delete
  i ignore · p pass · q quit
  › p
[4/8] Files
    + ~/.config/newapp  new  1 file
      ── settings.ini
      theme = dark
  a  add         add it to chezmoi
  s  secret      add it to chezmoi, encrypted
  i ignore · p pass · q quit
  › a
  That adds 1 file. Go on [y/N]? y
[5/8] Files
    + ~/.config/tokens  new  secret  1 file
      ── api
      abc123
  s  secret      add it to chezmoi, encrypted
  a  add         add it to chezmoi
  i ignore · p pass · q quit
  › i
[6/8] Files
    ~ /etc/fleet.conf  edited
      @@ -1 +1,2 @@
       managed = yes
      +mine = yes
  a  add         copy it into the type's etc/
  s  secret      copy it into the type's etc/, encrypted
  r  revert      write the fleet's version
  i ignore · p pass · q quit
  › p
[7/8] Files
    ~ ~/.gitconfig  edited
      @@ -1,3 +1,5 @@
       [user]
           name = Me
           email = me@example.com
      +[core]
      +    editor = vim
  a  add         copy this machine's version into chezmoi
  s  secret      copy this machine's version into chezmoi, encrypted
  r  revert      write the fleet's version
  i ignore · p pass · q quit
  › p
[8/8] Account
    + group audio  joined
  a  add         add it to `groups` in user.toml
  r  revert      leave
  i ignore · p pass · q quit
  › q
2 added, 1 ignored, 4 passed
1 not seen yet, for next time
Review on machine: 2 added, 1 ignored:
  M types/workstation/system.toml
  A types/workstation/users/me/home/dot_config/newapp/settings.ini
  M types/workstation/users/me/ignore.txt
Commit and push [y/N]? y
Committed [0-9a-f]{12} Review on machine: 2 added, 1 ignored (re)
```

Review committed what it changed, signed with this machine's key, and pushed it. The checkout
stays mine, and so does my chezmoi state. The tokens go into my own ignore list.

```console
$ cd /var/lib/mothership/fleet && git status --short && git log -1 --format=%s origin/main
Review on machine: 2 added, 1 ignored
$ git -C /var/lib/mothership/fleet show --name-only --format= HEAD
types/workstation/system.toml
types/workstation/users/me/home/dot_config/newapp/settings.ini
types/workstation/users/me/ignore.txt
$ find /var/lib/mothership/fleet ~/.config/chezmoi -not -user me | wc -l
0
$ grep -A3 '"alpha"' /var/lib/mothership/fleet/types/workstation/system.toml
  "alpha",
  "gamma",
  "beta",
]
$ tail -1 /var/lib/mothership/fleet/types/workstation/users/me/ignore.txt
home ~/.config/tokens
```

The fleet raises the level in `/etc/fleet.conf`. Apply leaves the file, since I edited it.

```console
$ cd /var/lib/mothership/fleet &&
> printf 'managed = yes\nlevel = 2\n' > types/workstation/etc/fleet.conf &&
> echo y | sudo mothership commit "Keep beta and newapp, raise the level" >/dev/null
```

I take `gamma` out of the fleet, add the proxy script, revert my edits to `/etc/fleet.conf` and
`~/.gitconfig`, and leave the `audio` group.

```console
$ printf 'a\na\nr\nr\nr\ny\n' | sudo mothership review
[1/5] Packages
    - gamma  uninstalled
  a  add         remove it from system.toml
  r  revert      install
  i ignore · p pass · q quit
  › a
[2/5] Files
    + /etc/profile.d/proxy.sh  new
      export HTTP_PROXY=http://proxy:3128
  a  add         copy it into the type's etc/
  s  secret      copy it into the type's etc/, encrypted
  r  revert      delete
  i ignore · p pass · q quit
  › a
[3/5] Files
    ~ /etc/fleet.conf  edited
      @@ -1,2 +1,2 @@
       managed = yes
      -level = 2
      +mine = yes
  a  add         copy it into the type's etc/
  s  secret      copy it into the type's etc/, encrypted
  r  revert      write the fleet's version
  i ignore · p pass · q quit
  › r
[4/5] Files
    ~ ~/.gitconfig  edited
      @@ -1,3 +1,5 @@
       [user]
           name = Me
           email = me@example.com
      +[core]
      +    editor = vim
  a  add         copy this machine's version into chezmoi
  s  secret      copy this machine's version into chezmoi, encrypted
  r  revert      write the fleet's version
  i ignore · p pass · q quit
  › r
[5/5] Account
    + group audio  joined
  a  add         add it to `groups` in user.toml
  r  revert      leave
  i ignore · p pass · q quit
  › r
2 added, 3 reverted
Review on machine: 2 added:
  A types/workstation/etc/profile.d/proxy.sh
  M types/workstation/system.toml
Then on this machine:
  /etc/fleet.conf: write the fleet's version
  ~/.gitconfig: write the fleet's version
  group audio: leave
Commit, push and revert [y/N]? y
Committed [0-9a-f]{12} Review on machine: 2 added (re)
Removing user me from group audio
```

The machine is back to the fleet, and the files review added are mine. The script
keeps its mode through `etc_modes`.

```console
$ cat ~/.gitconfig
[user]
	name = Me
	email = me@example.com
$ cat /etc/fleet.conf
managed = yes
level = 2
$ id -nG me | tr ' ' '\n' | sort
me
wheel
$ cd /var/lib/mothership/fleet/types/workstation && stat -c '%U %a %n' etc/profile.d etc/profile.d/proxy.sh
me 755 etc/profile.d
me 644 etc/profile.d/proxy.sh
$ grep -A2 '^\[etc_modes\]' /var/lib/mothership/fleet/types/workstation/system.toml
[etc_modes]
"sudoers.d/wheel" = "440"
"profile.d/proxy.sh" = "664"
```

Since review wrote the fleet's version of `/etc/fleet.conf`, apply follows the fleet's next
change to it.

```console
$ cd /var/lib/mothership/fleet &&
> printf 'managed = yes\nlevel = 3\n' > types/workstation/etc/fleet.conf &&
> echo y | sudo mothership commit "Add the proxy, raise the level" >/dev/null
$ echo y | sudo mothership apply
Coming from the fleet · 1
  Files
    ~ /etc/fleet.conf  update

Apply [y/N]? y
$ cat /etc/fleet.conf
managed = yes
level = 3
```

A file whose name isn't valid UTF-8 shows with a replacement character, and review adds the file
itself.

```console
$ printf 'x = 1\n' > "$(printf '/home/me/.config/caf\351.conf')"
$ printf 'a\ny\n' | sudo mothership review
[1/1] Files
    + ~/.config/caf�.conf  new
      x = 1
  a  add         add it to chezmoi
  s  secret      add it to chezmoi, encrypted
  i ignore · p pass · q quit
  › a
1 added
Review on machine: 1 added:
  A types/workstation/users/me/home/dot_config/caf�.conf
Commit and push [y/N]? y
Committed [0-9a-f]{12} Review on machine: 1 added (re)
$ ls /var/lib/mothership/fleet/types/workstation/users/me/home/dot_config | od -c | grep -c 351
1
$ sudo mothership scan | grep -c -e caf -e 'not scanned'
0
[exit 1]
```

Review changes nothing until its last question. Saying no there keeps the fleet and the machine
as they were.

```console
$ echo "mine" | sudo tee /etc/fleet.conf > /dev/null && printf 'x = 2\n' > ~/.config/other.conf
$ printf 'a\nr\nn\n' | sudo mothership review | tail -6
Review on machine: 1 added:
  A types/workstation/users/me/home/dot_config/other.conf
Then on this machine:
  /etc/fleet.conf: write the fleet's version
Commit, push and revert [y/N]? n
Nothing changed.
$ cat /etc/fleet.conf && git -C /var/lib/mothership/fleet log -1 --format=%s
mine
Review on machine: 1 added
```

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
$ echo "export HTTP_PROXY=http://proxy:3128" | sudo install -m 640 /dev/stdin /etc/profile.d/proxy.sh
$ printf '[core]\n\teditor = vim\n' >> ~/.gitconfig
$ mkdir -p ~/.config/newapp ~/.config/tokens &&
> echo "theme = dark" > ~/.config/newapp/settings.ini &&
> echo "abc123" > ~/.config/tokens/api && chmod 600 ~/.config/tokens/api
$ sudo usermod -aG audio me
```

Review goes through the machine first, then my own part. I add `beta` to the fleet's packages,
put `gamma` off, pass on the files I edited, add the new app's config and ignore the tokens.
Review never shows a secret's contents. Then I quit.

```console
$ printf 'a\nl\np\np\np\na\ny\ni\nq\n' | sudo mothership review
[1/8] Packages
beta  installed, not listed
    An app the fleet never lists
  add: add it to `packages` in system.toml
  revert: uninstall
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? a
[2/8] Packages
gamma  listed, not installed
  add: remove it from system.toml
  revert: install
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? l
[3/8] /etc
/etc/fleet.conf  differs from the fleet
    @@ -1 +1,2 @@
     managed = yes
    +mine = yes
  add: copy it into the type's etc/
  revert: write the fleet's version
  (a)dd (s)ecret (r)evert (i)gnore (l)ater (p)ass (q)uit? p
[4/8] /etc
/etc/profile.d/proxy.sh  from no package
    export HTTP_PROXY=http://proxy:3128
  add: copy it into the type's etc/
  revert: delete
  (a)dd (s)ecret (r)evert (i)gnore (l)ater (p)ass (q)uit? p
[5/8] Files chezmoi tracks, me
~/.gitconfig  edited here
    @@ -1,3 +1,5 @@
     [user]
         name = Me
         email = me@example.com
    +[core]
    +    editor = vim
  add: copy this machine's version into chezmoi
  revert: write the fleet's version
  (a)dd (s)ecret (r)evert (i)gnore (l)ater (p)ass (q)uit? p
[6/8] Untracked config in ~, me
~/.config/newapp  not in the fleet
    folder with 1 files
    settings.ini
  add: add it to chezmoi
  (a)dd (s)ecret (i)gnore (l)ater (p)ass (q)uit? a
  That adds 1 files. Go on [y/N]? y
[7/8] Untracked config in ~, me
~/.config/tokens  not in the fleet  [secret]
  add: add it to chezmoi, encrypted
  (a)dd (n)ot secret (i)gnore (l)ater (p)ass (q)uit? i
[8/8] Account, me
group audio  member, not declared
  add: add it to `groups` in user.toml
  revert: leave
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? q
2 added, 0 reverted, 1 ignored, 1 put off for later, 3 passed
Check the changes with `git -C /var/lib/mothership/fleet diff`, then commit.
```

Review edited the fleet checkout and committed nothing. What it wrote is mine, since I own the
checkout, and my chezmoi state stays mine. The tokens go into my own ignore list.

```console
$ cd /var/lib/mothership/fleet && git status --short | sort
 M types/workstation/system.toml
 M types/workstation/users/me/ignore.txt
?? dot_config/
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

I commit, and the fleet raises the level in `/etc/fleet.conf`. Apply leaves the file, since I
edited it.

```console
$ cd /var/lib/mothership/fleet &&
> printf 'managed = yes\nlevel = 2\n' > types/workstation/etc/fleet.conf &&
> git add -A && git commit -qm "Keep beta and newapp, raise the level"
```

`gamma` waits for next week. I revert my edits to `/etc/fleet.conf` and `~/.gitconfig`, add the
proxy script to the fleet, and leave the `audio` group.

```console
$ printf 'r\na\nr\nr\n' | sudo mothership review
[1/4] /etc
/etc/fleet.conf  differs from the fleet
    @@ -1,2 +1,2 @@
     managed = yes
    -level = 2
    +mine = yes
  add: copy it into the type's etc/
  revert: write the fleet's version
  (a)dd (s)ecret (r)evert (i)gnore (l)ater (p)ass (q)uit? r
[2/4] /etc
/etc/profile.d/proxy.sh  from no package
    export HTTP_PROXY=http://proxy:3128
  add: copy it into the type's etc/
  revert: delete
  (a)dd (s)ecret (r)evert (i)gnore (l)ater (p)ass (q)uit? a
[3/4] Files chezmoi tracks, me
~/.gitconfig  edited here
    @@ -1,3 +1,5 @@
     [user]
         name = Me
         email = me@example.com
    +[core]
    +    editor = vim
  add: copy this machine's version into chezmoi
  revert: write the fleet's version
  (a)dd (s)ecret (r)evert (i)gnore (l)ater (p)ass (q)uit? r
[4/4] Account, me
group audio  member, not declared
  add: add it to `groups` in user.toml
  revert: leave
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? r
Removing user me from group audio
1 added, 3 reverted, 0 ignored, 0 put off for later, 0 passed
1 put off before and not due yet
Check the changes with `git -C /var/lib/mothership/fleet diff`, then commit.
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
"profile.d/proxy.sh" = "640"
```

Since review wrote the fleet's version of `/etc/fleet.conf`, apply follows the fleet's next
change to it.

```console
$ cd /var/lib/mothership/fleet &&
> printf 'managed = yes\nlevel = 3\n' > types/workstation/etc/fleet.conf &&
> git add -A && git commit -qm "Add the proxy, raise the level"
$ echo y | sudo mothership apply
/etc (1)
  /etc/fleet.conf  write the fleet's version

1 other differences wait for `mothership review`.
Apply [y/N]? y
$ cat /etc/fleet.conf
managed = yes
level = 3
```

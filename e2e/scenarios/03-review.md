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

Without root, review can add to the fleet, ignore, put off and pass, but not revert. I add
`beta` to the fleet's packages, put `gamma` off, pass on `~/.gitconfig`, add the new app's config
and ignore the tokens. Review never shows a secret's contents. Then I quit.

```console
$ printf 'a\nl\np\na\ny\ni\nq\n' | mothership review
[1/10] Packages
beta  installed, not listed
    An app the fleet never lists
  add: add it to `packages` in system.toml
  (a)dd (i)gnore (l)ater (p)ass (q)uit? a
[2/10] Packages
gamma  listed, not installed
  add: remove it from system.toml
  (a)dd (i)gnore (l)ater (p)ass (q)uit? l
[3/10] Files chezmoi tracks
~/.gitconfig  edited here
    @@ -1,3 +1,5 @@
     [user]
         name = Me
         email = me@example.com
    +[core]
    +    editor = vim
  add: copy this machine's version into chezmoi
  (a)dd (s)ecret (i)gnore (l)ater (p)ass (q)uit? p
[4/10] Untracked config in ~
~/.config/newapp  not in the fleet
    folder with 1 files
    settings.ini
  add: add it to chezmoi
  (a)dd (s)ecret (i)gnore (l)ater (p)ass (q)uit? a
  That adds 1 files. Go on [y/N]? y
[5/10] Untracked config in ~
~/.config/tokens  not in the fleet  [secret]
  add: add it to chezmoi, encrypted
  (a)dd (n)ot secret (i)gnore (l)ater (p)ass (q)uit? i
[6/10] /etc
/etc/fleet.conf  differs from the fleet
    @@ -1 +1,2 @@
     managed = yes
    +mine = yes
  add: copy it into the type's etc/
  (a)dd (s)ecret (i)gnore (l)ater (p)ass (q)uit? q
2 added, 0 reverted, 1 ignored, 1 put off for later, 1 passed
Run it with sudo to also revert this machine to the fleet.
Check the changes with `git -C ~/.local/share/chezmoi diff`, then commit.
```

Review edited the fleet checkout and committed nothing.

```console
$ cd ~/.local/share/chezmoi && git status --short | sort
 M types/workstation/ignore.txt
 M types/workstation/system.toml
?? dot_config/
$ grep -A3 '"alpha"' ~/.local/share/chezmoi/types/workstation/system.toml
  "alpha",
  "gamma",
  "beta",
]
$ tail -1 ~/.local/share/chezmoi/types/workstation/ignore.txt
home ~/.config/tokens
```

I commit, and the fleet raises the level in `/etc/fleet.conf`. Apply leaves the file, since I
edited it.

```console
$ cd ~/.local/share/chezmoi &&
> printf 'managed = yes\nlevel = 2\n' > types/workstation/etc/fleet.conf &&
> git add -A && git commit -qm "Keep beta and newapp, raise the level"
```

With sudo, review also offers to revert. `gamma` waits for next week. I revert my edits to
`~/.gitconfig` and `/etc/fleet.conf`, add the proxy script to the fleet, and leave the `audio`
group.

```console
$ printf 'r\nr\na\nr\n' | sudo mothership review
[1/4] Files chezmoi tracks
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
[2/4] /etc
/etc/fleet.conf  differs from the fleet
    @@ -1,2 +1,2 @@
     managed = yes
    -level = 2
    +mine = yes
  add: copy it into the type's etc/
  revert: write the fleet's version
  (a)dd (s)ecret (r)evert (i)gnore (l)ater (p)ass (q)uit? r
[3/4] /etc
/etc/profile.d/proxy.sh  from no package
    export HTTP_PROXY=http://proxy:3128
  add: copy it into the type's etc/
  revert: delete
  (a)dd (s)ecret (r)evert (i)gnore (l)ater (p)ass (q)uit? a
[4/4] Account
group audio  member, not declared
  add: add it to `groups` in system.toml
  revert: leave
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? r
Removing user me from group audio
1 added, 3 reverted, 0 ignored, 0 put off for later, 0 passed
1 put off before and not due yet
Check the changes with `git -C ~/.local/share/chezmoi diff`, then commit.
```

The machine is back to the fleet, and the files review added under sudo are mine. The script
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
$ cd ~/.local/share/chezmoi/types/workstation && stat -c '%U %a %n' etc/profile.d etc/profile.d/proxy.sh
me 755 etc/profile.d
me 644 etc/profile.d/proxy.sh
$ grep -A2 '^\[etc_modes\]' ~/.local/share/chezmoi/types/workstation/system.toml
[etc_modes]
"sudoers.d/wheel" = "440"
"profile.d/proxy.sh" = "640"
```

Since review wrote the fleet's version of `/etc/fleet.conf`, apply follows the fleet's next
change to it.

```console
$ cd ~/.local/share/chezmoi &&
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

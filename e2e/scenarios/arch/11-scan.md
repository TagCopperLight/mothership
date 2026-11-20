# Scan tells my changes from the fleet's

The machine follows its fleet. Then both sides move. On the machine, I install `beta`, remove
`gamma`, edit `/etc/fleet.conf`, delete `~/.bash_profile`, add config for a new app and a folder
of tokens, and join the `audio` group.

```console
$ echo y | sudo mothership apply &>/dev/null
$ sudo pacman -S --noconfirm beta &>/dev/null
$ sudo pacman -R --noconfirm gamma &>/dev/null
$ echo "mine = yes" | sudo tee -a /etc/fleet.conf >/dev/null
$ rm ~/.bash_profile
$ mkdir -p ~/.config/newapp/themes ~/.config/tokens &&
> echo "theme = dark" > ~/.config/newapp/settings.ini &&
> touch ~/.config/newapp/themes/light ~/.config/newapp/themes/dark &&
> echo "abc123" > ~/.config/tokens/api && chmod 600 ~/.config/tokens/api
$ sudo usermod -aG audio me
```

In the fleet, `epsilon` replaces `alpha`, `/etc/new.conf` and the `video` group appear, and
`~/.bashrc` changes.

```console
$ cd /var/lib/mothership/fleet &&
> sed -i 's/^  "alpha",$/  "epsilon",/' types/workstation/system.toml &&
> printf 'new = yes\n' > types/workstation/etc/new.conf &&
> sed -i 's|^groups = .*|groups = ["wheel", "video"]|' types/workstation/users/me/user.toml &&
> printf '# from the fleet\n' >> types/workstation/users/me/home/dot_bashrc &&
> echo y | sudo mothership commit "Change the workstation" >/dev/null
```

Scan puts what I changed, which review sorts, apart from what the fleet changed, which apply
brings. Each line says what happened: `+` for what the machine gained, `~` for an edit, `-` for
what it lost. On the fleet's side, the sign and the verb say what apply does to the machine.

```console
$ sudo mothership scan
Changed on this machine · 7
  Packages
    + beta                 installed    An app the fleet never lists
    - gamma                uninstalled
  Files
    + ~/.config/newapp     new          3 files
    + ~/.config/tokens     new          secret  1 file
    ~ /etc/fleet.conf      edited
    - ~/.bash_profile      deleted
  Account
    + group audio          joined

Coming from the fleet · 5
  Packages
    - alpha          uninstall  A library other packages need
    + epsilon        install
  Files
    ~ ~/.bashrc      update
    + /etc/new.conf  create
  Account
    + group video    join

7 to review     sudo mothership review
5 to apply      sudo mothership apply
\d+ ignored (re)
```

Review asks only about my changes. I put `beta` off and agree to the deletion, which takes the
file out of the fleet.

```console
$ printf 'l\np\np\np\np\na\nq\ny\n' | sudo mothership review
...
[6/7] Files
    - ~/.bash_profile  deleted
      @@ -1 +0,0 @@
      -[[ -f ~/.bashrc ]] && . ~/.bashrc
  a  add         remove it from chezmoi
  r  revert      write the fleet's version
  i ignore · l later · p pass · q quit
  › a
[7/7] Account
...
  › q
1 added, 1 put off for later, 4 passed
1 not seen yet, for next time
5 coming from the fleet, for sudo mothership apply
Review on machine: 1 added:
  D types/workstation/users/me/home/dot_bash_profile
Commit and push [y/N]? y
Committed [0-9a-f]{12} Review on machine: 1 added (re)
$ git -C /var/lib/mothership/fleet show --name-status --format= HEAD
D	types/workstation/users/me/home/dot_bash_profile
```

Scan still shows `beta`, with when review brings it back, and leaves it out of the count. Review
pushed its commit and the one before it.

```console
$ sudo mothership scan | head -3
Changed on this machine · 6
  Packages
    + beta                 installed    later, back in 7 days  An app the fleet never lists
$ sudo mothership scan | tail -4
5 to review     sudo mothership review
5 to apply      sudo mothership apply
1 put off for later
\d+ ignored (re)
```

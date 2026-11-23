# A user without sudo can't turn mothership against root

`dev` has no sudo, and might run anything. Root reads dev's files with dev's rights, never
runs dev's login shell, and adds to the fleet only what it showed.

```console
$ echo y | sudo mothership apply &>/dev/null
$ cd /var/lib/mothership/fleet && mkdir -p types/workstation/users/dev/home/dot_config/devapp &&
> echo "theme = light" > types/workstation/users/dev/home/dot_config/devapp/conf &&
> printf 'home ~/.cargo\nhome ~/.bash{rc,_profile,_logout}\n' > types/workstation/users/dev/ignore.txt &&
> touch types/workstation/users/dev/user.toml && echo y | sudo mothership commit "Add dev" >/dev/null
$ printf 'y\ny\ny\n' | sudo mothership apply &>/dev/null &&
> yes | sudo mothership trust machine workstation dev="$(sudo cat /home/dev/.ssh/id_ed25519.pub)" > /dev/null
$ sudo mothership scan
✓ Nothing differs from the workstation type
\d+ ignored (re)
```

dev's login profile hangs and prints junk, and a fake `chezmoi` waits in dev's own folder.
Scan runs neither.

```console
$ sudo -u dev bash -c 'printf "echo junk\nsleep 1000\n" > ~/.profile &&
> mkdir -p ~/.local/bin && printf "#!/bin/sh\necho /etc/passwd\n" > ~/.local/bin/chezmoi &&
> chmod +x ~/.local/bin/chezmoi'
$ timeout 60 sudo mothership scan
Changed on this machine · 2
  Files · dev
    + ~/.local/bin/chezmoi  new
    + ~/.profile            new

2 to review     sudo mothership review
\d+ ignored (re)
$ sudo -u dev rm -r /home/dev/.profile /home/dev/.local
```

A key that is a pipe would hang a reader. Scan says dev's secrets stay locked, and goes on.

```console
$ sudo -u dev bash -c 'mkdir -p ~/.config/mothership && rm -f ~/.config/mothership/key.txt && mkfifo ~/.config/mothership/key.txt'
$ timeout 60 sudo mothership scan 2>&1 | head -1
dev's secrets stay locked: /home/dev/.config/mothership/key.txt isn't a regular file
$ sudo -u dev rm /home/dev/.config/mothership/key.txt
```

While review waits for my answer, dev swaps the folder of a new file for a link to
`/etc/mothership`. The add reads with dev's rights, so root's key stays out of the fleet.

```console
$ echo y | sudo mothership apply &>/dev/null; sudo test -s /etc/mothership/key.txt
$ sudo -u dev bash -c 'echo harmless > ~/.config/devapp/key.txt'
$ sudo -u dev bash -c '(sleep 3; mv ~/.config/devapp ~/old; ln -s /etc/mothership ~/.config/devapp) >/dev/null 2>&1 &'
$ (sleep 6; printf 'a\n') | sudo mothership review
[1/1] Files · dev
    + ~/.config/devapp/key.txt  new
      harmless
  a  add         add it to chezmoi
  s  secret      add it to chezmoi, encrypted
  i ignore · l later · p pass · q quit
  › a
  failed: opening /home/dev/.config/devapp/key.txt: Permission denied (os error 13)
  › 
1 not seen yet, for next time
$ sudo grep -rl AGE-SECRET-KEY /var/lib/mothership/fleet
[exit 1]
```

# The machine applies only commits the fleet's signers signed

An agent working as me edits the checkout without committing. Root applies only what it checked,
so the edit waits, and apply says why.

```console
$ echo y | sudo mothership apply &>/dev/null
$ cd /var/lib/mothership/fleet && sed -i 's/^  "gamma",$/  "gamma",\n  "zeta",/' types/workstation/system.toml
$ echo n | sudo mothership apply | tail -2
Nothing to apply.
The fleet has 1 change not committed  git -C /var/lib/mothership/fleet status
```

It commits without signing and pushes. Scan names the refused commit, and apply stays on the last
good one.

```console
$ cd /var/lib/mothership/fleet && git commit -qam "Add zeta" && git push -q
$ sudo mothership scan
Needs attention · 1
  Fleet commits
    \+ [0-9a-f]{12} Add zeta +refused, \d{4}-\d\d-\d\d, claims to be by Me +nobody signed it: sudo mothership commit asks whether to vouch for it or drop it (re)

1 to review     sudo mothership review
\d+ ignored (re)
$ echo n | sudo mothership apply | head -1
This machine refuses [0-9a-f]{12} "Add zeta" of \d{4}-\d\d-\d\d, which claims to be by Me, and what follows it: nobody signed it (re)
```

A command that signs shows me the commit and asks what to do with it. I answer nothing, and
nothing changes.

```console
$ echo n | sudo mothership commit "On top" > ~/out; grep -A1 "^main holds" ~/out; tail -1 ~/out
main holds 1 commit after [0-9a-f]{12}, the last one this machine accepts: (re)
  [0-9a-f]{12} Me, \d{4}-\d\d-\d\d: Add zeta (re)
Nothing changed.
$ git -C /var/lib/mothership/fleet log -1 --format=%s
Add zeta
```

It also tries to sign itself in, with a key of its own. I drop that commit: main and origin go
back to the last commit this machine accepts.

```console
$ ssh-keygen -q -t ed25519 -N '' -f ~/rogue && cd /var/lib/mothership/fleet &&
> git reset -q --hard HEAD~ && printf '\n# rogue\n%s\n' "$(cut -d' ' -f1,2 ~/rogue.pub)" >> signers.txt &&
> git -c gpg.format=ssh -c user.signingkey=$HOME/rogue commit -S -qam "Let me in" &&
> git push -qf
$ sudo mothership scan | grep refused
    \+ [0-9a-f]{12} Let me in +refused, \d{4}-\d\d-\d\d, claims to be by Me +a key the fleet doesn't list signed it: sudo mothership commit asks whether to vouch for it or drop it (re)
$ echo d | sudo mothership commit "On top" | tail -1
Nothing to commit.
$ cd /var/lib/mothership/fleet && git fetch -q && git log --format=%s origin/main | grep -c "Let me in"; git status -sb | head -1
0
## main...origin/main
```

The agent's work on a branch applies once I bring it in and vouch for it.

```console
$ cd /var/lib/mothership/fleet && git checkout -q -b agent &&
> sed -i 's/^  "gamma",$/  "gamma",\n  "zeta",/' types/workstation/system.toml &&
> git commit -qam "Add zeta" && git checkout -q main && git merge -q agent &&
> printf 'v\ny\n' | sudo mothership commit "Vouch for the agent's zeta" | tail -1
Committed [0-9a-f]{12} Vouch for the agent's zeta (re)
$ echo y | sudo mothership apply > ~/out; head -3 ~/out
Coming from the fleet · 1
  Packages
    + zeta  install
```

Root never runs git in my checkout: a hook the agent plants there runs as me.

```console
$ cd /var/lib/mothership/fleet && git config core.fsmonitor 'touch /tmp/ran-by-$(id -un)' &&
> touch nothing && echo y | sudo mothership commit "Add an empty file" &>/dev/null && echo y | sudo mothership apply &> /dev/null
$ ls /tmp | grep ran-by
ran-by-me
```

Once this machine signs only for another type, review refuses each answer that would change a
file it can't sign, and leaves revert and pass.

```console
$ cd /var/lib/mothership/fleet && git config --unset core.fsmonitor &&
> sed -i '/^ssh-ed25519/s/$/ types\/server/' signers.txt &&
> echo y | sudo mothership commit "Sign only for servers" >/dev/null && echo y | sudo mothership apply &> /dev/null
$ echo "new" | sudo install -m 644 /dev/stdin /etc/scoped.conf
$ printf 'a\np\n' | sudo mothership review | grep -A1 "can't sign"
  this machine can't sign that, so answer it from one that can: machine can't sign a change to types/workstation/etc/scoped.conf
  › p
$ git -C /var/lib/mothership/fleet log -1 --format=%s
Sign only for servers
```

A machine can't list a key a second time, with folders of its choosing: a machine checking a
commit would then find either line for that key.

```console
$ cd /var/lib/mothership/fleet && git reset -q --hard &&
> printf '\n# trap\n%s types/server/users\n' "$(cat /tmp/signing.pub)" >> signers.txt &&
> echo y | sudo mothership commit "Add a trap" 2>&1 | tail -1
Error: the fleet's machines would refuse this commit, so nothing changed: signers.txt lists one key twice, as machine and trap
```

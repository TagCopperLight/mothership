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
    \+ [0-9a-f]{12} Add zeta +refused, by Me +nobody signed it: sign it again or drop it, then push (re)

1 to review     sudo mothership review
\d+ ignored (re)
$ echo n | sudo mothership apply | head -1
This machine refuses [0-9a-f]{12} "Add zeta" by Me, and what follows it: nobody signed it (re)
```

It also tries to sign itself in, with a key of its own.

```console
$ ssh-keygen -q -t ed25519 -N '' -f ~/rogue && cd /var/lib/mothership/fleet &&
> git reset -q --hard HEAD~ && printf '\n# rogue\n%s\n' "$(cut -d' ' -f1,2 ~/rogue.pub)" >> signers.txt &&
> git -c gpg.format=ssh -c user.signingkey=$HOME/rogue commit -S -qam "Let me in" &&
> git push -qf
$ sudo mothership scan | grep refused
    \+ [0-9a-f]{12} Let me in +refused, by Me +a key the fleet doesn't list signed it: sign it again or drop it, then push (re)
```

I drop it. The agent's work on a branch applies once I bring it in and commit it through
mothership.

```console
$ cd /var/lib/mothership/fleet && git reset -q --hard HEAD~ && git push -qf &&
> git checkout -q -b agent && sed -i 's/^  "gamma",$/  "gamma",\n  "zeta",/' types/workstation/system.toml &&
> git commit -qam "Add zeta" && git checkout -q main && git merge --squash -q agent >/dev/null &&
> echo y | sudo mothership commit "Add the agent's zeta" >/dev/null
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

Once this machine signs only for another type, a review here would change files it can't sign.
It stops before signing, and pushes nothing.

```console
$ cd /var/lib/mothership/fleet && git config --unset core.fsmonitor &&
> sed -i '/^ssh-ed25519/s/$/ types\/server/' signers.txt &&
> echo y | sudo mothership commit "Sign only for servers" >/dev/null && echo y | sudo mothership apply &> /dev/null
$ echo "new" | sudo install -m 644 /dev/stdin /etc/scoped.conf
$ printf 'a\ny\n' | sudo mothership review 2>&1 | tail -1
Error: the fleet's machines would refuse this commit, so nothing changed: machine can't sign a change to types/workstation/etc/scoped.conf
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

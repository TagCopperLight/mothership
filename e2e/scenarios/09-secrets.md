# Secrets go into the fleet encrypted

The machine follows its fleet. `key` makes root's key and mine. Root's opens the secrets of
`/etc`, mine those of my home, and each goes into its own list. This machine is the first of its
type, so it trusts its own keys.

```console
$ echo y | sudo mothership apply &>/dev/null
$ sudo mothership key | sed 's/age1[a-z0-9]*/<key>/'
root    <key>
me      <key>
Add them from a machine that opens the secrets, or here for the first one:
  sudo mothership trust workstation/root <name> <key>
  sudo mothership trust workstation/me <name> <key>
$ sudo stat -c '%a %U' /etc/mothership/key.txt ~/.config/mothership/key.txt
600 root
600 me
$ sudo mothership trust workstation/root laptop "$(sudo grep -o 'age1.*' /etc/mothership/key.txt)"
laptop opens the 0 secrets of workstation/root now. Commit and push the fleet.
$ sudo mothership trust workstation/me laptop "$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
laptop opens the 0 secrets of workstation/me now. Commit and push the fleet.
$ sudo mothership trust workstation/dev laptop "$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
Error: workstation has no user dev
[exit 1]
```

I add an SSH key, a Wi-Fi password only root can read, and a token anyone can read. The scan
marks what only its owner can read, and misses the token.

```console
$ mkdir -p ~/.ssh ~/.config/app &&
> echo "SSH-SECRET" > ~/.ssh/id_test && chmod 600 ~/.ssh/id_test &&
> echo "TOKEN-SECRET" > ~/.config/app/token
$ echo "psk=WIFI-SECRET" | sudo install -m 600 /dev/stdin /etc/wifi.conf
$ sudo mothership scan
/etc (1)
  /etc/wifi.conf  from no package  [secret]

Untracked config in ~, me (2)
  ~/.config/app  not in the fleet
  ~/.ssh         not in the fleet  [secret]

3 differences, \d+ ignored (re)
```

Review adds the marked ones encrypted, and I answer **secret** for the token. The fleet holds no
secret in clear.

```console
$ printf 'a\ns\ny\na\ny\n' | sudo mothership review
[1/3] /etc
/etc/wifi.conf  from no package  [secret]
  add: copy it into the type's etc/, encrypted
  revert: delete
  (a)dd (n)ot secret (r)evert (i)gnore (l)ater (p)ass (q)uit? a
[2/3] Untracked config in ~, me
~/.config/app  not in the fleet
    folder with 1 files
    token
  add: add it to chezmoi
  (a)dd (s)ecret (i)gnore (l)ater (p)ass (q)uit? s
  That adds 1 files. Go on [y/N]? y
[3/3] Untracked config in ~, me
~/.ssh  not in the fleet  [secret]
  add: add it to chezmoi, encrypted
  (a)dd (n)ot secret (i)gnore (l)ater (p)ass (q)uit? a
  That adds 1 files. Go on [y/N]? y
3 added, 0 reverted, 0 ignored, 0 put off for later, 0 passed
Check the changes with `git -C /var/lib/mothership/fleet diff`, then commit.
$ cd /var/lib/mothership/fleet && find dot_ssh dot_config types/workstation/etc/wifi.conf.age | sort
dot_config
dot_config/app
dot_config/app/encrypted_token.age
dot_ssh
dot_ssh/encrypted_private_id_test.age
types/workstation/etc/wifi.conf.age
$ grep -r SECRET /var/lib/mothership/fleet
[exit 1]
$ grep -c wifi /var/lib/mothership/fleet/types/workstation/system.toml
0
[exit 1]
$ cd /var/lib/mothership/fleet && git add -A && git commit -qm "Add the secrets"
```

An editor keeps its settings for its owner only, though they hold no secret. Review lets me say
so, and the scan shows the folder as an ordinary one from then on.

```console
$ mkdir -p ~/.config/editor && echo "theme = dark" > ~/.config/editor/settings &&
> chmod 600 ~/.config/editor/settings
$ printf 'n\n' | sudo mothership review
[1/1] Untracked config in ~, me
~/.config/editor  not in the fleet  [secret]
  add: add it to chezmoi, encrypted
  (a)dd (n)ot secret (i)gnore (l)ater (p)ass (q)uit? n
0 added, 0 reverted, 0 ignored, 0 put off for later, 0 passed
1 marked not secret, which the next review shows as ordinary files
Check the changes with `git -C /var/lib/mothership/fleet diff`, then commit.
$ cat /var/lib/mothership/fleet/types/workstation/users/me/not-secret.txt
~/.config/editor
$ sudo mothership scan
Untracked config in ~, me (1)
  ~/.config/editor  not in the fleet

1 differences, \d+ ignored (re)
$ rm -r ~/.config/editor
```

A secret the fleet already holds in clear shows up too. Review encrypts it and reminds me to
change it.

```console
$ echo "machine example.com login me" > ~/.netrc && chmod 600 ~/.netrc && chezmoi add ~/.netrc
$ printf 'a\n' | sudo mothership review
[1/1] Files chezmoi tracks, me
~/.netrc  in the fleet in clear  [secret]
  add: copy this machine's version into chezmoi, encrypted
  (a)dd (n)ot secret (i)gnore (l)ater (p)ass (q)uit? a
  git history keeps the version in clear, so change it.
1 added, 0 reverted, 0 ignored, 0 put off for later, 0 passed
Check the changes with `git -C /var/lib/mothership/fleet diff`, then commit.
$ ls /var/lib/mothership/fleet | grep netrc
encrypted_private_dot_netrc.age
```

Apply and revert write secrets decrypted, for their owner only. Review never shows them.

```console
$ sudo rm /etc/wifi.conf && echo "MINE" > ~/.ssh/id_test
$ echo y | sudo mothership apply
/etc (1)
  /etc/wifi.conf  write the fleet's version

Left alone, since they were edited on this machine:
  me: ~/.ssh/id_test

Apply [y/N]? y
$ sudo stat -c '%a %U' /etc/wifi.conf && sudo cat /etc/wifi.conf
600 root
psk=WIFI-SECRET
$ printf 'r\n' | sudo mothership review
[1/1] Files chezmoi tracks, me
~/.ssh/id_test  edited here  [secret]
  add: copy this machine's version into chezmoi, encrypted
  revert: write the fleet's version
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? r
0 added, 1 reverted, 0 ignored, 0 put off for later, 0 passed
$ stat -c '%a' ~/.ssh/id_test && cat ~/.ssh/id_test
600
SSH-SECRET
```

A second machine has its own keys, which the fleet doesn't know yet. Scan and apply leave the
secrets alone.

```console
$ sudo mv /etc/mothership/key.txt /root/laptop.txt && mv ~/.config/mothership/key.txt ~/laptop.txt
$ sudo mothership key >/dev/null
$ sudo mothership scan
/etc (1)
  /etc/wifi.conf  this machine's key can't open it  [secret]

Files chezmoi tracks, me (3)
  ~/.config/app/token  this machine's key can't open it  [secret]
  ~/.netrc             this machine's key can't open it  [secret]
  ~/.ssh/id_test       this machine's key can't open it  [secret]

4 differences, \d+ ignored (re)
$ echo y | sudo mothership apply
Left alone, since this machine's keys can't open them:
  /etc/wifi.conf
  me: ~/.config/app/token
  me: ~/.netrc
  me: ~/.ssh/id_test

Nothing to apply.
```

Back on the laptop, a template takes a secret value from my `secrets/` folder, which chezmoi
writes nowhere. The laptop trusts the desktop in both lists, then the desktop retires the
laptop and lists what to change. A list never loses its last key.

```console
$ sudo mv /etc/mothership/key.txt /root/desktop.txt && sudo mv /root/laptop.txt /etc/mothership/key.txt
$ mv ~/.config/mothership/key.txt ~/desktop.txt && mv ~/laptop.txt ~/.config/mothership/key.txt
$ cd /var/lib/mothership/fleet && mkdir -p types/workstation/users/me/secrets &&
> echo "PROXY-SECRET" | chezmoi encrypt > types/workstation/users/me/secrets/proxy-key.age &&
> echo '{{ include "types/workstation/users/me/secrets/proxy-key.age" | decrypt | trim }}' > dot_proxy.tmpl
$ chezmoi apply --force ~/.proxy && cat ~/.proxy
PROXY-SECRET
$ sudo mothership trust workstation/root desktop "$(sudo grep -o 'age1.*' /root/desktop.txt)"
desktop opens the 1 secrets of workstation/root now. Commit and push the fleet.
$ sudo mothership trust workstation/me desktop "$(grep -o 'age1.*' ~/desktop.txt)"
desktop opens the 4 secrets of workstation/me now. Commit and push the fleet.
$ sudo mothership trust workstation/me desktop "$(grep -o 'age1.*' ~/desktop.txt)"
Error: workstation/me already holds desktop
[exit 1]
$ sudo mv /etc/mothership/key.txt /root/laptop.txt && sudo mv /root/desktop.txt /etc/mothership/key.txt
$ mv ~/.config/mothership/key.txt ~/laptop.txt && mv ~/desktop.txt ~/.config/mothership/key.txt
$ sudo mothership scan
0 differences, \d+ ignored (re)
$ sudo mothership retire workstation/me laptop
laptop is gone from workstation/me. Commit and push the fleet.
git history keeps the versions laptop opens, so change these:
  types/workstation/users/me/secrets/proxy-key.age
  ~/.config/app/token
  ~/.netrc
  ~/.ssh/id_test
$ sudo mothership retire workstation/root laptop
laptop is gone from workstation/root. Commit and push the fleet.
git history keeps the versions laptop opens, so change these:
  /etc/wifi.conf
$ sudo mothership retire workstation/root desktop
Error: desktop is the last key of workstation/root: trust another one first
[exit 1]
$ sed 's/age1[a-z0-9]*/<key>/' /var/lib/mothership/fleet/types/workstation/users/me/recipients.txt
# desktop
<key>
$ chezmoi execute-template < /var/lib/mothership/fleet/dot_proxy.tmpl
PROXY-SECRET
$ find /var/lib/mothership/fleet -not -user me | wc -l
0
```

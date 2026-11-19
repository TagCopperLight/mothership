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
Changed on this machine · 3
  Files
    + /etc/wifi.conf  new  secret
    + ~/.config/app   new  1 file
    + ~/.ssh          new  secret  1 file

3 to review     sudo mothership review
The fleet has 2 changes not committed  git -C /var/lib/mothership/fleet status
\d+ ignored (re)
```

Review adds the marked ones encrypted, and I answer **secret** for the token. The fleet holds no
secret in clear.

```console
$ printf 'a\ns\ny\na\ny\n' | sudo mothership review
[1/3] Files
    + /etc/wifi.conf  new  secret
  a  add         copy it into the type's etc/, encrypted
  n  not secret  list it in not-secret.txt, and add it later in clear
  r  revert      delete
  i ignore · l later · p pass · q quit
  › a
[2/3] Files
    + ~/.config/app  new  1 file
      token
  a  add         add it to chezmoi
  s  secret      add it to chezmoi, encrypted
  i ignore · l later · p pass · q quit
  › s
  That adds 1 file. Go on [y/N]? y
[3/3] Files
    + ~/.ssh  new  secret  1 file
  a  add         add it to chezmoi, encrypted
  n  not secret  list it in not-secret.txt, and add it later in clear
  i ignore · l later · p pass · q quit
  › a
  That adds 1 file. Go on [y/N]? y
3 added
Check the changes with git -C /var/lib/mothership/fleet diff, then commit.
$ cd /var/lib/mothership/fleet && find homes/me/dot_ssh homes/me/dot_config types/workstation/etc/wifi.conf.age | sort
homes/me/dot_config
homes/me/dot_config/app
homes/me/dot_config/app/encrypted_token.age
homes/me/dot_ssh
homes/me/dot_ssh/encrypted_private_id_test.age
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
[1/1] Files
    + ~/.config/editor  new  secret  1 file
  a  add         add it to chezmoi, encrypted
  n  not secret  list it in not-secret.txt, and add it later in clear
  i ignore · l later · p pass · q quit
  › n
1 marked not secret, which the next review shows as ordinary files
Check the changes with git -C /var/lib/mothership/fleet diff, then commit.
$ cat /var/lib/mothership/fleet/types/workstation/users/me/not-secret.txt
~/.config/editor
$ sudo mothership scan
Changed on this machine · 1
  Files
    + ~/.config/editor  new  1 file

1 to review     sudo mothership review
The fleet has 1 change not committed and 1 commit not pushed  git -C /var/lib/mothership/fleet status
\d+ ignored (re)
$ rm -r ~/.config/editor
```

A secret the fleet already holds in clear shows up too. Review encrypts it and reminds me to
change it.

```console
$ echo "machine example.com login me" > ~/.netrc && chmod 600 ~/.netrc && chezmoi add ~/.netrc
$ printf 'a\n' | sudo mothership review
[1/1] Files
    ! ~/.netrc  secret in clear  git history keeps it: change it once encrypted
  a  add         copy this machine's version into chezmoi, encrypted
  n  not secret  list it in not-secret.txt, and add it later in clear
  i ignore · l later · p pass · q quit
  › a
  git history keeps the version in clear, so change it.
1 added
Check the changes with git -C /var/lib/mothership/fleet diff, then commit.
$ ls /var/lib/mothership/fleet/homes/me | grep netrc
encrypted_private_dot_netrc.age
```

Apply and revert write secrets decrypted, for their owner only. Review never shows them.

```console
$ sudo rm /etc/wifi.conf && echo "MINE" > ~/.ssh/id_test
$ echo y | sudo mothership apply
Coming from the fleet · 1
  Files
    + /etc/wifi.conf  create  secret

Left alone, since they were edited on this machine:
  Files
    ~ ~/.ssh/id_test  edited  secret

1 changed on this machine, for sudo mothership review
Apply [y/N]? y
$ sudo stat -c '%a %U' /etc/wifi.conf && sudo cat /etc/wifi.conf
600 root
psk=WIFI-SECRET
$ printf 'r\n' | sudo mothership review
[1/1] Files
    ~ ~/.ssh/id_test  edited  secret
  a  add         copy this machine's version into chezmoi, encrypted
  r  revert      write the fleet's version
  i ignore · l later · p pass · q quit
  › r
1 reverted
$ stat -c '%a' ~/.ssh/id_test && cat ~/.ssh/id_test
600
SSH-SECRET
```

A secret of `/etc` edited on the machine stays one the fleet holds: review doesn't ask whether it
is one.

```console
$ echo "psk=OTHER" | sudo install -m 600 /dev/stdin /etc/wifi.conf
$ printf 'r\n' | sudo mothership review
[1/1] Files
    ~ /etc/wifi.conf  edited  secret
  a  add         copy it into the type's etc/, encrypted
  r  revert      write the fleet's version
  i ignore · l later · p pass · q quit
  › r
1 reverted
$ sudo cat /etc/wifi.conf
psk=WIFI-SECRET
```

A second machine has its own keys, which the fleet doesn't know yet. Scan and apply leave the
secrets alone.

```console
$ sudo mv /etc/mothership/key.txt /root/laptop.txt && mv ~/.config/mothership/key.txt ~/laptop.txt
$ sudo mothership key >/dev/null
$ sudo mothership scan
Needs attention · 4
  Files
    ? /etc/wifi.conf       locked  this machine's key isn't on its list
    ? ~/.config/app/token  locked  this machine's key isn't on its list
    ? ~/.netrc             locked  this machine's key isn't on its list
    ? ~/.ssh/id_test       locked  this machine's key isn't on its list

4 to review     sudo mothership review
The fleet has 2 changes not committed and 1 commit not pushed  git -C /var/lib/mothership/fleet status
\d+ ignored (re)
$ echo y | sudo mothership apply
Left alone, since this machine's key can't open them:
  Files
    ? /etc/wifi.conf       locked  this machine's key isn't on its list
    ? ~/.config/app/token  locked  this machine's key isn't on its list
    ? ~/.netrc             locked  this machine's key isn't on its list
    ? ~/.ssh/id_test       locked  this machine's key isn't on its list

4 changed on this machine, for sudo mothership review
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
> echo '{{ include (joinPath .fleet "types/workstation/users/me/secrets/proxy-key.age") | decrypt | trim }}' > homes/me/dot_proxy.tmpl
$ chezmoi apply --force ~/.proxy && cat ~/.proxy
PROXY-SECRET
$ sudo mothership trust workstation/root desktop "$(sudo grep -o 'age1.*' /root/desktop.txt)"
desktop opens the 1 secrets of workstation/root now. Commit and push the fleet.
$ sudo mothership trust workstation/me desktop "$(grep -o 'age1.*' ~/desktop.txt)"
desktop opens the 4 secrets of workstation/me now. Commit and push the fleet.
$ sudo mothership trust workstation/me desktop "$(grep -o 'age1.*' ~/desktop.txt)"
Error: workstation/me already holds this key, as desktop
[exit 1]
$ sudo mv /etc/mothership/key.txt /root/laptop.txt && sudo mv /root/desktop.txt /etc/mothership/key.txt
$ mv ~/.config/mothership/key.txt ~/laptop.txt && mv ~/desktop.txt ~/.config/mothership/key.txt
$ sudo mothership scan
✓ Nothing differs from the workstation type
The fleet has 9 changes not committed and 1 commit not pushed  git -C /var/lib/mothership/fleet status
\d+ ignored (re)
$ sudo mothership retire workstation/me laptop
laptop is gone from workstation/me. Commit and push the fleet.
git history keeps the versions laptop opens, and that machine may keep copies, so change these:
  types/workstation/users/me/secrets/proxy-key.age
  ~/.config/app/token
  ~/.netrc
  ~/.ssh/id_test
$ sudo mothership retire workstation/root laptop
laptop is gone from workstation/root. Commit and push the fleet.
git history keeps the versions laptop opens, and that machine may keep copies, so change these:
  /etc/wifi.conf
$ sudo mothership retire workstation/root desktop
Error: desktop is the last key of workstation/root: trust another one first
[exit 1]
$ sed 's/age1[a-z0-9]*/<key>/' /var/lib/mothership/fleet/types/workstation/users/me/recipients.txt
# desktop
<key>
$ chezmoi execute-template < /var/lib/mothership/fleet/homes/me/dot_proxy.tmpl
PROXY-SECRET
$ find /var/lib/mothership/fleet -not -user me | wc -l
0
```

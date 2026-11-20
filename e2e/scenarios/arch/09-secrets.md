# Secrets go into the fleet encrypted

The machine follows its fleet. `key` makes root's key and mine. Root's opens the secrets of
`/etc`, mine those of my home, and each goes into its own list. This machine already signs for
the fleet, so it trusts its own keys, and commits the lists.

```console
$ echo y | sudo mothership apply &>/dev/null
$ sudo mothership key | sed 's/age1[a-z0-9]*/<key>/g'
root     <key>
me       <key>
signing  ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBiaU3yvKl1XfZP1NPwoqvUpk/NKEmD0N2A069AmOCrB
Run this on a machine that signs for the fleet:
  sudo mothership trust machine workstation root=<key> me=<key>
$ sudo stat -c '%a %U' /etc/mothership/key.txt ~/.config/mothership/key.txt
600 root
600 me
$ sudo mothership trust laptop workstation root="$(sudo grep -o 'age1.*' /etc/mothership/key.txt)" \
> me="$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
laptop opens the 0 secrets of workstation/root now.
laptop opens the 0 secrets of workstation/me now.
Committed [0-9a-f]{12} Trust laptop as workstation (re)
On laptop, type this code: [0-9a-f]{12} (re)
$ sudo mothership trust laptop workstation dev="$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
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
\d+ ignored (re)
```

Review adds the marked ones encrypted, and I answer **secret** for the token. The fleet holds no
secret in clear.

```console
$ printf 'a\ns\ny\na\ny\n' | sudo mothership review
[1/3] Files
    + /etc/wifi.conf  new  secret
      psk=WIFI-SECRET
  a  add         copy it into the type's etc/, encrypted
  n  not secret  list it in not-secret.txt, and add it later in clear
  r  revert      delete
  i ignore · l later · p pass · q quit
  › a
[2/3] Files
    + ~/.config/app  new  1 file
      ── token
      TOKEN-SECRET
  a  add         add it to chezmoi
  s  secret      add it to chezmoi, encrypted
  i ignore · l later · p pass · q quit
  › s
  That adds 1 file. Go on [y/N]? y
[3/3] Files
    + ~/.ssh  new  secret  1 file
      ── id_test
      SSH-SECRET
  a  add         add it to chezmoi, encrypted
  n  not secret  list it in not-secret.txt, and add it later in clear
  i ignore · l later · p pass · q quit
  › a
  That adds 1 file. Go on [y/N]? y
3 added
Committed [0-9a-f]{12} Review on machine: 3 added (re)
$ stat -c '%a' ~/.config/app/token
600
$ cd /var/lib/mothership/fleet && find types/workstation/users/me/home/dot_ssh types/workstation/users/me/home/dot_config types/workstation/etc/wifi.conf.age | sort
types/workstation/etc/wifi.conf.age
types/workstation/users/me/home/dot_config
types/workstation/users/me/home/dot_config/app
types/workstation/users/me/home/dot_config/app/encrypted_private_token.age
types/workstation/users/me/home/dot_ssh
types/workstation/users/me/home/dot_ssh/encrypted_private_id_test.age
$ grep -r SECRET /var/lib/mothership/fleet
[exit 1]
$ grep -c wifi /var/lib/mothership/fleet/types/workstation/system.toml
0
[exit 1]
```

An editor keeps its settings for its owner only, though they hold no secret. Review lets me say
so, and the scan shows the folder as an ordinary one from then on.

```console
$ mkdir -p ~/.config/editor && echo "theme = dark" > ~/.config/editor/settings &&
> chmod 600 ~/.config/editor/settings
$ printf 'n\n' | sudo mothership review
[1/1] Files
    + ~/.config/editor  new  secret  1 file
      ── settings
      theme = dark
  a  add         add it to chezmoi, encrypted
  n  not secret  list it in not-secret.txt, and add it later in clear
  i ignore · l later · p pass · q quit
  › n
1 marked not secret, which the next review shows as ordinary files
Committed [0-9a-f]{12} Review on machine: 1 marked not secret (re)
$ cat /var/lib/mothership/fleet/types/workstation/users/me/not-secret.txt
~/.config/editor
$ sudo mothership scan
Changed on this machine · 1
  Files
    + ~/.config/editor  new  1 file

1 to review     sudo mothership review
\d+ ignored (re)
$ rm -r ~/.config/editor
```

A secret the fleet already holds in clear shows up too. Review encrypts it and reminds me to
change it.

```console
$ echo "machine example.com login me" > ~/.netrc && chmod 600 ~/.netrc &&
> cd /var/lib/mothership/fleet && cp ~/.netrc types/workstation/users/me/home/private_dot_netrc &&
> git add -A && git commit -S -qm "Add .netrc" && sudo mothership scan >/dev/null
$ printf 'a\n' | sudo mothership review
[1/1] Files
    ! ~/.netrc  secret in clear  git history keeps it: change it once encrypted
      machine example.com login me
  a  add         copy this machine's version into chezmoi, encrypted
  n  not secret  list it in not-secret.txt, and add it later in clear
  i ignore · l later · p pass · q quit
  › a
  git history keeps the version in clear, so change it.
1 added
Committed [0-9a-f]{12} Review on machine: 1 added (re)
$ ls /var/lib/mothership/fleet/types/workstation/users/me/home | grep netrc
encrypted_private_dot_netrc.age
```

Apply and revert write secrets decrypted, for their owner only. Review shows them decrypted.

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
      @@ -1 +1 @@
      -SSH-SECRET
      +MINE
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
      @@ -1 +1 @@
      -psk=WIFI-SECRET
      +psk=OTHER
  a  add         copy it into the type's etc/, encrypted
  r  revert      write the fleet's version
  i ignore · l later · p pass · q quit
  › r
1 reverted
$ sudo cat /etc/wifi.conf
psk=WIFI-SECRET
```

A second machine has its own keys, which the fleet doesn't know yet. Scan says what to run where
the secrets open, and apply leaves them alone.

```console
$ sudo mv /etc/mothership/key.txt /root/laptop.txt && mv ~/.config/mothership/key.txt ~/laptop.txt
$ sudo mothership key >/dev/null
$ sudo mothership scan
Needs attention · 5
  Files
    ? /etc/wifi.conf       locked  this machine's key isn't on its list
    ? ~/.config/app/token  locked  this machine's key isn't on its list
    ? ~/.netrc             locked  this machine's key isn't on its list
    ? ~/.ssh/id_test       locked  this machine's key isn't on its list
  Keys
    \+ machine +not trusted +run `sudo mothership trust machine workstation root=age1[a-z0-9]+ me=age1[a-z0-9]+` on a machine that signs for the fleet (re)

5 to review     sudo mothership review
\d+ ignored (re)
$ echo y | sudo mothership apply
Left alone, since this machine's key can't open them:
  Files
    ? /etc/wifi.conf       locked  this machine's key isn't on its list
    ? ~/.config/app/token  locked  this machine's key isn't on its list
    ? ~/.netrc             locked  this machine's key isn't on its list
    ? ~/.ssh/id_test       locked  this machine's key isn't on its list

5 changed on this machine, for sudo mothership review
Nothing to apply.
```

Back on the laptop, a template takes a secret value from my home's `.secrets/` folder, which
chezmoi writes nowhere. The laptop trusts the desktop in both lists, then the desktop retires the
laptop and lists what to change.

```console
$ sudo mv /etc/mothership/key.txt /root/desktop.txt && sudo mv /root/laptop.txt /etc/mothership/key.txt
$ mv ~/.config/mothership/key.txt ~/desktop.txt && mv ~/laptop.txt ~/.config/mothership/key.txt
$ cd /var/lib/mothership/fleet/types/workstation/users/me/home && mkdir .secrets &&
> echo "PROXY-SECRET" | chezmoi encrypt > .secrets/proxy-key.age &&
> echo '{{ include ".secrets/proxy-key.age" | decrypt | trim }}' > dot_proxy.tmpl &&
> git add -A && git commit -S -qm "Read the proxy key"
$ echo y | sudo mothership apply >/dev/null && cat ~/.proxy
PROXY-SECRET
$ printf 'y\ny\n' | sudo mothership trust desktop workstation root="$(sudo grep -o 'age1.*' /root/desktop.txt)" \
> me="$(grep -o 'age1.*' ~/desktop.txt)"
Re-encrypting for workstation/root:
  /etc/wifi.conf  workstation/root
Go on [y/N]? y
desktop opens the 1 secrets of workstation/root now.
Re-encrypting for workstation/me:
  .secrets/proxy-key.age  workstation/me
  ~/.config/app/token     workstation/me
  ~/.netrc                workstation/me
  ~/.ssh/id_test          workstation/me
Go on [y/N]? y
desktop opens the 4 secrets of workstation/me now.
Committed [0-9a-f]{12} Trust desktop as workstation (re)
On desktop, type this code: [0-9a-f]{12} (re)
$ sudo mv /etc/mothership/key.txt /root/laptop.txt && sudo mv /root/desktop.txt /etc/mothership/key.txt
$ mv ~/.config/mothership/key.txt ~/laptop.txt && mv ~/desktop.txt ~/.config/mothership/key.txt
$ sudo mothership scan
✓ Nothing differs from the workstation type
\d+ ignored (re)
$ printf 'y\ny\n' | sudo mothership retire laptop
Re-encrypting for workstation/root:
  /etc/wifi.conf  workstation/root
Go on [y/N]? y
laptop is gone from workstation/root.
git history keeps the versions laptop opens, and that machine may keep copies, so change these:
  /etc/wifi.conf
Re-encrypting for workstation/me:
  .secrets/proxy-key.age  workstation/me
  ~/.config/app/token     workstation/me
  ~/.netrc                workstation/me
  ~/.ssh/id_test          workstation/me
Go on [y/N]? y
laptop is gone from workstation/me.
git history keeps the versions laptop opens, and that machine may keep copies, so change these:
  .secrets/proxy-key.age
  ~/.config/app/token
  ~/.netrc
  ~/.ssh/id_test
Committed [0-9a-f]{12} Retire laptop (re)
$ sed 's/age1[a-z0-9]*/<key>/' /var/lib/mothership/fleet/types/workstation/users/me/recipients.txt
# desktop
<key>
$ chezmoi execute-template < /var/lib/mothership/fleet/types/workstation/users/me/home/dot_proxy.tmpl
PROXY-SECRET
$ find /var/lib/mothership/fleet -not -user me | wc -l
0
```

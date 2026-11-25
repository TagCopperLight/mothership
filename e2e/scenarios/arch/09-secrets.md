# Secrets go into the fleet encrypted

The machine follows its fleet, and apply made root's key and mine. Root's opens the secrets of
`/etc`, mine those of my home, and both go into the machine's file in `machines/`. This machine
already signs for the fleet, so it trusts its own keys, and commits its file.

```console
$ echo y | sudo mothership apply &>/dev/null
$ sudo stat -c '%a %U' /etc/mothership/key.txt ~/.config/mothership/key.txt
600 root
600 me
$ yes | sudo mothership trust machine workstation root="$(sudo grep -o 'age1.*' /etc/mothership/key.txt)" \
> me="$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
machine opens the 0 secrets of server/root now.
machine opens the 0 secrets of server/me now.
machine opens the 0 secrets of workstation/root now.
machine opens the 0 secrets of workstation/me now.
Trust machine as workstation:
  M machines/machine.toml
Commit and push [y/N]? y
Committed [0-9a-f]{12} Trust machine as workstation (re)
On machine, type this code: [0-9a-f]{12} (re)
$ yes | sudo mothership trust machine workstation dev="$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
Error: machine lists keys for dev, and workstation has no such user
[exit 1]
```

I add an SSH key, a Wi-Fi password only root can read, and a token anyone can read. The scan
marks what only its owner can read, and misses the token.

```console
$ mkdir -p ~/.config/app &&
> echo "SSH-SECRET" > ~/.ssh/id_test && chmod 600 ~/.ssh/id_test &&
> echo "TOKEN-SECRET" > ~/.config/app/token
$ echo "psk=WIFI-SECRET" | sudo install -m 600 /dev/stdin /etc/wifi.conf
$ sudo mothership scan
Changed on this machine · 3
  Files
    + /etc/wifi.conf  new  secret
    + ~/.config/app   new  1 file
    + ~/.ssh/id_test  new  secret

3 to review     sudo mothership review
\d+ ignored (re)
```

I answer **secret** for the token, then change my mind at the last question: its mode stays as
it was.

```console
$ printf 'p\ns\ny\np\nn\n' | sudo mothership review | tail -7
1 added, 2 passed
Review on machine: 1 added:
  A types/workstation/users/me/home/dot_config/app/encrypted_private_token.age
Then on this machine:
  ~/.config/app: make it readable by its owner only
Commit and push [y/N]? n
Nothing changed.
$ stat -c '%a' ~/.config/app/token
644
```

Review offers the marked ones encrypted first. I answer **secret** for all three, the token too.
The fleet holds no secret in clear.

```console
$ printf 's\ns\ny\ns\ny\n' | sudo mothership review
[1/3] Files
    + /etc/wifi.conf  new  secret
      psk=WIFI-SECRET
  s  secret      copy it into the type's etc/, encrypted
  a  add         copy it into the type's etc/
  r  revert      delete
  i ignore · p pass · q quit
  › s
[2/3] Files
    + ~/.config/app  new  1 file
      ── token
      TOKEN-SECRET
  a  add         add it to chezmoi
  s  secret      add it to chezmoi, encrypted
  i ignore · p pass · q quit
  › s
  That adds 1 file. Go on [y/N]? y
[3/3] Files
    + ~/.ssh/id_test  new  secret
      SSH-SECRET
  s  secret      add it to chezmoi, encrypted
  a  add         add it to chezmoi
  i ignore · p pass · q quit
  › s
3 added
Review on machine: 3 added:
  A types/workstation/etc/wifi.conf.age
  A types/workstation/users/me/home/dot_config/app/encrypted_private_token.age
  A types/workstation/users/me/home/private_dot_ssh/encrypted_private_id_test.age
Then on this machine:
  ~/.config/app: make it readable by its owner only
Commit and push [y/N]? y
Committed [0-9a-f]{12} Review on machine: 3 added (re)
$ stat -c '%a' ~/.config/app/token
600
$ cd /var/lib/mothership/fleet && find types/workstation/users/me/home/private_dot_ssh types/workstation/users/me/home/dot_config types/workstation/etc/wifi.conf.age | sort
types/workstation/etc/wifi.conf.age
types/workstation/users/me/home/dot_config
types/workstation/users/me/home/dot_config/app
types/workstation/users/me/home/dot_config/app/encrypted_private_token.age
types/workstation/users/me/home/private_dot_ssh
types/workstation/users/me/home/private_dot_ssh/encrypted_private_id_test.age
$ grep -r SECRET /var/lib/mothership/fleet
[exit 1]
$ grep -c wifi /var/lib/mothership/fleet/types/workstation/system.toml
0
[exit 1]
```

An editor keeps its settings for its owner only, though they hold no secret. I add them in clear.

```console
$ mkdir -p ~/.config/editor && echo "theme = dark" > ~/.config/editor/settings &&
> chmod 600 ~/.config/editor/settings
$ printf 'a\ny\ny\n' | sudo mothership review
[1/1] Files
    + ~/.config/editor  new  secret  1 file
      ── settings
      theme = dark
  s  secret      add it to chezmoi, encrypted
  a  add         add it to chezmoi
  i ignore · p pass · q quit
  › a
  That adds 1 file. Go on [y/N]? y
1 added
Review on machine: 1 added:
  A types/workstation/users/me/home/dot_config/editor/private_settings
Commit and push [y/N]? y
Committed [0-9a-f]{12} Review on machine: 1 added (re)
```

The scan doesn't guess again about a file the fleet tracks in clear, until I paste a token into
it. Review then only offers to encrypt it, and reminds me to change the token.

```console
$ echo "theme = light" > ~/.config/editor/settings && sudo mothership scan
Changed on this machine · 1
  Files
    ~ ~/.config/editor/settings  edited

1 to review     sudo mothership review
\d+ ignored (re)
$ echo "token = ghp_$(head -c 36 /dev/zero | tr '\0' x)" >> ~/.config/editor/settings
$ printf 'a\ny\n' | sudo mothership review
[1/1] Files
    ~ ~/.config/editor/settings  edited  secret  holds what looks like a GitHub token
      @@ -1 +1,2 @@
      -theme = dark
      +theme = light
      +token = ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
  a  add         copy this machine's version into chezmoi, encrypted
  r  revert      write the fleet's version
  i ignore · p pass · q quit
  › a
  git history keeps the version in clear, so change it.
1 added
Review on machine: 1 added:
  A types/workstation/users/me/home/dot_config/editor/encrypted_private_settings.age
  D types/workstation/users/me/home/dot_config/editor/private_settings
Commit and push [y/N]? y
Committed [0-9a-f]{12} Review on machine: 1 added (re)
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
$ printf 'r\ny\n' | sudo mothership review
[1/1] Files
    ~ ~/.ssh/id_test  edited  secret
      @@ -1 +1 @@
      -SSH-SECRET
      +MINE
  a  add         copy this machine's version into chezmoi, encrypted
  r  revert      write the fleet's version
  i ignore · p pass · q quit
  › r
1 reverted
Then on this machine:
  ~/.ssh/id_test: write the fleet's version
Revert them [y/N]? y
$ stat -c '%a' ~/.ssh/id_test && cat ~/.ssh/id_test
600
SSH-SECRET
```

A secret of `/etc` edited on the machine stays one the fleet holds: review doesn't ask whether it
is one.

```console
$ echo "psk=OTHER" | sudo install -m 600 /dev/stdin /etc/wifi.conf
$ printf 'r\ny\n' | sudo mothership review
[1/1] Files
    ~ /etc/wifi.conf  edited  secret
      @@ -1 +1 @@
      -psk=WIFI-SECRET
      +psk=OTHER
  a  add         copy it into the type's etc/, encrypted
  r  revert      write the fleet's version
  i ignore · p pass · q quit
  › r
1 reverted
Then on this machine:
  /etc/wifi.conf: write the fleet's version
Revert them [y/N]? y
$ sudo cat /etc/wifi.conf
psk=WIFI-SECRET
```

A second machine, the desktop, has its own name and keys, which the fleet doesn't know yet. I play
it on this container: I set its keys aside and name it in its own `machine.toml`. Scan says what
to run where the secrets open, and apply leaves them alone.

```console
$ sudo mv /etc/mothership/key.txt /root/laptop.txt && sudo mv /etc/mothership/signing /root/laptop-signing &&
> mv ~/.config/mothership/key.txt ~/laptop.txt
$ printf 'name = "desktop"\ntype = "workstation"\n' | sudo tee /etc/mothership/machine.toml >/dev/null
$ echo y | sudo mothership apply &>/dev/null
$ sudo mothership scan
Needs attention · 5
  Files
    ? /etc/wifi.conf             locked  this machine's key isn't on its list
    ? ~/.config/app/token        locked  this machine's key isn't on its list
    ? ~/.config/editor/settings  locked  this machine's key isn't on its list
    ? ~/.ssh/id_test             locked  this machine's key isn't on its list
  Keys
    \+ desktop +not trusted +run `sudo mothership trust desktop workstation root=age1[a-z0-9]+ me=age1[a-z0-9]+ 'me=ssh-ed25519 [A-Za-z0-9+/]+' 'signing=ssh-ed25519 [A-Za-z0-9+/]+'` on a machine that signs for the fleet (re)

5 to review     sudo mothership review
\d+ ignored (re)
$ echo y | sudo mothership apply
Left alone, since this machine's key can't open them:
  Files
    ? /etc/wifi.conf             locked  this machine's key isn't on its list
    ? ~/.config/app/token        locked  this machine's key isn't on its list
    ? ~/.config/editor/settings  locked  this machine's key isn't on its list
    ? ~/.ssh/id_test             locked  this machine's key isn't on its list

5 changed on this machine, for sudo mothership review
Nothing to apply.
```

Back on the laptop, a template takes a secret value from my home's `.secrets/` folder, which
chezmoi writes nowhere. Commit refuses it until its name says only I may read the result. The
laptop trusts the desktop, which may sign everything, and every
secret its keys read opens for it.

```console
$ sudo mv /etc/mothership/key.txt /root/desktop.txt && sudo mv /etc/mothership/signing /root/desktop-signing &&
> sudo mv /root/laptop.txt /etc/mothership/key.txt && sudo mv /root/laptop-signing /etc/mothership/signing
$ mv ~/.config/mothership/key.txt ~/desktop.txt && mv ~/laptop.txt ~/.config/mothership/key.txt
$ printf 'name = "machine"\n' | sudo tee /etc/mothership/machine.toml >/dev/null
$ cd /var/lib/mothership/fleet/types/workstation/users/me/home && mkdir .secrets &&
> echo "PROXY-SECRET" | chezmoi encrypt > .secrets/proxy-key.age &&
> echo '{{ include ".secrets/proxy-key.age" | decrypt | trim }}' > dot_proxy.tmpl &&
> echo y | sudo mothership commit "Read the proxy key"
Error: machines couldn't load the fleet after this change, so nothing changed: loading the server type: types/workstation/users/me/home/dot_proxy.tmpl reads a secret, so only its owner may read it: add `private_` to its name
[exit 1]
$ cd /var/lib/mothership/fleet/types/workstation/users/me/home && mv dot_proxy.tmpl private_dot_proxy.tmpl &&
> echo y | sudo mothership commit "Read the proxy key" >/dev/null
$ echo y | sudo mothership apply >/dev/null && cat ~/.proxy && stat -c '%a' ~/.proxy
PROXY-SECRET
600
$ printf '*\ny\n' | sudo mothership trust desktop workstation root="$(sudo grep -o 'age1.*' /root/desktop.txt)" \
> me="$(grep -o 'age1.*' ~/desktop.txt)" me="$(cat ~/.ssh/id_ed25519.pub)" \
> signing="$(sudo ssh-keygen -y -f /root/desktop-signing)"
desktop may change: Enter for types/workstation, * for everything, or folders separated by spaces › *
Re-encrypting:
  /etc/wifi.conf             workstation/root
  .secrets/proxy-key.age     workstation/me
  ~/.config/app/token        workstation/me
  ~/.config/editor/settings  workstation/me
  ~/.ssh/id_test             workstation/me
desktop opens the 0 secrets of server/root now.
desktop opens the 0 secrets of server/me now.
desktop opens the 1 secrets of workstation/root now.
desktop opens the 4 secrets of workstation/me now.
workstation/me on desktop has an SSH key in the fleet now.
desktop signs for the fleet now.
Trust desktop as workstation:
  A machines/desktop.toml
  M types/workstation/etc/wifi.conf.age
  M types/workstation/users/me/home/.secrets/proxy-key.age
  M types/workstation/users/me/home/dot_config/app/encrypted_private_token.age
  M types/workstation/users/me/home/dot_config/editor/encrypted_private_settings.age
  M types/workstation/users/me/home/private_dot_ssh/encrypted_private_id_test.age
Commit and push [y/N]? y
Committed [0-9a-f]{12} Trust desktop as workstation (re)
On desktop, type this code: [0-9a-f]{12} (re)
```

On the desktop, everything opens. It retires the laptop and lists what to change.

```console
$ sudo mv /etc/mothership/key.txt /root/laptop.txt && sudo mv /etc/mothership/signing /root/laptop-signing &&
> sudo mv /root/desktop.txt /etc/mothership/key.txt && sudo mv /root/desktop-signing /etc/mothership/signing
$ mv ~/.config/mothership/key.txt ~/laptop.txt && mv ~/desktop.txt ~/.config/mothership/key.txt
$ printf 'name = "desktop"\n' | sudo tee /etc/mothership/machine.toml >/dev/null
$ sudo mothership scan
✓ Nothing differs from the workstation type
\d+ ignored (re)
$ yes | sudo mothership retire machine
machine signs nothing now.
machine can still open these secrets of workstation/root, so change them:
  /etc/wifi.conf
machine can still open these secrets of workstation/me, so change them:
  .secrets/proxy-key.age
  ~/.config/app/token
  ~/.config/editor/settings
  ~/.ssh/id_test
Retire machine:
  D machines/machine.toml
Commit and push [y/N]? y
Committed [0-9a-f]{12} Retire machine (re)
$ ls /var/lib/mothership/fleet/machines
desktop.toml
$ chezmoi execute-template < /var/lib/mothership/fleet/types/workstation/users/me/home/private_dot_proxy.tmpl
PROXY-SECRET
$ find /var/lib/mothership/fleet -not -user me | wc -l
0
```

# Secrets go into the fleet encrypted

The machine follows its fleet. Trusting a key needs this machine's own key, since chezmoi reads
it even to encrypt. It makes its key and trusts it, as the first machine of its type.

```console
$ echo y | sudo mothership apply &>/dev/null
$ mothership trust recovery age1hk92umraas504tfkt4mdn5w6xqqp49wedf3pk4za9jvr53x7y5uqf4vwl6
Error: this machine has no key yet: make it with `mothership key` first
[exit 1]
$ mothership key | sed 's/age1[a-z0-9]*/<key>/'
<key>
Add it from a machine that opens the secrets, or here for the first one:
  mothership trust <name> <key>
$ stat -c '%a' ~/.config/mothership/key.txt
600
$ mothership trust laptop "$(grep -o 'age1.*' ~/.config/mothership/key.txt)"
laptop opens the type's 0 secrets now. Commit and push the fleet.
```

I add an SSH key, a Wi-Fi password only root can read, and a token anyone can read. The scan
marks what only its owner can read, and misses the token.

```console
$ mkdir -p ~/.ssh ~/.config/app &&
> echo "SSH-SECRET" > ~/.ssh/id_test && chmod 600 ~/.ssh/id_test &&
> echo "TOKEN-SECRET" > ~/.config/app/token
$ echo "psk=WIFI-SECRET" | sudo install -m 600 /dev/stdin /etc/wifi.conf
$ sudo mothership scan
Untracked config in ~ (2)
  ~/.config/app  not in the fleet
  ~/.ssh         not in the fleet  [secret]

/etc (1)
  /etc/wifi.conf  from no package  [secret]

3 differences, \d+ ignored by ~/.local/share/chezmoi/types/workstation/ignore.txt (re)
```

Review adds the marked ones encrypted, and I answer **secret** for the token. The fleet holds no
secret in clear.

```console
$ printf 's\ny\na\ny\na\n' | sudo mothership review
[1/3] Untracked config in ~
~/.config/app  not in the fleet
    folder with 1 files
    token
  add: add it to chezmoi
  (a)dd (s)ecret (i)gnore (l)ater (p)ass (q)uit? s
  That adds 1 files. Go on [y/N]? y
[2/3] Untracked config in ~
~/.ssh  not in the fleet  [secret]
  add: add it to chezmoi, encrypted
  (a)dd (i)gnore (l)ater (p)ass (q)uit? a
  That adds 1 files. Go on [y/N]? y
[3/3] /etc
/etc/wifi.conf  from no package  [secret]
  add: copy it into the type's etc/, encrypted
  revert: delete
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? a
3 added, 0 reverted, 0 ignored, 0 put off for later, 0 passed
Check the changes with `git -C ~/.local/share/chezmoi diff`, then commit.
$ cd ~/.local/share/chezmoi && find dot_ssh dot_config types/workstation/etc/wifi.conf.age | sort
dot_config
dot_config/app
dot_config/app/encrypted_token.age
dot_ssh
dot_ssh/encrypted_private_id_test.age
types/workstation/etc/wifi.conf.age
$ grep -r SECRET ~/.local/share/chezmoi
[exit 1]
$ grep -c wifi ~/.local/share/chezmoi/types/workstation/system.toml
0
[exit 1]
$ cd ~/.local/share/chezmoi && git add -A && git commit -qm "Add the secrets"
```

A secret the fleet already holds in clear shows up too. Review encrypts it and reminds me to
change it.

```console
$ echo "machine example.com login me" > ~/.netrc && chmod 600 ~/.netrc && chezmoi add ~/.netrc
$ printf 'a\n' | sudo mothership review
[1/1] Files chezmoi tracks
~/.netrc  in the fleet in clear  [secret]
  add: copy this machine's version into chezmoi, encrypted
  (a)dd (i)gnore (l)ater (p)ass (q)uit? a
  git history keeps the version in clear, so change it.
1 added, 0 reverted, 0 ignored, 0 put off for later, 0 passed
Check the changes with `git -C ~/.local/share/chezmoi diff`, then commit.
$ ls ~/.local/share/chezmoi | grep netrc
encrypted_private_dot_netrc.age
```

Apply and revert write secrets decrypted, for their owner only. Review never shows them.

```console
$ sudo rm /etc/wifi.conf && echo "MINE" > ~/.ssh/id_test
$ echo y | sudo mothership apply
/etc (1)
  /etc/wifi.conf  write the fleet's version

Left alone, since they were edited on this machine:
  ~/.ssh/id_test

Apply [y/N]? y
$ sudo stat -c '%a %U' /etc/wifi.conf && sudo cat /etc/wifi.conf
600 root
psk=WIFI-SECRET
$ printf 'r\n' | sudo mothership review
[1/1] Files chezmoi tracks
~/.ssh/id_test  edited here  [secret]
  add: copy this machine's version into chezmoi, encrypted
  revert: write the fleet's version
  (a)dd (r)evert (i)gnore (l)ater (p)ass (q)uit? r
0 added, 1 reverted, 0 ignored, 0 put off for later, 0 passed
$ stat -c '%a' ~/.ssh/id_test && cat ~/.ssh/id_test
600
SSH-SECRET
```

A second machine has its own key, which the fleet doesn't know yet. Scan and apply leave the
secrets alone.

```console
$ mv ~/.config/mothership/key.txt ~/laptop.txt && mothership key >/dev/null
$ sudo mothership scan
Files chezmoi tracks (3)
  ~/.config/app/token  this machine's key can't open it  [secret]
  ~/.netrc             this machine's key can't open it  [secret]
  ~/.ssh/id_test       this machine's key can't open it  [secret]

/etc (1)
  /etc/wifi.conf  this machine's key can't open it  [secret]

4 differences, \d+ ignored by ~/.local/share/chezmoi/types/workstation/ignore.txt (re)
$ echo y | sudo mothership apply
Left alone, since this machine's key can't open them:
  /etc/wifi.conf
  ~/.config/app/token
  ~/.netrc
  ~/.ssh/id_test

Nothing to apply.
```

The laptop trusts it, then the desktop retires the laptop and lists what to change.

```console
$ mv ~/.config/mothership/key.txt ~/desktop.txt && mv ~/laptop.txt ~/.config/mothership/key.txt
$ mothership trust desktop "$(grep -o 'age1.*' ~/desktop.txt)"
desktop opens the type's 4 secrets now. Commit and push the fleet.
$ mothership trust desktop "$(grep -o 'age1.*' ~/desktop.txt)"
Error: recipients.txt already holds desktop
[exit 1]
$ mv ~/.config/mothership/key.txt ~/laptop.txt && mv ~/desktop.txt ~/.config/mothership/key.txt
$ sudo mothership scan
0 differences, \d+ ignored by ~/.local/share/chezmoi/types/workstation/ignore.txt (re)
$ mothership retire laptop
laptop is gone. Commit and push the fleet.
git history keeps the versions laptop opens, so change these:
  /etc/wifi.conf
  ~/.config/app/token
  ~/.netrc
  ~/.ssh/id_test
$ sed 's/age1[a-z0-9]*/<key>/' ~/.local/share/chezmoi/types/workstation/recipients.txt
# desktop
<key>
```

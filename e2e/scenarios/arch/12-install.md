# Install has the fleet trust a new machine's keys

The laptop's keys are in its file in `machines/`, and root's key guards a Wi-Fi password. Then
the laptop's keys move aside: this machine becomes a new one, `newbox`, with keys of its own.

```console
$ echo y | sudo mothership apply &>/dev/null
$ yes | sudo mothership trust machine workstation root="$(sudo grep -o 'age1.*' /etc/mothership/key.txt)" \
> me="$(grep -o 'age1.*' ~/.config/mothership/key.txt)" > /dev/null
$ echo "psk=WIFI-SECRET" | sudo install -m 600 /dev/stdin /etc/wifi.conf &&
> printf 's\ny\n' | sudo mothership review > /dev/null
$ sudo mv /etc/mothership/key.txt /root/laptop.txt && sudo mv /etc/mothership/signing /root/laptop-signing &&
> mv ~/.config/mothership/key.txt ~/laptop.txt &&
> printf 'name = "newbox"\ntype = "workstation"\n' | sudo tee /etc/mothership/machine.toml >/dev/null &&
> (echo y | sudo mothership apply &>/dev/null) &&
> sudo mv /etc/mothership/key.txt /root/new.txt && sudo mv /etc/mothership/signing /root/new-signing &&
> mv ~/.config/mothership/key.txt ~/new.txt
```

On the laptop, trust takes the new machine's keys, commits and pushes, and prints a code.

```console
$ sudo cp /root/laptop.txt /etc/mothership/key.txt && sudo cp /root/laptop-signing /etc/mothership/signing &&
> cp ~/laptop.txt ~/.config/mothership/key.txt &&
> printf 'name = "machine"\n' | sudo tee /etc/mothership/machine.toml >/dev/null &&
> printf '\ny\n' | sudo mothership trust newbox workstation root="$(sudo grep -o 'age1.*' /root/new.txt)" \
> me="$(grep -o 'age1.*' ~/new.txt)" me="$(cat ~/.ssh/id_ed25519.pub)" \
> signing="$(sudo ssh-keygen -y -f /root/new-signing)" | tail -1 > ~/code && cat ~/code
On newbox, type this code: [0-9a-f]{12} (re)
```

The new machine has checked no commit yet, so an attacker who can push could have rewritten the
fleet. Every command but install refuses to run, and install starts from the commit whose code I
type, and checks what follows it. A code it can't find, it asks again. Once the fleet holds the
machine's file, its `machine.toml` keeps only its name.

```console
$ sudo mv /root/new.txt /etc/mothership/key.txt && sudo mv /root/new-signing /etc/mothership/signing &&
> mv ~/new.txt ~/.config/mothership/key.txt &&
> printf 'name = "newbox"\ntype = "workstation"\n' | sudo tee /etc/mothership/machine.toml >/dev/null &&
> sudo rm -r /var/lib/mothership/fleet.git /var/lib/mothership/current /var/lib/mothership/checked
$ echo n | sudo mothership apply
Error: this machine trusts no commit of the fleet yet: run `sudo mothership install`
[exit 1]
$ printf 'c\nbadc0de\nc\n%s\n' "$(grep -o '[0-9a-f]*$' ~/code)" | sudo mothership install > ~/out; head -6 ~/out
This machine needs a fleet commit to trust first.
c type the code it prints · r type the recovery key › c
Code: badc0de
That code doesn't work: the fleet has no commit badc0de
This machine needs a fleet commit to trust first.
c type the code it prints · r type the recovery key › c
$ sudo cat /etc/wifi.conf /etc/mothership/machine.toml
psk=WIFI-SECRET
name = "newbox"
```

Without the laptop, I type the recovery key instead, and install trusts a spare machine's keys
itself, signed with the recovery key.

```console
$ sudo rm /etc/mothership/key.txt /etc/mothership/signing &&
> printf 'name = "spare"\ntype = "workstation"\n' | sudo tee /etc/mothership/machine.toml >/dev/null
$ printf 'r\n%s\n\ny\n' "$(grep AGE-SECRET ~/recovery.txt)" | sudo mothership install |
> sed 's/age1[a-z0-9]*/<key>/g; s/ssh-ed25519 [A-Za-z0-9+/]*/<ssh>/g'
Run this on a machine that signs for the fleet:
  sudo mothership trust spare workstation root=<key> me=<key> 'me=<ssh>' 'signing=<ssh>'
c type the code it prints · r type the recovery key › r
Recovery key: 
spare may change: Enter for types/workstation, * for everything, or folders separated by spaces › 
Re-encrypting:
  /etc/wifi.conf  workstation/root
spare opens the 1 secrets of workstation/root now.
spare opens the 0 secrets of workstation/me now.
workstation/me on spare has an SSH key in the fleet now.
spare signs for the fleet now.
Trust spare as workstation:
  A machines/spare.toml
  M types/workstation/etc/wifi.conf.age
Commit and push [y/N]? y
Committed [0-9a-f]{12} Trust spare as workstation (re)
...
$ ls /var/lib/mothership/fleet/machines
machine.toml
newbox.toml
spare.toml
$ git -C /var/lib/mothership/fleet log -1 --format=%s
Trust spare as workstation
$ sudo test -e /etc/sudoers.d/zz-mothership-install
[exit 1]
```

Only the recovery key the fleet names works. Here, root's new key stands in for another one.

```console
$ sudo rm /etc/mothership/key.txt && echo y | sudo mothership apply &>/dev/null
$ printf 'r\n%s\n' "$(sudo grep AGE-SECRET /etc/mothership/key.txt)" | sudo mothership install 2>&1 | tail -1
Error: that isn't the recovery key the fleet's mothership.toml names
```

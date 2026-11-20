# Install has the fleet trust a new machine's keys

The laptop's keys are in the fleet's lists, and root's list guards a Wi-Fi password. Then the
laptop's keys move aside: this machine becomes a new one, with keys of its own.

```console
$ echo y | sudo mothership apply &>/dev/null
$ sudo mothership key > /dev/null &&
> sudo mothership trust laptop workstation root="$(sudo grep -o 'age1.*' /etc/mothership/key.txt)" \
> me="$(grep -o 'age1.*' ~/.config/mothership/key.txt)" > /dev/null
$ echo "psk=WIFI-SECRET" | sudo install -m 600 /dev/stdin /etc/wifi.conf &&
> printf 'a\ny\n' | sudo mothership review > /dev/null
$ sudo mv /etc/mothership/key.txt /root/laptop.txt && mv ~/.config/mothership/key.txt ~/laptop.txt &&
> sudo mothership key > /dev/null &&
> sudo mv /etc/mothership/key.txt /root/new.txt && mv ~/.config/mothership/key.txt ~/new.txt
```

On the laptop, trust takes the new machine's keys, commits and pushes, and prints a code.

```console
$ sudo cp /root/laptop.txt /etc/mothership/key.txt && cp ~/laptop.txt ~/.config/mothership/key.txt &&
> printf 'y\n' | sudo mothership trust newbox workstation root="$(sudo grep -o 'age1.*' /root/new.txt)" \
> me="$(grep -o 'age1.*' ~/new.txt)" | tail -1 > ~/code && cat ~/code
On newbox, type this code: [0-9a-f]{12} (re)
```

The new machine has checked no commit yet, so an attacker who can push could have rewritten the
fleet. Install starts from the commit whose code I type, and checks what follows it.

```console
$ sudo mv /root/new.txt /etc/mothership/key.txt && mv ~/new.txt ~/.config/mothership/key.txt &&
> sudo rm -r /var/lib/mothership/fleet.git /var/lib/mothership/current /var/lib/mothership/checked
$ printf 'c\n%s\n' "$(grep -o '[0-9a-f]*$' ~/code)" | sudo mothership install > ~/out; head -3 ~/out
This machine needs a fleet commit to trust first.
c type the code it prints · r type the recovery key › c
Code: [0-9a-f]{12} (re)
$ sudo cat /etc/wifi.conf
psk=WIFI-SECRET
```

Without the laptop, I type the recovery key instead, and install trusts the new keys itself,
signed with the recovery key.

```console
$ sudo rm /etc/mothership/key.txt
$ printf 'r\n%s\ny\n' "$(grep AGE-SECRET ~/recovery.txt)" | sudo mothership install | sed 's/age1[a-z0-9]*/<key>/'
Run this on a machine that signs for the fleet:
  sudo mothership trust machine workstation root=<key>
c type the code it prints · r type the recovery key › r
Recovery key: 
Re-encrypting for workstation/root:
  /etc/wifi.conf  workstation/root
Go on [y/N]? y
machine opens the 1 secrets of workstation/root now.
Committed [0-9a-f]{12} Trust machine as workstation (re)
...
$ cd /var/lib/mothership/fleet && grep -c '^# ' types/workstation/recipients.txt types/workstation/users/me/recipients.txt
types/workstation/recipients.txt:3
types/workstation/users/me/recipients.txt:2
$ git -C /var/lib/mothership/fleet log -1 --format=%s
Trust machine as workstation
$ sudo test -e /etc/sudoers.d/zz-mothership-install
[exit 1]
```

Only the recovery key the fleet names works. Here, root's new key stands in for another one.

```console
$ sudo rm /etc/mothership/key.txt && sudo mothership key > /dev/null
$ printf 'r\n%s\n' "$(sudo grep AGE-SECRET /etc/mothership/key.txt)" | sudo mothership install 2>&1 | tail -1
Error: that isn't the recovery key the fleet's mothership.toml names
```

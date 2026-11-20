# Install has the fleet trust a new machine's keys

The laptop's keys are in the fleet's lists, and root's list guards a Wi-Fi password. This
machine is new: it has no key yet.

```console
$ echo y | sudo mothership apply &>/dev/null
$ sudo mothership key > /dev/null &&
> sudo mothership trust workstation/root laptop "$(sudo grep -o 'age1.*' /etc/mothership/key.txt)" > /dev/null &&
> sudo mothership trust workstation/me laptop "$(grep -o 'age1.*' ~/.config/mothership/key.txt)" > /dev/null
$ echo "psk=WIFI-SECRET" | sudo install -m 600 /dev/stdin /etc/wifi.conf &&
> printf 'a\ny\n' | sudo mothership review > /dev/null &&
> sudo rm /etc/mothership/key.txt ~/.config/mothership/key.txt
$ cd /var/lib/mothership/fleet && git add -A &&
> git -c user.name=Me -c user.email=me@example.com commit -qm "Trust the laptop"
```

Install makes the keys. My list guards no secret, so install trusts my key itself. Root's list
needs the laptop, or the recovery key, which I type instead.

```console
$ printf 'r\n%s\ny\n' "$(grep AGE-SECRET ~/recovery.txt)" | sudo mothership install | sed 's/age1[a-z0-9]*/<key>/'
machine opens the 0 secrets of workstation/me now. Commit and push the fleet.
The fleet doesn't trust this machine's keys yet. Run these, then commit and push:
  On laptop:
    sudo mothership trust workstation/root machine <key>
d done, pull the fleet · r type the recovery key › r
Recovery key: 
Re-encrypting for workstation/root:
  /etc/wifi.conf  workstation/root
...
$ cd /var/lib/mothership/fleet && grep -c '^# ' types/workstation/recipients.txt types/workstation/users/me/recipients.txt
types/workstation/recipients.txt:2
types/workstation/users/me/recipients.txt:2
$ sudo test -e /etc/sudoers.d/zz-mothership-install
[exit 1]
```

Only the recovery key the fleet names works. Here, root's new key stands in for another one.

```console
$ sudo rm /etc/mothership/key.txt && sudo mothership key > /dev/null
$ printf 'r\n%s\n' "$(sudo grep AGE-SECRET /etc/mothership/key.txt)" | sudo mothership install 2>&1 | tail -1
Error: that isn't the recovery key the fleet's mothership.toml names
```

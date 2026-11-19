# Install has the fleet trust a new machine's keys

The fleet's lists already hold the recovery key and the laptop's keys. This machine is new: it
has no key yet.

```console
$ echo y | sudo mothership apply &>/dev/null
$ sudo mothership key > /dev/null && sudo cp /etc/mothership/key.txt ~/recovery.txt &&
> sudo chown me ~/recovery.txt &&
> sudo mothership trust workstation/root recovery "$(grep -o 'age1.*' ~/recovery.txt)" > /dev/null &&
> sudo mothership trust workstation/me recovery "$(grep -o 'age1.*' ~/recovery.txt)" > /dev/null &&
> sudo rm /etc/mothership/key.txt ~/.config/mothership/key.txt
$ sudo mothership key > /dev/null &&
> sudo mothership trust workstation/root laptop "$(sudo grep -o 'age1.*' /etc/mothership/key.txt)" > /dev/null &&
> sudo mothership trust workstation/me laptop "$(grep -o 'age1.*' ~/.config/mothership/key.txt)" > /dev/null &&
> sudo rm /etc/mothership/key.txt ~/.config/mothership/key.txt
$ cd /var/lib/mothership/fleet && git add -A &&
> git -c user.name=Me -c user.email=me@example.com commit -qm "Trust the laptop"
```

Install makes the keys and says what to run on the laptop. I type the recovery key instead, and
install trusts both keys itself.

```console
$ printf 'r\n%s\n' "$(grep AGE-SECRET ~/recovery.txt)" | sudo mothership install | sed 's/age1[a-z0-9]*/<key>/'
The fleet doesn't trust this machine's keys yet. Run these, then commit and push:
  On laptop:
    sudo mothership trust workstation/root machine <key>
    sudo mothership trust workstation/me machine <key>
d done, pull the fleet · r type the recovery key › r
Recovery key: 
machine opens the 0 secrets of workstation/root now. Commit and push the fleet.
machine opens the 0 secrets of workstation/me now. Commit and push the fleet.
...
$ cd /var/lib/mothership/fleet && grep -c '^# ' types/workstation/recipients.txt types/workstation/users/me/recipients.txt
types/workstation/recipients.txt:3
types/workstation/users/me/recipients.txt:3
$ sudo test -e /etc/sudoers.d/zz-mothership-install
[exit 1]
```

A recovery key that isn't in the lists opens nothing. Here, root's new key stands in for one.

```console
$ sudo rm /etc/mothership/key.txt && sudo mothership key > /dev/null
$ printf 'r\n%s\n' "$(sudo grep AGE-SECRET /etc/mothership/key.txt)" | sudo mothership install 2>&1 | tail -1
Error: that key isn't in /var/lib/mothership/fleet/types/workstation/recipients.txt
```

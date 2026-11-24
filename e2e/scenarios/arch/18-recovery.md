# A new recovery key takes over every secret

Root's key guards a Wi-Fi password. I change the recovery key: mothership asks the old one,
makes the new one, encrypts every secret again and commits, signed with the old key.

```console
$ echo y | sudo mothership apply &>/dev/null
$ yes | sudo mothership trust machine workstation root="$(sudo grep -o 'age1.*' /etc/mothership/key.txt)" > /dev/null
$ echo "psk=WIFI-SECRET" | sudo install -m 600 /dev/stdin /etc/wifi.conf &&
> printf 's\ny\n' | sudo mothership review > /dev/null
$ printf '%s\nn\ny\n' "$(grep AGE-SECRET ~/recovery.txt)" | sudo mothership recovery > ~/out; cat ~/out | sed 's/AGE-SECRET-KEY-[A-Z0-9]*/<new key>/'
The fleet's recovery key now: 
The new recovery key:
n make a new one · t type the one you have › n
The recovery key opens every secret, signs any change, and nothing else keeps it. Write it on paper and in your password manager:
  <new key>
Re-encrypting:
  /etc/wifi.conf  workstation/root
Change the recovery key:
  M mothership.toml
  M types/workstation/etc/wifi.conf.age
Commit and push [y/N]? y
Committed [0-9a-f]{12} Change the recovery key (re)
The old recovery key opens what git history keeps, so change those secrets too.
$ sudo mothership scan | grep -c refused
0
[exit 1]
```

The old key opens nothing any more, and install takes only the new one, here for a spare machine
with keys of its own.

```console
$ sudo rm /etc/mothership/key.txt /etc/mothership/signing &&
> printf 'name = "spare"\ntype = "workstation"\n' | sudo tee /etc/mothership/machine.toml >/dev/null
$ printf 'r\n%s\n' "$(grep AGE-SECRET ~/recovery.txt)" | sudo mothership install 2>&1 | tail -1
Error: that isn't the recovery key the fleet's mothership.toml names
$ printf 'r\n%s\n\ny\n' "$(grep -o 'AGE-SECRET-KEY-[A-Z0-9]*' ~/out)" | sudo mothership install > /dev/null && sudo cat /etc/wifi.conf
psk=WIFI-SECRET
```

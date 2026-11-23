# Install reads the fleet over SSH, with the admin's own key

The fleet waits on an SSH server, and the machine has nothing of mothership yet. My SSH key
isn't on the server's list.

```console
$ git clone -q --bare /var/lib/mothership/fleet ~/fleet.git
$ sudo rm -r /var/lib/mothership /etc/mothership
$ sudo ssh-keygen -A > /dev/null && sudo mkdir -p /run/sshd && sudo /usr/sbin/sshd
$ ssh-keyscan -t ed25519 localhost 2>/dev/null | sudo install -m 644 /dev/stdin /etc/ssh/ssh_known_hosts
$ echo n | sudo mothership install --admin me --fleet me@localhost:fleet.git --type workstation --hostname box 2>&1 |
> sed 's/ssh-ed25519 [A-Za-z0-9+/]*/<key>/'
localhost doesn't let this machine's key in yet.
Let it in for the fleet's repository: <key>
Is it there now [y/N]? n
Error: install stopped before it could read the fleet
```

Once the server lets my key in, install clones over SSH, and the fleet's origin stays the SSH URL.

```console
$ cat ~/.ssh/id_ed25519.pub >> ~/.ssh/authorized_keys
$ printf 'r\n%s\ny\n\ny\n' "$(grep AGE-SECRET ~/recovery.txt)" |
> sudo mothership install --admin me --fleet me@localhost:fleet.git --type workstation --hostname box &> ~/out
$ grep '^Committed' ~/out
Committed [0-9a-f]{12} Trust box as workstation (re)
$ git -C /var/lib/mothership/fleet remote get-url origin
me@localhost:fleet.git
$ git -C ~/fleet.git log -1 --format=%s
Trust box as workstation
```

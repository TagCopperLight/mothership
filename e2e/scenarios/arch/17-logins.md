# A user lets in the SSH keys the fleet lists for the users it names

My `user.toml` lets in every user of the workstation type, and my home's `authorized_keys` lists
the keys mothership hands to chezmoi. The fleet already lists this machine's key for me.

```console
$ cd /var/lib/mothership/fleet/types/workstation/users/me &&
> echo 'ssh_from = ["workstation"]' >> user.toml && mkdir -p home/private_dot_ssh &&
> printf '{{ range .authorized_keys }}{{ . }}\n{{ end }}' > home/private_dot_ssh/authorized_keys.tmpl &&
> echo y | sudo mothership commit "Let workstation users in" >/dev/null
$ printf "y\ny\n" | sudo mothership apply &> /dev/null
$ cat ~/.ssh/authorized_keys
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPznprSpdY8b3HM0Sy/iwuhzpCwaaMA8rvhDULqPQV9P machine/me
$ sudo ssh-keygen -A > /dev/null && sudo mkdir -p /run/sshd && sudo /usr/sbin/sshd &&
> ssh-keyscan -t ed25519 localhost 2>/dev/null > ~/.ssh/known_hosts
$ ssh -o BatchMode=yes localhost echo logged in
logged in
```

Retiring the only machine whose key a user lets in would lock that user out, so retire refuses.

```console
$ yes | sudo mothership retire machine 2>&1 | tail -1
Error: machine stays: types/workstation/users/me/user.toml's ssh_from finds no SSH key in the fleet, so every login would be refused
```

A name the fleet lacks stops commit before it signs anything, as the fleet wouldn't load.

```console
$ cd /var/lib/mothership/fleet/types/workstation/users/me &&
> sed -i 's/"workstation"/"workstaton"/' user.toml && echo y | sudo mothership commit "Typo" 2>&1 | tail -1
Error: machines couldn't load the fleet after this change, so nothing changed: loading the workstation type: /var/lib/mothership/staging/types/workstation/users/me/user.toml's ssh_from names workstaton, and the fleet has no type workstaton
$ git -C /var/lib/mothership/fleet log -1 --format=%s
Let workstation users in
```

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

A name the fleet lacks stops mothership before it writes anything, and so does a list with no key
in it.

```console
$ cd /var/lib/mothership/fleet/types/workstation/users/me &&
> sed -i 's/"workstation"/"workstaton"/' user.toml && echo y | sudo mothership commit "Typo" >/dev/null
$ sudo mothership apply 2>&1 | tail -1
Error: /var/lib/mothership/current/types/workstation/users/me/user.toml's ssh_from names workstaton, and the fleet has no type workstaton
$ cat ~/.ssh/authorized_keys | wc -l
1
```

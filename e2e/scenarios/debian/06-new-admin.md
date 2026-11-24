# Install hands the fleet's clone to an admin it creates

A cloud image comes with its own user, and the fleet's admin doesn't exist yet. I add `ops` to the
server type, with a `.gitconfig` that names them, and pick it as the admin. The fleet sits in a
folder every user may write.

```console
$ sudo mkdir -p /var/lib/mothership/fleet/types/server/users/ops
$ printf 'shell = "/bin/bash"\ngroups = ["sudo"]\n' | sudo tee /var/lib/mothership/fleet/types/server/users/ops/user.toml > /dev/null
$ sudo mkdir /var/lib/mothership/fleet/types/server/users/ops/home
$ printf '[user]\n\tname = Ops\n\temail = ops@example.com\n' | sudo tee /var/lib/mothership/fleet/types/server/users/ops/home/dot_gitconfig > /dev/null
$ git -C /var/lib/mothership/fleet add -A && git -C /var/lib/mothership/fleet commit -qm "Add ops"
$ sudo git clone -q --bare /var/lib/mothership/fleet /opt/fleet.git && sudo chmod -R a+rwX /opt/fleet.git
$ sudo git config --system --add safe.directory '*'
$ sudo rm -r /var/lib/mothership /etc/mothership
$ id ops
id: 'ops': no such user
[exit 1]
```

Install clones the fleet as root, creates `ops`, asks for its password, and gives it the clone,
which `ops` then signs and pushes from. The commit's author comes from the fleet's `.gitconfig`
for `ops`, and the clone keeps no author of its own.

```console
$ printf 'ops-password\nops-password\nr\n%s\ny\n\ny\ny\n' "$(grep AGE-SECRET ~/recovery.txt)" |
> sudo mothership install --admin ops --fleet /opt/fleet.git --type server --hostname box > ~/out 2>&1
$ stat -c %U /var/lib/mothership/fleet
ops
$ git -C /opt/fleet.git log -1 --format='%an <%ae> %s'
Ops <ops@example.com> Trust box as server
$ git -C /var/lib/mothership/fleet config --local user.name
[exit 1]
```

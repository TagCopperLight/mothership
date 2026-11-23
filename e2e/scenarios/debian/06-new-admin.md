# Install hands the fleet's clone to an admin it creates

A cloud image comes with its own user, and the fleet's admin doesn't exist yet. I add `ops` to the
server type and pick it as the admin. The fleet sits in a folder every user may write.

```console
$ sudo mkdir -p /var/lib/mothership/fleet/types/server/users/ops
$ printf 'shell = "/bin/bash"\ngroups = ["sudo"]\n' | sudo tee /var/lib/mothership/fleet/types/server/users/ops/user.toml > /dev/null
$ git -C /var/lib/mothership/fleet add -A && git -C /var/lib/mothership/fleet commit -qm "Add ops"
$ sudo git clone -q --bare /var/lib/mothership/fleet /opt/fleet.git && sudo chmod -R a+rwX /opt/fleet.git
$ sudo git config --system --add safe.directory '*'
$ sudo rm -r /var/lib/mothership /etc/mothership
$ id ops
id: 'ops': no such user
[exit 1]
```

Install clones the fleet as root, creates `ops`, asks for its password and its name for commits,
and gives it the clone, which `ops` then signs and pushes from.

```console
$ printf 'ops-password\nops-password\nr\n%s\ny\n\nOps\nops@example.com\ny\ny\n' "$(grep AGE-SECRET ~/recovery.txt)" |
> sudo mothership install --admin ops --fleet /opt/fleet.git --type server --hostname box > ~/out 2>&1
$ stat -c %U /var/lib/mothership/fleet
ops
$ git -C /opt/fleet.git log -1 --format='%an %s'
Ops Trust box as server
```

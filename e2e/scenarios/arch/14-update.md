# mothership follows the version the fleet pins

The fleet pins the mothership this machine already runs, so apply goes on as usual.

```console
$ cd /var/lib/mothership/fleet && sha=$(sha256sum /usr/local/bin/mothership | cut -d' ' -f1) &&
> { printf 'version = "0.1.0"\n\n[sha256]\nx86_64 = "%s"\n\n' "$sha"; cat mothership.toml; } > pinned &&
> mv pinned mothership.toml &&
> git add mothership.toml && echo y | sudo mothership commit "Pin mothership" >/dev/null
$ echo n | sudo mothership apply | tail -1
Apply [y/N]? n
```

The fleet moves to another version. Apply asks before it replaces itself, and stops when I say
no, since the fleet's changes may need that version.

```console
$ cd /var/lib/mothership/fleet && sed -i 's/^x86_64 = .*/x86_64 = "0000"/; s/0.1.0/0.2.0/' mothership.toml &&
> echo y | sudo mothership commit "Move to 0.2.0" >/dev/null
$ echo n | sudo mothership apply
The fleet runs mothership 0.2.0.
Update [y/N]? n
Apply stops: the fleet's changes may need that version.
```

A version change nobody signed never reaches the machine: apply refuses the commit and stays on
the pinned version, without asking to update.

```console
$ cd /var/lib/mothership/fleet && git revert --no-commit HEAD &&
> echo y | sudo mothership commit "Back to 0.1.0" >/dev/null &&
> sed -i 's/^x86_64 = .*/x86_64 = "1111"/; s/0.1.0/0.3.0/' mothership.toml &&
> git commit -qam "Move to 0.3.0" && echo n | sudo mothership apply | head -1
This machine refuses [0-9a-f]{12} "Move to 0.3.0" of \d{4}-\d\d-\d\d, which claims to be by Me, and what follows it: nobody signed it (re)
$ echo n | sudo mothership apply | grep -c Update
0
[exit 1]
```

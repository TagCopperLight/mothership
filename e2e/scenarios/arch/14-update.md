# mothership follows the version the fleet pins

The fleet pins the mothership this machine already runs, so apply goes on as usual.

```console
$ cd /var/lib/mothership/fleet && sha=$(sha256sum /usr/local/bin/mothership | cut -d' ' -f1) &&
> printf 'version = "0.1.0"\nsha256 = "%s"\n' "$sha" > mothership.toml &&
> git add mothership.toml && git commit -qm "Pin mothership"
$ echo n | sudo mothership apply | tail -1
Apply [y/N]? n
```

The fleet moves to another version. Apply asks before it replaces itself, and stops when I say
no, since the fleet's changes may need that version.

```console
$ cd /var/lib/mothership/fleet && sed -i 's/^sha256 = .*/sha256 = "0000"/; s/0.1.0/0.2.0/' mothership.toml &&
> git commit -qam "Move to 0.2.0"
$ echo n | sudo mothership apply
The fleet runs mothership 0.2.0.
Update [y/N]? n
Apply stops: the fleet's changes may need that version.
```

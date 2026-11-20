# A package from a source the fleet adds

The fleet lists `delta`, which only `/extra` has, and adds that source to apt's config. Apply
writes the source before it installs, so apt finds `delta`.

```console
$ echo y | sudo mothership apply > /dev/null 2>&1
$ cd /var/lib/mothership/fleet
> mkdir -p types/server/etc/apt/sources.list.d
> echo 'deb [trusted=yes] copy:/extra ./' > types/server/etc/apt/sources.list.d/extra.list
> sed -i 's/  "gamma",/  "gamma",\n  "delta",/' types/server/system.toml
> echo y | sudo mothership commit "Add delta" >/dev/null
$ echo y | sudo mothership apply
Coming from the fleet · 2
  Packages
    + delta                                   install
  Files
    + /etc/apt/sources.list.d/extra.list      create

Apply [y/N]? y
...
$ dpkg-query -W -f '${Package} ${db:Status-Abbrev}\n' delta
delta ii
$ sudo mothership scan
✓ Nothing differs from the server type
\d+ ignored (re)
```

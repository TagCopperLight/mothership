# Apply undoes what the fleet drops, unless I edited it

The machine follows its fleet. Then the fleet configures `gamma` and adds two files.

```console
$ echo y | sudo mothership apply &>/dev/null
$ cd /var/lib/mothership/fleet &&
> printf 'volume = 11\n' > types/workstation/etc/gamma.conf &&
> printf 'a = 1\n' > types/workstation/etc/extra.conf &&
> printf 'b = 1\n' > types/workstation/etc/kept.conf &&
> git add -A && git commit -S -qm "Configure gamma, add two files"
$ echo y | sudo mothership apply
Coming from the fleet · 3
  Files
    ~ /etc/gamma.conf  update
    + /etc/extra.conf  create
    + /etc/kept.conf   create

Apply [y/N]? y
$ cat /etc/gamma.conf
volume = 11
```

I edit `/etc/kept.conf`. Then the fleet drops the three files and `alpha`.

```console
$ echo "mine = yes" | sudo tee -a /etc/kept.conf >/dev/null
$ cd /var/lib/mothership/fleet &&
> git rm -q types/workstation/etc/gamma.conf types/workstation/etc/extra.conf types/workstation/etc/kept.conf &&
> sed -i '/^  "alpha",$/d' types/workstation/system.toml &&
> git commit -S -qam "Drop alpha and the files"
```

Apply uninstalls `alpha`, which nothing needs, puts back the config `gamma` shipped, and deletes
the extra file. It keeps the file I edited, for review.

```console
$ echo y | sudo mothership apply
Coming from the fleet · 3
  Packages
    - alpha            uninstall                      A library other packages need
  Files
    - /etc/extra.conf  delete
    ~ /etc/gamma.conf  restore the package's version  owned by gamma

1 changed on this machine, for sudo mothership review
Apply [y/N]? y
...
$ pacman -Qq alpha
error: package 'alpha' was not found
[exit 1]
$ cat /etc/gamma.conf
volume = 5
$ ls /etc/kept.conf
/etc/kept.conf
$ ls /etc/extra.conf
ls: cannot access '/etc/extra.conf': No such file or directory
[exit 2]
```

# Apply undoes what the fleet drops, unless I edited it

The machine follows its fleet. Then the fleet configures `gamma` and adds two files.

```console
$ echo y | sudo mothership apply &>/dev/null
$ cd ~/.local/share/chezmoi &&
> printf 'volume = 11\n' > types/workstation/etc/gamma.conf &&
> printf 'a = 1\n' > types/workstation/etc/extra.conf &&
> printf 'b = 1\n' > types/workstation/etc/kept.conf &&
> git add -A && git commit -qm "Configure gamma, add two files"
$ echo y | sudo mothership apply
/etc (3)
  /etc/extra.conf  write the fleet's version
  /etc/gamma.conf  write the fleet's version
  /etc/kept.conf   write the fleet's version

Apply [y/N]? y
$ cat /etc/gamma.conf
volume = 11
```

I edit `/etc/kept.conf`. Then the fleet drops the three files and `alpha`.

```console
$ echo "mine = yes" | sudo tee -a /etc/kept.conf >/dev/null
$ cd ~/.local/share/chezmoi &&
> git rm -q types/workstation/etc/gamma.conf types/workstation/etc/extra.conf types/workstation/etc/kept.conf &&
> printf 'gamma\n' > packages/apps.txt &&
> git commit -qam "Drop alpha and the files"
```

Apply uninstalls `alpha`, which nothing needs, puts back the config `gamma` shipped, and deletes
the extra file. It keeps the file I edited, for review.

```console
$ echo y | sudo mothership apply
Packages (1)
  alpha  uninstall

/etc (2)
  /etc/gamma.conf  restore the package's version
  /etc/extra.conf  delete

1 other differences wait for `mothership review`.
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

# Files install scripts write in /etc

`kappa`'s install script writes `/etc/kappa.conf` through ucf, adds an alternative and links
its init script into `/etc/rc?.d`. No package lists those files, and the scan knows whose they
are, so only the package itself is new.

```console
$ echo y | sudo mothership apply > /dev/null 2>&1
$ sudo apt-get install -y kappa > /dev/null 2>&1
$ sudo mothership scan
Changed on this machine · 1
  Packages
    + kappa  installed  An app whose install script writes to /etc

1 to review     sudo mothership review
\d+ ignored (re)
```

I edit the config file and pick the alternative by hand. dpkg's choice was `kappa-one`, which
has the higher priority.

```console
$ echo "level = 2" | sudo tee /etc/kappa.conf > /dev/null
$ sudo update-alternatives --set kappa /usr/bin/kappa-two > /dev/null
$ sudo mothership scan
Changed on this machine · 4
  Packages
    + kappa                         installed  An app whose install script writes to /etc
  Files
    + /etc/alternatives/kappa       new        → /usr/bin/kappa-two
    + /etc/alternatives/kappa.1.gz  new        → /usr/share/kappa/two.1.gz
    ~ /etc/kappa.conf               edited     written by kappa's install script

4 to review     sudo mothership review
\d+ ignored (re)
```

# Apply installs the fleet's tools as me

The machine follows its fleet.

```console
$ echo y | sudo mothership apply &>/dev/null
```

The fleet adds a cargo tool from git, and an installer script, to my part of the type.

```console
$ cd /var/lib/mothership/fleet && cat >> types/workstation/users/me/user.toml <<'TOML' && echo y | sudo mothership commit "Add tools" >/dev/null
> [[installers]]
> name = "greeter"
> run = "mkdir ~/.greeter && echo hi > ~/.greeter/hi"
> creates = "~/.greeter"
> [tools]
> cargo = [{ name = "hello", git = "file:///crates/hello" }]
> TOML
```

Apply installs both, in my home and as me.

```console
$ echo y | sudo mothership apply
Coming from the fleet · 2
  Tools
    + cargo hello  install
    + greeter      run its installer  ~/.greeter is missing

Apply [y/N]? y
...
$ ~/.cargo/bin/hello
hello
$ stat -c %U ~/.cargo/bin/hello ~/.greeter/hi
me
me
```

The fleet moves `hello` to its `loud` branch, and apply reinstalls it from there.

```console
$ cd /var/lib/mothership/fleet &&
> sed -i 's|"file:///crates/hello" }|"file:///crates/hello", branch = "loud" }|' types/workstation/users/me/user.toml &&
> echo y | sudo mothership commit "Follow the loud branch" >/dev/null
$ echo y | sudo mothership apply
Coming from the fleet · 1
  Tools
    ~ cargo hello  reinstall from the fleet's source  from file:///crates/hello, the fleet says file:///crates/hello, branch loud

Apply [y/N]? y
...
$ ~/.cargo/bin/hello
HELLO
```

The fleet drops `hello`, and I delete what the installer made. Apply uninstalls `hello`, and
doesn't run the installer again: the missing folder waits for review.

```console
$ cd /var/lib/mothership/fleet && sed -i '/^\[tools\]/,$d' types/workstation/users/me/user.toml &&
> echo y | sudo mothership commit "Drop hello" >/dev/null && rm -r ~/.greeter
$ echo y | sudo mothership apply
Coming from the fleet · 1
  Tools
    - cargo hello  uninstall  from file:///crates/hello, branch loud

1 changed on this machine, for sudo mothership review
Apply [y/N]? y
...
$ cargo install --list
$ sudo mothership scan | head -3
Changed on this machine · 1
  Tools
    - greeter  not installed  ~/.greeter is missing
```

I install `hello` again by hand, from its `loud` branch. Review offers to declare it with that
source, and once added, the scan finds nothing left to say about it.

```console
$ cargo install -q --git file:///crates/hello --branch loud hello &>/dev/null
$ printf 'a\nq\ny\n' | sudo mothership review
[1/2] Tools
    + cargo hello  installed  from file:///crates/hello, branch loud
  a  add         write this machine's source into `tools.cargo` in user.toml
  r  revert      uninstall
  i ignore · l later · p pass · q quit
  › a
[2/2] Tools
    - greeter  not installed  ~/.greeter is missing
  a  add         remove its installer from user.toml
  r  revert      run its installer
  i ignore · l later · p pass · q quit
  › q
...
$ grep hello /var/lib/mothership/fleet/types/workstation/users/me/user.toml
cargo = [{ name = "hello", git = "file:///crates/hello", branch = "loud" }]
$ sudo mothership scan | head -3
Changed on this machine · 1
  Tools
    - greeter  not installed  ~/.greeter is missing
```

I download a program into `~/.local/bin` and an app into `~/.local/opt`, and write a script of
my own next to the program. The fleet could keep the script as a file, but it can't download the
other two again, so review only offers to add the script.

```console
$ mkdir -p ~/.local/opt ~/.local/bin && cp /usr/bin/true ~/.local/bin/stamp && cp /usr/bin/true ~/.local/opt/App.AppImage &&
> printf '#!/bin/sh\necho hi\n' > ~/.local/bin/hi
$ printf 'p\np\np\nq\ny\n' | sudo mothership review
[1/4] Tools
    + ~/.local/bin/stamp  installed
  can't add: nothing in the fleet downloads it: write an installer for it
  i ignore · l later · p pass · q quit
  › p
[2/4] Tools
    + ~/.local/opt/App.AppImage  installed
  can't add: nothing in the fleet downloads it: write an installer for it
  i ignore · l later · p pass · q quit
  › p
[3/4] Tools
    - greeter  not installed  ~/.greeter is missing
  a  add         remove its installer from user.toml
  r  revert      run its installer
  i ignore · l later · p pass · q quit
  › p
[4/4] Files
    + ~/.local/bin/hi  new
      #!/bin/sh
      echo hi
  a  add         add it to chezmoi
  s  secret      add it to chezmoi, encrypted
  i ignore · l later · p pass · q quit
  › q
3 passed
1 not seen yet, for next time
```

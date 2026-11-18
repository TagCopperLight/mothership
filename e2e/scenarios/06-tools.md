# Apply installs the fleet's tools as me

The machine follows its fleet.

```console
$ echo y | sudo mothership apply &>/dev/null
```

The fleet adds a cargo tool from git, and an installer script.

```console
$ cd ~/.local/share/chezmoi && cat >> types/workstation/system.toml <<'TOML' && git commit -qam "Add tools"
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
Tools outside pacman (2)
  cargo hello  install
  greeter      run its installer

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
$ cd ~/.local/share/chezmoi &&
> sed -i 's|"file:///crates/hello" }|"file:///crates/hello", branch = "loud" }|' types/workstation/system.toml &&
> git commit -qam "Follow the loud branch"
$ echo y | sudo mothership apply
Tools outside pacman (1)
  cargo hello  reinstall from the fleet's source

Apply [y/N]? y
...
$ ~/.cargo/bin/hello
HELLO
```

The fleet drops `hello`, and I delete what the installer made. Apply uninstalls `hello`, and
doesn't run the installer again: the missing folder waits for review.

```console
$ cd ~/.local/share/chezmoi && sed -i '/^\[tools\]/,$d' types/workstation/system.toml &&
> git commit -qam "Drop hello" && rm -r ~/.greeter
$ echo y | sudo mothership apply
Tools outside pacman (1)
  cargo hello  uninstall

1 other differences wait for `mothership review`.
Apply [y/N]? y
...
$ cargo install --list
$ mothership scan | head -2
Tools outside pacman (1)
  greeter  declared, not installed
```

I install `hello` again by hand, from its `loud` branch. Review offers to declare it with that
source, and once added, the scan finds nothing left to say about it.

```console
$ cargo install -q --git file:///crates/hello --branch loud hello &>/dev/null
$ printf 'a\nq\n' | mothership review
\[1/\d+\] Tools outside pacman (re)
cargo hello  installed from file:///crates/hello, branch loud, not declared
  add: write this machine's source into `tools.cargo` in system.toml
  (a)dd (i)gnore (l)ater (p)ass (q)uit? a
\[2/\d+\] Tools outside pacman (re)
greeter  declared, not installed
  add: remove its installer from system.toml
  (a)dd (i)gnore (l)ater (p)ass (q)uit? q
...
$ grep hello ~/.local/share/chezmoi/types/workstation/system.toml
cargo = [{ name = "hello", git = "file:///crates/hello", branch = "loud" }]
$ mothership scan | head -2
Tools outside pacman (1)
  greeter  declared, not installed
```

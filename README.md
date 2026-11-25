<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/wordmark-dark.svg">
    <img src="assets/wordmark.svg" alt="mothership" width="360">
  </picture>
</p>

mothership sets up machines from a git repository, the fleet, and tells you when a machine drifts away from it. The fleet describes device types, such as "workstation", and every machine of a type ends up identical. It runs on Arch, Debian and Ubuntu.

[chezmoi](https://chezmoi.io) writes the files in each user's home. mothership handles the rest: packages, `/etc`, services, user accounts, and tools installed with cargo, uv and bun.

## Getting started

1. **Write a fleet** in a private git repository: one folder per device type, with its packages, `/etc` files and users' homes. [Writing a fleet](docs/fleet.md) shows the layout.
2. **Download mothership** on the machine and check it:

   ```sh
   base=https://github.com/TagCopperLight/mothership/releases/latest/download
   curl -fLO $base/mothership-x86_64 -fLO $base/SHA256SUMS
   sha256sum -c SHA256SUMS && install -m 755 mothership-x86_64 mothership
   ```

3. **Install**: `sudo ./mothership install` asks for the fleet's SSH URL, the hostname, the type and the user who owns the machine's copy of the fleet. On a new fleet's first machine, install makes the fleet's recovery key: write it on paper. To let a later machine in, run the `trust` command it prints on a machine already in the fleet and type back the code, or type the recovery key. From the Arch ISO, install also partitions and encrypts the disk.

From then on, `sudo mothership apply` brings in the fleet's changes, `sudo mothership scan` shows how the machine drifted, and `sudo mothership review` lets you keep each change in the fleet or undo it.

## Learn more

- [The design](docs/MOTHERSHIP.md), with every command
- [Writing a fleet](docs/fleet.md)

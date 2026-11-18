# A throwaway Arch machine for the scenarios. They run with no network, so everything they
# install comes from the packages built here.
FROM archlinux:base AS machine
RUN pacman -Syu --noconfirm --needed base-devel git sudo chezmoi rust uv dconf \
 && rm -rf /var/cache/pacman/pkg/*

# A local repo stands in for pacman's repos, and /aur for the AUR. /local holds a package
# built on the machine, which neither has.
COPY packages /build
RUN useradd -m builder && chown -R builder /build \
 && for dir in /build/*/*/; do (cd "$dir" && runuser -u builder -- makepkg --nodeps); done \
 && mkdir /repo /aur /local \
 && cp /build/repo/*/*.pkg.tar.zst /repo && cp /build/aur/*/*.pkg.tar.zst /aur \
 && cp /build/local/*/*.pkg.tar.zst /local \
 && repo-add -q /repo/fleettest.db.tar.gz /repo/*.pkg.tar.zst \
 && userdel -r builder && rm -rf /build \
 && printf '\n[fleettest]\nSigLevel = Never\nServer = file:///repo\n' >> /etc/pacman.conf \
 && pacman -Sy

# A crate cargo installs from git, with a `loud` branch that prints otherwise.
COPY crates /crates
RUN cd /crates/hello && git init -q -b main && git add -A \
 && git -c user.name=Me -c user.email=me@example.com commit -qm hello \
 && git checkout -qb loud && sed -i 's/"hello"/"HELLO"/' src/main.rs \
 && git -c user.name=Me -c user.email=me@example.com commit -qam loud && git checkout -q main

# No systemd runs in the container, so systemctl only logs what it was asked.
COPY bin/systemctl bin/fake-aur /usr/local/bin/
RUN touch /var/log/systemctl.log && chmod 666 /var/log/systemctl.log

FROM machine
COPY fleet /fleet
RUN useradd -m -G wheel -s /bin/bash me \
 && install -m 440 /fleet/types/workstation/etc/sudoers.d/wheel /etc/sudoers.d/wheel \
 && mkdir -p /home/me/.local/share /home/me/.config/chezmoi \
 && cp -r /fleet /home/me/.local/share/chezmoi && rm -rf /fleet \
 && printf '%s\n' 'encryption = "age"' '[age]' 'useBuiltin = true' \
      'identity = "~/.config/mothership/key.txt"' \
      'recipientsFile = "~/.local/share/chezmoi/types/workstation/recipients.txt"' \
      '[data]' 'profile = "workstation"' > /home/me/.config/chezmoi/chezmoi.toml \
 && printf 'type = "workstation"\n' | install -D -m 644 /dev/stdin /etc/mothership/machine.toml \
 && chown -R me:me /home/me \
 && runuser -u me -- git -C /home/me/.local/share/chezmoi init -q -b main \
 && runuser -u me -- git -C /home/me/.local/share/chezmoi add -A \
 && runuser -u me -- git -C /home/me/.local/share/chezmoi \
      -c user.name=Me -c user.email=me@example.com commit -q -m "The fleet" \
 && runuser -u me -- env HOME=/home/me chezmoi apply
USER me
WORKDIR /home/me
ENV HOME=/home/me USER=me

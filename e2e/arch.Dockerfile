FROM archlinux:base AS machine
RUN pacman -Syu --noconfirm --needed base-devel git sudo chezmoi rust uv dconf \
 && rm -rf /var/cache/pacman/pkg/*

COPY arch-packages /build
RUN useradd -m builder && chown -R builder /build \
 && for dir in /build/*/*/; do (cd "$dir" && runuser -u builder -- makepkg --nodeps); done \
 && mkdir /repo /aur /local \
 && cp /build/repo/*/*.pkg.tar.zst /repo && cp /build/aur/*/*.pkg.tar.zst /aur \
 && cp /build/local/*/*.pkg.tar.zst /local \
 && repo-add -q /repo/fleettest.db.tar.gz /repo/*.pkg.tar.zst \
 && userdel -r builder && rm -rf /build \
 && printf '\n[fleettest]\nSigLevel = Never\nServer = file:///repo\n' >> /etc/pacman.conf \
 && pacman -Sy

COPY crates /crates
RUN cd /crates/hello && git init -q -b main && git add -A \
 && git -c user.name=Me -c user.email=me@example.com commit -qm hello \
 && git checkout -qb loud && sed -i 's/"hello"/"HELLO"/' src/main.rs \
 && git -c user.name=Me -c user.email=me@example.com commit -qam loud && git checkout -q main

COPY bin/systemctl bin/loginctl bin/yay /usr/local/bin/
RUN touch /var/log/systemctl.log && chmod 666 /var/log/systemctl.log

FROM machine
COPY fleet /var/lib/mothership/fleet
COPY chezmoi/workstation.toml /tmp/chezmoi.toml
COPY recovery.txt /tmp/recovery.txt
RUN useradd -m -G wheel -s /bin/bash me \
 && install -o me -g me -m 600 /tmp/recovery.txt /home/me/recovery.txt \
 && install -m 440 /var/lib/mothership/fleet/types/workstation/etc/sudoers.d/wheel /etc/sudoers.d/wheel \
 && printf 'type = "workstation"\n' | install -D -m 644 /dev/stdin /etc/mothership/machine.toml \
 && chown -R me:me /var/lib/mothership/fleet \
 && cd /var/lib/mothership/fleet \
 && runuser -u me -- git init -q -b main \
 && runuser -u me -- git add -A \
 && runuser -u me -- git -c user.name=Me -c user.email=me@example.com commit -q -m "The fleet" \
 && install -d -o me -g me /srv/fleet.git \
 && runuser -u me -- git init -q --bare /srv/fleet.git \
 && runuser -u me -- git remote add origin /srv/fleet.git \
 && runuser -u me -- git push -q -u origin main \
 && runuser -u me -- install -D -m 644 /tmp/chezmoi.toml /home/me/.config/chezmoi/chezmoi.toml \
 && cd / && runuser -u me -- env HOME=/home/me chezmoi apply
USER me
WORKDIR /home/me
ENV HOME=/home/me USER=me

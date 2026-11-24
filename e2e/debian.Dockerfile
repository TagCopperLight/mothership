FROM debian:trixie AS machine
RUN apt-get update && apt-get install -y --no-install-recommends sudo git dpkg-dev curl ca-certificates ucf \
 && curl -fsSL -o /tmp/chezmoi.deb \
    https://github.com/twpayne/chezmoi/releases/download/v2.74.2/chezmoi_2.74.2_linux_amd64.deb \
 && apt-get install -y /tmp/chezmoi.deb && rm /tmp/chezmoi.deb

COPY debian-packages /build
RUN for group in repo local extra; do mkdir "/$group" \
    && for dir in /build/$group/*/; do dpkg-deb --root-owner-group -b "$dir" "/$group" >/dev/null; done; done \
 && rm -rf /build \
 && (cd /repo && dpkg-scanpackages . > Packages 2>/dev/null) \
 && (cd /extra && dpkg-scanpackages . > Packages 2>/dev/null) \
 && rm /etc/apt/sources.list.d/debian.sources /etc/apt/apt.conf.d/docker-clean \
 && echo 'deb [trusted=yes] copy:/repo ./' > /etc/apt/sources.list.d/fleettest.list \
 && apt-get update

COPY bin/systemctl bin/loginctl bin/hostnamectl /usr/local/bin/
RUN touch /var/log/systemctl.log && chmod 666 /var/log/systemctl.log

FROM machine
COPY fleet /var/lib/mothership/fleet
COPY chezmoi/server.toml /tmp/chezmoi.toml
COPY signing signing.pub /tmp/
COPY ssh ssh.pub /tmp/
COPY recovery.txt /tmp/recovery.txt
COPY anchor.sh /tmp/anchor.sh
RUN useradd -m -G sudo -s /bin/bash me \
 && install -o me -g me -m 600 /tmp/recovery.txt /home/me/recovery.txt \
 && install -D -m 600 /tmp/signing /etc/mothership/signing \
 && install -d -o me -g me -m 700 /home/me/.ssh \
 && install -o me -g me -m 600 /tmp/ssh /home/me/.ssh/id_ed25519 \
 && install -o me -g me -m 644 /tmp/ssh.pub /home/me/.ssh/id_ed25519.pub \
 && install -m 440 /var/lib/mothership/fleet/types/server/etc/sudoers.d/sudo /etc/sudoers.d/sudo \
 && printf 'type = "server"\n' | install -D -m 644 /dev/stdin /etc/mothership/machine.toml \
 && chown -R me:me /var/lib/mothership/fleet \
 && cd /var/lib/mothership/fleet \
 && runuser -u me -- git init -q -b main \
 && runuser -u me -- git add -A \
 && runuser -u me -- git -c user.name=Me -c user.email=me@example.com commit -q -m "The fleet" \
 && install -d -o me -g me /srv/fleet.git \
 && runuser -u me -- git init -q --bare /srv/fleet.git \
 && runuser -u me -- git remote add origin /srv/fleet.git \
 && runuser -u me -- git push -q -u origin main \
 && /tmp/anchor.sh \
 && runuser -u me -- install -D -m 644 /tmp/chezmoi.toml /home/me/.config/chezmoi/chezmoi.toml \
 && cd / && runuser -u me -- env HOME=/home/me chezmoi apply
USER me
WORKDIR /home/me
ENV HOME=/home/me USER=me

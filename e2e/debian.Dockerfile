FROM debian:trixie AS machine
RUN apt-get update && apt-get install -y --no-install-recommends sudo git dpkg-dev curl ca-certificates ucf \
 && curl -fsSL -o /tmp/chezmoi.deb \
    https://github.com/twpayne/chezmoi/releases/download/v2.74.1/chezmoi_2.74.1_linux_amd64.deb \
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
RUN useradd -m -G sudo -s /bin/bash me \
 && install -m 440 /var/lib/mothership/fleet/types/server/etc/sudoers.d/sudo /etc/sudoers.d/sudo \
 && printf 'type = "server"\n' | install -D -m 644 /dev/stdin /etc/mothership/machine.toml \
 && chown -R me:me /var/lib/mothership/fleet \
 && cd /var/lib/mothership/fleet \
 && runuser -u me -- git init -q -b main \
 && runuser -u me -- git add -A \
 && runuser -u me -- git -c user.name=Me -c user.email=me@example.com commit -q -m "The fleet" \
 && runuser -u me -- install -D -m 644 /tmp/chezmoi.toml /home/me/.config/chezmoi/chezmoi.toml \
 && cd / && runuser -u me -- env HOME=/home/me chezmoi apply
USER me
WORKDIR /home/me
ENV HOME=/home/me USER=me

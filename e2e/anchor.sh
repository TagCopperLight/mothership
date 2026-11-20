#!/bin/sh
# Does what install does once the machine trusts a fleet commit.
set -eu
lib=/var/lib/mothership
git init -q --bare "$lib/fleet.git"
git --git-dir "$lib/fleet.git" fetch -q --upload-pack 'runuser -u me -- git-upload-pack' "$lib/fleet" main:refs/incoming/heads/main
commit=$(git --git-dir "$lib/fleet.git" rev-parse refs/incoming/heads/main)
git --git-dir "$lib/fleet.git" update-ref refs/mothership/checked "$commit"
mkdir -p "$lib/checked/$commit"
git --git-dir "$lib/fleet.git" archive "$commit" | tar -x --no-same-owner -C "$lib/checked/$commit"
ln -s "$lib/checked/$commit" "$lib/current"
key="key::$(ssh-keygen -y -f /etc/mothership/signing | cut -d' ' -f1,2)"
cd "$lib/fleet"
runuser -u me -- git config gpg.format ssh
runuser -u me -- git config gpg.ssh.program /usr/local/bin/mothership
runuser -u me -- git config user.signingkey "$key"

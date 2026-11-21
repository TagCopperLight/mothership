#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["pyrage"]
# ///

import hashlib
import http.server
import json
import shutil
import socket
import ssl
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
from functools import partial
from pathlib import Path

import pyrage
from pyrage import x25519

HERE = Path(__file__).resolve().parent
E2E = HERE.parent
sys.path.insert(0, str(E2E))
import binary

CACHE = Path.home() / ".cache/mothership-vm"
CLOUD_IMAGE = CACHE / "debian-13-genericcloud-amd64.qcow2"
CHEZMOI = CACHE / "chezmoi_2.74.1_linux_amd64.deb"
PREPARED = CACHE / "debian-services.qcow2"
LOG = CACHE / "services.log"
MIRROR = "http://mirrors.aliyun.com/debian"
HOST = "192.168.76.2"
NET = "192.168.76.0/24"
TOOLS = ["qemu-system-x86_64", "qemu-img", "docker", "openssl", "ssh", "scp", "rustc"]
PACKAGES = ["chezmoi", "curl", "dbus-user-session", "docker-compose", "docker.io", "git",
            "rootlesskit", "slirp4netns", "sudo", "systemd-container", "uidmap"]
VERSIONS = ["1.0.0", "1.0.1", "1.0.2", "1.0.3"]
TOKEN = "s3cret-token"
FLEET = "/var/lib/mothership/fleet"


def main() -> int:
    if missing := [tool for tool in TOOLS if not shutil.which(tool)]:
        sys.exit(f"the services VM test needs {', '.join(missing)}")
    for needed in [CLOUD_IMAGE, CHEZMOI]:
        if not needed.exists():
            sys.exit(f"{needed} is missing")
    binary.build()
    with tempfile.TemporaryDirectory(prefix="mothership-services-") as tmp, open(LOG, "w") as log:
        tmp = Path(tmp)
        registry = Registry(tmp)
        servers = [serve(registry.handler(), tls(tmp)), serve(partial(Seed, directory=str(seed(tmp))))]
        try:
            registry_port, seed_port = (server.server_address[1] for server in servers)
            image = f"{HOST}:{registry_port}/app"
            if not prepared():
                prepare(tmp, seed_port, log)
            vm = VM(tmp, seed_port, log)
            try:
                scenario(vm, registry, image, tmp)
            finally:
                vm.stop()
        finally:
            for server in servers:
                server.shutdown()
    print("The services run as services under rootless Docker: install, update, rollback, drop and probe.")
    return 0


def tls(tmp: Path) -> ssl.SSLContext:
    subprocess.run(["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
                    "-nodes", "-days", "2", "-subj", "/CN=mothership test registry",
                    "-addext", f"subjectAltName=IP:{HOST}", "-keyout", str(tmp / "registry.key"),
                    "-out", str(tmp / "registry.crt")], check=True, capture_output=True)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(tmp / "registry.crt", tmp / "registry.key")
    return context


class Registry:
    def __init__(self, tmp: Path):
        self.blobs = tmp / "blobs"
        self.blobs.mkdir()
        self.tags: dict[str, tuple[str, str]] = {}
        self.built: dict[str, tuple[str, str]] = {}
        for version in VERSIONS:
            self.built[version] = build(tmp, version, self.blobs)
        for version in VERSIONS[:3]:
            self.publish(version)

    def publish(self, version: str) -> None:
        self.tags[version] = self.built[version]

    def digest(self, version: str) -> str:
        return self.built[version][0]

    def handler(self):
        registry = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args) -> None:
                pass

            def do_HEAD(self) -> None:
                self.answer(body=False)

            def do_GET(self) -> None:
                self.answer(body=True)

            def answer(self, body: bool) -> None:
                parts = self.path.split("?")[0].strip("/").split("/")
                if parts == ["v2"]:
                    return self.send(200, b"{}", "application/json", body)
                if parts[:2] == ["v2", "app"] and parts[2:] == ["tags", "list"]:
                    tags = json.dumps({"name": "app", "tags": sorted(registry.tags)}).encode()
                    return self.send(200, tags, "application/json", body)
                if parts[:3] == ["v2", "app", "manifests"] and len(parts) == 4:
                    reference = parts[3]
                    found = registry.tags.get(reference)
                    if found is None and reference.startswith("sha256:"):
                        found = next((t for t in registry.built.values() if t[0] == reference), None)
                    if found is None:
                        return self.send(404, b"{}", "application/json", body)
                    digest, media = found
                    blob = (registry.blobs / digest.split(":")[1]).read_bytes()
                    return self.send(200, blob, media, body, digest)
                if parts[:3] == ["v2", "app", "blobs"] and len(parts) == 4:
                    path = registry.blobs / parts[3].split(":")[-1]
                    if path.exists():
                        return self.send(200, path.read_bytes(), "application/octet-stream", body,
                                         parts[3])
                self.send(404, b"{}", "application/json", body)

            def send(self, status, data, media, body, digest=None) -> None:
                self.send_response(status)
                self.send_header("Content-Type", media)
                self.send_header("Content-Length", str(len(data)))
                if digest:
                    self.send_header("Docker-Content-Digest", digest)
                self.end_headers()
                if body:
                    self.wfile.write(data)

        return Handler


def build(tmp: Path, version: str, blobs: Path) -> tuple[str, str]:
    context = tmp / f"app-{version}"
    context.mkdir()
    shutil.copy(HERE / "app/Dockerfile", context)
    subprocess.run(["rustc", "-O", "--target", "x86_64-unknown-linux-musl", "-C",
                    "target-feature=+crt-static", "-o", str(context / "app"),
                    str(HERE / "app/main.rs")], check=True, env={"APP_VERSION": version,
                                                                 "PATH": "/usr/bin:/bin"})
    tag = f"mothership-vm-app:{version}"
    subprocess.run(["docker", "build", "--quiet", "--provenance=false", "--sbom=false", "-t", tag,
                    str(context)], check=True, capture_output=True)
    saved = tmp / f"app-{version}.tar"
    subprocess.run(["docker", "save", "-o", str(saved), tag], check=True)
    subprocess.run(["docker", "rmi", "--force", tag], check=True, capture_output=True)
    oci = tmp / f"oci-{version}"
    with tarfile.open(saved) as archive:
        archive.extractall(oci, filter="data")
    for blob in (oci / "blobs/sha256").iterdir():
        shutil.copy(blob, blobs / blob.name)
    index = json.loads((oci / "index.json").read_text())["manifests"][0]
    return index["digest"], index["mediaType"]


class Seed(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args) -> None:
        pass


def seed(tmp: Path) -> Path:
    dir = tmp / "seed"
    dir.mkdir()
    (dir / "meta-data").write_text("instance-id: mothership-services\nlocal-hostname: minipc\n")
    (dir / "user-data").write_text(user_data())
    shutil.copy(CHEZMOI, dir / "chezmoi.deb")
    return dir


def user_data() -> str:
    key = (E2E / "ssh.pub").read_text().strip()
    packages = "\n".join(f"  - {p}" for p in PACKAGES if p != "chezmoi")
    return f"""#cloud-config
hostname: minipc
users:
  - name: me
    groups: [sudo]
    shell: /bin/bash
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    ssh_authorized_keys: ["{key}"]
apt:
  primary:
    - arches: [default]
      uri: {MIRROR}
  security:
    - arches: [default]
      uri: {MIRROR}-security
package_update: true
packages:
{packages}
runcmd:
  - [sh, -c, "curl -fsS -o /tmp/chezmoi.deb http://{HOST}:$SEED/chezmoi.deb && apt-get install -y /tmp/chezmoi.deb"]
"""


def serve(handler, context=None) -> http.server.ThreadingHTTPServer:
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    if context:
        server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def cache_key() -> str:
    return hashlib.sha256(user_data().encode()).hexdigest()


def prepared() -> bool:
    stamp = PREPARED.with_suffix(".key")
    return PREPARED.exists() and stamp.exists() and stamp.read_text() == cache_key()


def prepare(tmp: Path, seed_port: int, log) -> None:
    (tmp / "seed/user-data").write_text(user_data().replace("$SEED", str(seed_port)))
    disk = tmp / "prepare.qcow2"
    subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(CLOUD_IMAGE), "-F", "qcow2",
                    str(disk), "12G"], check=True)
    vm = VM(tmp, seed_port, log, disk)
    try:
        vm.run("cloud-init status --wait > /dev/null; cloud-init status", timeout=2400)
        vm.run("dpkg -s " + " ".join(PACKAGES) + " > /dev/null")
        vm.run("sudo apt-get clean && sudo touch /etc/cloud/cloud-init.disabled")
    finally:
        vm.stop()
    part = PREPARED.with_suffix(".part")
    subprocess.run(["qemu-img", "convert", "-O", "qcow2", str(disk), str(part)], check=True)
    part.rename(PREPARED)
    PREPARED.with_suffix(".key").write_text(cache_key())


class VM:
    def __init__(self, tmp: Path, seed_port: int, log, disk: Path | None = None):
        self.log = log
        self.key = tmp / "ssh"
        shutil.copy(E2E / "ssh", self.key)
        self.key.chmod(0o600)
        if disk is None:
            disk = tmp / "disk.qcow2"
            subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(PREPARED), "-F",
                            "qcow2", str(disk)], check=True)
        self.port = free_port()
        self.process = subprocess.Popen([
            "qemu-system-x86_64", "-enable-kvm", "-cpu", "host", "-m", "3072", "-smp", "2",
            "-display", "none", "-serial", f"file:{tmp / 'serial.log'}", "-monitor", "none",
            "-drive", f"file={disk},if=virtio,format=qcow2",
            "-nic", f"user,model=virtio-net-pci,net={NET},host={HOST},"
                    f"hostfwd=tcp:127.0.0.1:{self.port}-:22",
            "-smbios", f"type=1,serial=ds=nocloud;s=http://{HOST}:{seed_port}/",
        ], stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        deadline = time.time() + 600
        while self.run("true", check=False, timeout=20, quiet=True).returncode != 0:
            if self.process.poll() is not None:
                sys.exit(f"the VM stopped: see {tmp / 'serial.log'}")
            if time.time() > deadline:
                self.stop()
                sys.exit("the VM didn't answer on SSH")
            time.sleep(3)

    def ssh(self, *args: str) -> list[str]:
        return ["-i", str(self.key), "-o", "StrictHostKeyChecking=no", "-o",
                "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=5",
                *args]

    def run(self, command: str, input: str = "", check: bool = True, timeout: int = 900,
            quiet: bool = False) -> subprocess.CompletedProcess:
        out = subprocess.run(["ssh", *self.ssh("-p", str(self.port)), "me@127.0.0.1", command],
                             input=input, capture_output=True, text=True, timeout=timeout)
        if not quiet:
            self.log.write(f"$ {command}\n{out.stdout}{out.stderr}[exit {out.returncode}]\n\n")
            self.log.flush()
        if check and out.returncode != 0:
            sys.exit(f"`{command}` failed with {out.returncode}:\n{out.stdout}{out.stderr}\n"
                     f"see {LOG}")
        return out

    def copy(self, *files: Path) -> None:
        subprocess.run(["scp", *self.ssh("-P", str(self.port)), *map(str, files), "me@127.0.0.1:/tmp/"],
                       check=True, capture_output=True, timeout=300)

    def stop(self) -> None:
        if self.process.poll() is None:
            self.run("sudo systemctl poweroff", check=False, timeout=30, quiet=True)
            try:
                self.process.wait(timeout=120)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=30)


def fleet(tmp: Path, image: str, registry: Registry, root: x25519.Identity) -> Path:
    work = tmp / "fleet"
    shutil.copytree(HERE / "services", work)
    manifest = (work / "mothership.toml").read_text()
    recovery = x25519.Recipient.from_str(manifest.split('age = "')[1].split('"')[0])
    server = work / "types/server"
    (server / "recipients.txt").write_text(f"# minipc\n{root.to_public()}\n")
    (server / "services/app/secrets").mkdir()
    (server / "services/app/secrets/token.age").write_bytes(
        pyrage.encrypt(f"{TOKEN}\n".encode(), [root.to_public(), recovery]))
    (server / "services/app/service.toml").write_text(pinned(image, registry, "1.0.0", mine=True))
    (server / "services/extra/service.toml").write_text(
        pinned(image, registry, "1.0.0", mine=False).replace("[images.app]", "[images.extra]"))
    archive = tmp / "fleet.tar"
    with tarfile.open(archive, "w") as tar:
        tar.add(work, arcname=".")
    return archive


def pinned(image: str, registry: Registry, version: str, mine: bool) -> str:
    repo = 'repo = "me/app"\n' if mine else ""
    return (f'[images.app]\nimage = "{image}"\nversion = "{version}"\n'
            f'digest = "{registry.digest(version)}"\n{repo}')


def expect(out: subprocess.CompletedProcess, *needles: str, code: int = 0) -> None:
    text = out.stdout + out.stderr
    if out.returncode != code:
        sys.exit(f"expected exit {code}, got {out.returncode}:\n{text}\nsee {LOG}")
    for needle in needles:
        if needle not in text:
            sys.exit(f"expected {needle!r} in:\n{text}\nsee {LOG}")


def scenario(vm: VM, registry: Registry, image: str, tmp: Path) -> None:
    root = x25519.Identity.generate()
    (tmp / "key.txt").write_text(f"{root}\n")
    vm.copy(binary.BINARY, tmp / "registry.crt", E2E / "signing", tmp / "key.txt", E2E / "ssh",
            E2E / "ssh.pub", fleet(tmp, image, registry, root), E2E / "anchor.sh")
    vm.run(f"""set -e
sudo hostnamectl set-hostname minipc
sudo install -m 755 /tmp/mothership /usr/local/bin/mothership
sudo install -m 644 /tmp/registry.crt /usr/local/share/ca-certificates/mothership-test.crt
sudo update-ca-certificates > /dev/null
sudo install -D -m 600 /tmp/signing /etc/mothership/signing
sudo install -D -m 600 /tmp/key.txt /etc/mothership/key.txt
printf 'type = "server"\\n' | sudo install -D -m 644 /dev/stdin /etc/mothership/machine.toml
install -m 600 /tmp/ssh ~/.ssh/id_ed25519
install -m 644 /tmp/ssh.pub ~/.ssh/id_ed25519.pub
ssh-keyscan -q localhost >> ~/.ssh/known_hosts
git init -q --bare ~/origin.git
sudo install -d -o me -g me {FLEET}
tar -x -C {FLEET} -f /tmp/fleet.tar
cd {FLEET}
git init -q -b main
git add -A
git -c user.name=Me -c user.email=me@example.com commit -q -m 'The fleet'
git remote add origin ssh://me@localhost/home/me/origin.git
git push -q -u origin main
sudo sh /tmp/anchor.sh
""")
    apply = lambda: vm.run("sudo mothership apply", input="y\n" * 6, check=False, timeout=1800)
    data = lambda service, name: vm.run(f"sudo cat /srv/{service}/{name}").stdout

    expect(apply(), "Started app", "Started extra")
    assert data("app", "version") == "1.0.0"
    assert data("app", "token") == f"{TOKEN}\n", "the secret reaches /run/secrets/token"
    assert data("app", "conf") == f"token = {TOKEN}\n", "the template is filled in"
    assert data("app", "precious") == "kept"
    expect(vm.run("stat -c '%U %a' /srv/app /var/lib/mothership/services/app"),
           "services 700", "root 750")
    expect(apply(), "probe  install and start it")
    expect(apply(), "Nothing to apply.")
    scan = vm.run("sudo mothership scan").stdout
    assert "Containers" not in scan, scan

    def commit(message: str, change: str) -> None:
        vm.run(f"cd {FLEET} && {change}")
        expect(vm.run(f"sudo mothership commit '{message}'", input="y\n"), "Committed")

    def pin(version: str) -> str:
        text = pinned(image, registry, version, mine=True)
        return f"cat > types/server/services/app/service.toml <<'EOF'\n{text}EOF"

    commit("Update app", pin("1.0.1"))
    expect(apply(), "Updated app")
    assert data("app", "version") == "1.0.1"
    assert vm.run("sudo cat /var/lib/services/snapshots/app/version").stdout == "1.0.0"

    commit("Drop extra", "git rm -q -r types/server/services/extra")
    expect(apply(), "Removed extra, keeping /srv/extra")
    assert data("extra", "version") == "1.0.0", "a dropped service keeps its data"
    containers = vm.run("sudo -u services env DOCKER_HOST=unix:///run/user/$(id -u services)/docker.sock "
                        "docker ps --all --format '{{.Names}}'").stdout
    assert "extra" not in containers and "app" in containers, containers
    expect(vm.run("sudo mothership scan"), "/srv/extra")

    commit("Break app", pin("1.0.2"))
    expect(apply(), "app failed its health check", "these services need attention", code=1)
    assert data("app", "version") == "1.0.1", "the last healthy version runs again"
    assert data("app", "precious") == "kept", "the snapshot is back"
    expect(vm.run("sudo mothership scan"), "not applied", "images.app 1.0.2 failed its health check")
    expect(apply(), "these services need attention", code=1)

    keys = vm.run("sudo mothership key").stdout
    trust = next(line.strip() for line in keys.splitlines() if "mothership trust" in line)
    probe_ssh = next(line.split(None, 1)[1] for line in keys.splitlines()
                     if line.startswith("probe-ssh"))
    expect(vm.run(trust, input="y\n"), "minipc/probe changes the versions of the fleet's images now")
    vm.run(f"echo '{probe_ssh}' >> ~/.ssh/authorized_keys")
    expect(apply(), "these services need attention", code=1)
    expect(vm.run("systemctl is-enabled mothership-probe.timer"), "enabled")
    registry.publish("1.0.3")
    vm.run("sudo systemctl start mothership-probe.service", check=False, timeout=600)
    expect(vm.run("sudo journalctl -u mothership-probe --no-pager"), "Pushed")
    expect(apply(), "Updated app")
    assert data("app", "version") == "1.0.3"
    log = vm.run(f"git -C {FLEET} log -1 --format='%an %s'").stdout
    assert log.startswith("minipc/probe Update 1 image"), log


if __name__ == "__main__":
    sys.exit(main())

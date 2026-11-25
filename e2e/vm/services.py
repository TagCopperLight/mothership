#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["pyrage", "pyyaml"]
# ///

import hashlib
import http.server
import json
import re
import shutil
import socket
import ssl
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import uuid
from functools import partial
from pathlib import Path

import pyrage
import yaml
from pyrage import x25519

HERE = Path(__file__).resolve().parent
E2E = HERE.parent
sys.path.insert(0, str(E2E))
import binary

CACHE = Path.home() / ".cache/mothership-vm"
CLOUD_IMAGE = CACHE / "debian-13-genericcloud-amd64.qcow2"
CHEZMOI = CACHE / "chezmoi_2.74.2_linux_amd64.deb"
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
STAND_IN = "stand0"
LINK = "link.app.app.extra.extra"


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
        self.tags: dict[tuple[str, str], str] = {}
        self.media: dict[str, str] = {}
        self.uploads: dict[str, bytearray] = {}
        self.built: dict[str, str] = {}
        for version in VERSIONS:
            digest, media = build(tmp, version, self.blobs)
            self.built[version] = digest
            self.media[digest] = media
        for version in VERSIONS[:3]:
            self.publish(version)

    def publish(self, version: str) -> None:
        self.tags[("app", version)] = self.built[version]

    def digest(self, version: str) -> str:
        return self.built[version]

    def store(self, data: bytes) -> str:
        digest = "sha256:" + hashlib.sha256(data).hexdigest()
        (self.blobs / digest.split(":")[1]).write_bytes(data)
        return digest

    def handler(self):
        registry = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *args) -> None:
                pass

            def do_HEAD(self) -> None:
                self.answer("HEAD")

            def do_GET(self) -> None:
                self.answer("GET")

            def do_POST(self) -> None:
                self.answer("POST")

            def do_PATCH(self) -> None:
                self.answer("PATCH")

            def do_PUT(self) -> None:
                self.answer("PUT")

            def read_body(self) -> bytes:
                if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
                    data = bytearray()
                    while size := int(self.rfile.readline().split(b";")[0].strip(), 16):
                        data += self.rfile.read(size)
                        self.rfile.readline()
                    self.rfile.readline()
                    return bytes(data)
                return self.rfile.read(int(self.headers.get("Content-Length") or 0))

            def answer(self, method: str) -> None:
                path, _, query = self.path.partition("?")
                params = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
                body = self.read_body() if method in ("POST", "PATCH", "PUT") else b""
                parts = path.strip("/").split("/")
                if parts == ["v2"]:
                    return self.send(200, b"{}", "application/json", method)
                kind = next((i for i, p in enumerate(parts) if i > 1 and p in
                             ("manifests", "blobs", "tags")), None)
                if kind is None:
                    return self.send(404, b"{}", "application/json", method)
                name, rest = "/".join(parts[1:kind]), parts[kind:]
                if rest == ["tags", "list"]:
                    tags = sorted(tag for (repo, tag) in registry.tags if repo == name)
                    data = json.dumps({"name": name, "tags": tags}).encode()
                    return self.send(200, data, "application/json", method)
                if rest[0] == "manifests" and len(rest) == 2:
                    reference = rest[1]
                    if method == "PUT":
                        digest = registry.store(body)
                        registry.media[digest] = self.headers["Content-Type"]
                        if not reference.startswith("sha256:"):
                            registry.tags[(name, reference)] = digest
                        return self.send(201, b"", "text/plain", method, digest)
                    digest = registry.tags.get((name, reference), reference)
                    if digest not in registry.media:
                        return self.send(404, b"{}", "application/json", method)
                    data = (registry.blobs / digest.split(":")[1]).read_bytes()
                    return self.send(200, data, registry.media[digest], method, digest)
                if rest[:2] == ["blobs", "uploads"]:
                    if method == "POST":
                        upload = uuid.uuid4().hex
                        registry.uploads[upload] = bytearray(body)
                    else:
                        upload = rest[2]
                        registry.uploads[upload] += body
                    if method == "PUT":
                        data = bytes(registry.uploads.pop(upload))
                        digest = registry.store(data)
                        assert digest == params["digest"].replace("%3A", ":"), digest
                        return self.send(201, b"", "text/plain", method, digest,
                                         f"/v2/{name}/blobs/{digest}")
                    size = len(registry.uploads[upload])
                    return self.send(202, b"", "text/plain", method, None,
                                     f"/v2/{name}/blobs/uploads/{upload}", f"0-{max(size - 1, 0)}")
                if rest[0] == "blobs" and len(rest) == 2:
                    blob = registry.blobs / rest[1].split(":")[-1]
                    if blob.exists():
                        return self.send(200, blob.read_bytes(), "application/octet-stream",
                                         method, rest[1])
                self.send(404, b"{}", "application/json", method)

            def send(self, status, data, media, method, digest=None, location=None,
                     range=None) -> None:
                self.send_response(status)
                self.send_header("Content-Type", media)
                self.send_header("Content-Length", str(len(data)))
                if digest:
                    self.send_header("Docker-Content-Digest", digest)
                if location:
                    self.send_header("Location", location)
                if range:
                    self.send_header("Range", range)
                self.end_headers()
                if method != "HEAD":
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
    return hashlib.sha256((user_data() + CHEZMOI.name).encode()).hexdigest()


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
    signing = " ".join((E2E / "signing.pub").read_text().split()[:2])
    (work / "machines").mkdir()
    (work / "machines/minipc.toml").write_text(
        f'type = "server"\nsigns = "everything"\n\n[keys]\nsigning = "{signing}"\nage = "{root.to_public()}"\n')
    (work / "shared/.secrets").mkdir(parents=True)
    (work / "shared/.secrets/token.age").write_bytes(
        pyrage.encrypt(f"{TOKEN}\n".encode(), [root.to_public(), recovery]))
    (server / "services/app/secrets").mkdir()
    (server / "services/app/secrets/token.age").symlink_to("../../../../../shared/.secrets/token.age")
    (server / "services/app/service.toml").write_text(
        pinned(image, registry, "1.0.0", mine=True, callers=["extra/extra"]))
    for other in ["extra", "lonely"]:
        (server / f"services/{other}/service.toml").write_text(
            pinned(image, registry, "1.0.0", mine=False).replace("[images.app]", f"[images.{other}]"))
    archive = tmp / "fleet.tar"
    with tarfile.open(archive, "w") as tar:
        tar.add(work, arcname=".")
    return archive


def pinned(image: str, registry: Registry, version: str, mine: bool,
           callers: list[str] = []) -> str:
    repo = ('repo = "me/app"\nports.public = ["80/tcp", "443/udp"]\nports.local = ["5432/tcp"]\n'
            f'ports.{STAND_IN} = ["53/udp"]\n' if mine else "")
    if callers:
        repo += f"callers = {json.dumps(callers)}\n"
    snapshot = 'snapshot = ["precious"]\n\n' if mine else ""
    return (f'{snapshot}[images.app]\nimage = "{image}"\nversion = "{version}"\n'
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
printf 'name = "minipc"\\n' | sudo install -D -m 644 /dev/stdin /etc/mothership/machine.toml
install -m 600 /tmp/ssh ~/.ssh/id_ed25519
install -m 644 /tmp/ssh.pub ~/.ssh/id_ed25519.pub
ssh-keyscan -q localhost | sudo tee /etc/ssh/ssh_known_hosts > /dev/null
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
sudo ip link add {STAND_IN} type dummy
sudo ip address add 10.99.0.1/24 dev {STAND_IN}
sudo ip link set {STAND_IN} up
""")
    apply = lambda: vm.run("sudo mothership apply", input="y\n" * 6, check=False, timeout=1800)
    data = partial(data_of, vm)

    expect(apply(), "Started app", "Started extra")
    assert data("app", "version") == "1.0.0"
    assert data("app", "token") == f"{TOKEN}\n", "the secret reaches /run/secrets/token"
    assert data("app", "conf") == f"token = {TOKEN}\n", "the template is filled in"
    assert data("app", "precious") == "kept"
    expect(vm.run("stat -c '%U %a' /srv/app /var/lib/mothership/services/app"),
           "services 700", "root 750")
    ports(vm, image, registry)
    links(vm, image)
    expect(apply(), "probe  install and start it")
    expect(apply(), "Nothing to apply.")
    stray_link(vm, apply)
    scan = vm.run("sudo mothership scan").stdout
    assert "Containers" not in scan, scan
    interface(vm, apply)
    assert linked(vm), "the link works again once Docker and stand0 come back"

    def commit(message: str, change: str) -> None:
        vm.run(f"cd {FLEET} && {change}")
        expect(vm.run(f"sudo mothership commit '{message}'", input="y\n"), "Committed")

    callers = ["extra/extra"]

    def pin(version: str) -> str:
        text = pinned(image, registry, version, mine=True, callers=callers)
        return f"cat > types/server/services/app/service.toml <<'EOF'\n{text}EOF"

    expect(vm.run("sudo mothership secret app token", input="n3w-token\ny\n"), "Committed")
    link = vm.run(f"readlink {FLEET}/types/server/services/app/secrets/token.age").stdout
    assert link.strip() == "../../../../../shared/.secrets/token.age", link
    expect(apply(), "Updated app")
    assert data("app", "token") == "n3w-token", "the secret command reaches the container"
    assert data("app", "conf") == "token = n3w-token\n"

    commit("Update app", pin("1.0.1"))
    expect(apply(), "Updated app")
    assert data("app", "version") == "1.0.1"
    gone = vm.run("sudo test -e /var/lib/services/snapshots/app", check=False).returncode
    assert gone != 0, "apply deletes the snapshot once the update ends"

    extra = "types/server/services/extra"
    commit("Publish from compose", f"echo '    ports: [\"8080:80\"]' >> {extra}/compose.yaml")
    expect(apply(), "extra publishes ports in compose.yaml: declare them in service.toml", code=1)
    commit("Publish nothing from compose", f"git checkout HEAD~1 -- {extra}/compose.yaml")
    expect(apply(), "Nothing to apply.")
    commit("Use the host network", f"echo '    network_mode: host' >> {extra}/compose.yaml")
    expect(apply(), "extra sets network_mode: a container stays on its service's networks", code=1)
    commit("Stay on the service network", f"git checkout HEAD~1 -- {extra}/compose.yaml")
    expect(apply(), "Nothing to apply.")
    commit("Take port 80", f"echo 'ports.public = [\"80/tcp\"]' >> {extra}/service.toml")
    expect(apply(), "app's images.app and extra's images.extra both publish 80/tcp on public", code=1)
    commit("Give port 80 back", f"git checkout HEAD~1 -- {extra}/service.toml")
    expect(apply(), "Nothing to apply.")
    commit("Publish 8443/tcp", "sed -i 's/\"443\\/udp\"/\"443\\/udp\", \"8443\\/tcp\"/' "
           "types/server/services/app/service.toml")
    expect(apply(), "Updated app")
    assert running(vm, "extra"), "apply starts a service without a restart policy after Docker restarts"
    assert linked(vm), "the link works again after apply restarts Docker"
    commit("Stop publishing 8443/tcp", "git checkout HEAD~1 -- types/server/services/app/service.toml")
    expect(apply(), "Updated app")

    commit("Drop extra", "git rm -q -r types/server/services/extra types/server/services/lonely")
    expect(apply(), "app's images.app lets extra call it, and the type has no service extra", code=1)
    callers.clear()
    commit("Drop the link to extra", pin("1.0.1"))
    expect(apply(), "Removed extra, keeping /srv/extra", "Removed lonely", "Updated app")
    assert LINK not in vm.run(f"{AS_SERVICES} network ls").stdout, "apply removes a link nobody needs"
    assert data("extra", "version") == "1.0.0", "a dropped service keeps its data"
    containers = vm.run("sudo -u services env DOCKER_HOST=unix:///run/user/$(id -u services)/docker.sock "
                        "docker ps --all --format '{{.Names}}'").stdout
    assert "extra" not in containers and "app" in containers, containers
    vm.run("sudo mkdir /srv/stray")
    expect(vm.run("sudo mothership scan"), "/srv/extra", "/srv/stray")

    commit("Break app", pin("1.0.2"))
    expect(apply(), "app failed, so it runs its last healthy versions again", "these services need attention", code=1)
    assert data("app", "version") == "1.0.1", "the last healthy version runs again"
    assert data("app", "precious") == "kept", "the snapshot is back"
    expect(vm.run("sudo mothership scan"), "not applied", "images.app 1.0.2 failed (app-app-1 reports unhealthy)")
    expect(apply(), "these services need attention", code=1)

    scan = vm.run("sudo mothership scan").stdout
    trust = re.search(r"`(sudo mothership trust [^`]*)`", scan).group(1)
    probe_ssh = re.search(r"'probe-ssh=([^']*)'", trust).group(1)
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

    commit("Stop publishing 443/udp", "sed -i 's/, \"443\\/udp\"//' types/server/services/app/service.toml")
    expect(apply(), "Updated app")
    assert reach(vm, "tcp", 80) == "1.0.3"
    assert reach(vm, "udp", 443) == "", "a port the fleet no longer declares is closed"
    allowed = vm.run("systemctl show -p SocketBindAllow mothership-services").stdout
    assert "udp443" not in allowed and "tcp80" in allowed, allowed
    refused(vm, f"{image}:1.0.3@{registry.digest('1.0.3')}", "443:443/udp")

    recipes(vm, image, commit, apply)
    restart_manager(vm)
    last_service(vm, commit, apply)


def links(vm: VM, image: str) -> None:
    assert linked(vm), "extra calls app by its link name"
    expect(vm.run(f"{AS_SERVICES} network inspect --format '{{{{.Internal}}}} "
                  f"{{{{range .IPAM.Config}}}}{{{{.Subnet}}}}{{{{end}}}}' {LINK}"), "true 172.")
    assert vm.run(f"{AS_SERVICES} network inspect --format '{{{{range .IPAM.Config}}}}{{{{.Subnet}}}}"
                  f"{{{{end}}}}' {LINK}").stdout.strip().endswith("/24"), "links take small subnets"
    address = vm.run(f"{AS_SERVICES} inspect --format "
                     f"'{{{{(index .NetworkSettings.Networks \"{LINK}\").IPAddress}}}}' app-app-1").stdout.strip()
    assert address, "app sits on its link network"
    for target in ["app.app:80", f"{address}:80"]:
        out = vm.run(f"{AS_SERVICES} exec lonely-lonely-1 /app reach {target}", check=False)
        assert out.returncode == 1, f"a service without a link can't reach app at {target}: {out.stdout}"
    outside = image.split("/")[0]
    expect(vm.run(f"{AS_SERVICES} exec extra-extra-1 /app reach {outside}"), "reached")
    expect(vm.run(f"{AS_SERVICES} exec app-app-1 /app reach {outside}"), "reached")

    vm.run("sudo systemd-run --unit host-listener --property=RuntimeMaxSec=300 "
           "python3 -m http.server --bind 10.99.0.1 8000")
    try:
        deadline = time.time() + 30
        while ":8000 " not in vm.run("ss -tln").stdout and time.time() < deadline:
            time.sleep(1)
        out = vm.run(f"{AS_SERVICES} exec extra-extra-1 /app reach 10.99.0.1:8000", check=False)
        assert out.stdout.strip() == "reached", \
            f"a container reaches a program on the machine through {STAND_IN}: {out.stdout}{out.stderr}"
    finally:
        vm.run("sudo systemctl stop host-listener", check=False)


def stray_link(vm: VM, apply) -> None:
    vm.run(f"{AS_SERVICES} network create --internal link.app.app.stray && "
           f"{AS_SERVICES} network connect link.app.app.stray lonely-lonely-1")
    expect(vm.run("sudo mothership scan"), "link.app.app.stray  disconnect its containers and remove it")
    apply()
    networks = vm.run(f"{AS_SERVICES} network ls").stdout
    assert "link.app.app.stray" not in networks and LINK in networks, networks
    assert running(vm, "lonely"), "removing a link network disconnects its containers first"


def linked(vm: VM) -> bool:
    out = vm.run(f"{AS_SERVICES} exec extra-extra-1 /app reach app.app:80", check=False)
    return out.stdout.strip() == "reached"


def last_service(vm: VM, commit, apply) -> None:
    uid = vm.run("id -u services").stdout.strip()
    commit("Drop every service and Docker", "git rm -q -r types/server/services && sed -i "
           "'/\"docker-compose\"\\|\"docker.io\"\\|\"rootlesskit\"/d' types/server/system.toml")
    expect(vm.run("sudo mothership scan"), "the type runs no service")
    expect(apply(), "Removed app", "Removed tool", "Removed the services account")
    expect(vm.run("id services", check=False), "no such user", code=1)
    left = vm.run("ls -d /var/lib/services /var/lib/systemd/linger/services "
                  "/etc/systemd/system/mothership-services* "
                  f"/etc/systemd/system/*.wants/mothership-services* /run/user/{uid}; "
                  "grep -h services /etc/subuid /etc/subgid /etc/group", check=False).stdout
    assert left == "", f"apply removes everything it set up for services: {left}"
    assert data_of(vm, "app", "version") == "1.0.3", "the data stays"
    expect(vm.run("sudo mothership scan"), "/srv/app", "no service of the fleet owns it")
    expect(apply(), "Nothing to apply.")


def interface(vm: VM, apply) -> None:
    vm.run(f"sudo ip link del {STAND_IN} && sudo systemctl restart mothership-services && sleep 5")
    expect(vm.run("sudo mothership scan"), "stand0 has no address yet")
    expect(apply(), "app: it waits for stand0 to have an address", code=1)
    assert running(vm, "extra"), "Docker's start starts every service, restart policy or not"
    assert not running(vm, "app"), "app waits for stand0"
    vm.run(f"sudo ip link add {STAND_IN} type dummy && sleep 40")
    waiting = vm.run(f"systemctl is-active mothership-services-start@{STAND_IN}", check=False)
    assert waiting.stdout.strip() == "activating", "the start unit waits as long as stand0 has no address"
    vm.run(f"sudo flock /var/lib/mothership/lock sh -c 'ip address add 10.99.0.2/24 dev {STAND_IN} && "
           f"ip link set {STAND_IN} up && sleep 10'")
    deadline = time.time() + 90
    while reach(vm, "udp", 53, "10.99.0.2") != "1.0.0":
        if time.time() > deadline:
            sys.exit(f"app didn't start when {STAND_IN} came up: see {LOG}")
        time.sleep(3)
    vm.run(f"sudo ip address replace 10.99.0.3/24 dev {STAND_IN} && "
           f"sudo ip address del 10.99.0.2/24 dev {STAND_IN}")
    expect(vm.run("sudo mothership scan"), "other addresses")
    expect(apply(), "app")
    assert reach(vm, "udp", 53, "10.99.0.3") == "1.0.0", "apply binds app to stand0's new address"
    vm.run(f"{AS_SERVICES} stop extra-extra-1")
    expect(vm.run("sudo mothership scan"), "extra", "start it")
    apply()
    assert running(vm, "extra"), "apply starts a service whose containers all stopped"


def restart_manager(vm: VM) -> None:
    uid = vm.run("id -u services").stdout.strip()
    vm.run(f"sudo systemctl kill --signal=SIGKILL user@{uid}.service; sleep 5")
    vm.run(f"sudo systemctl start user@{uid}.service")
    deadline = time.time() + 120
    while reach(vm, "tcp", 80) != "1.0.3":
        if time.time() > deadline:
            sys.exit(f"app didn't come back after services' user manager restarted: see {LOG}")
        time.sleep(5)


AS_SERVICES = "sudo -u services env DOCKER_HOST=unix:///run/user/$(id -u services)/docker.sock docker"


def reach(vm: VM, protocol: str, port: int, host: str = "127.0.0.1") -> str:
    kind = "SOCK_STREAM" if protocol == "tcp" else "SOCK_DGRAM"
    family = "AF_INET6" if ":" in host else "AF_INET"
    return vm.run(f"""python3 -c '
import socket
s = socket.socket(socket.{family}, socket.{kind})
s.settimeout(3)
try:
    s.connect(("{host}", {port}))
    s.send(b"hi")
    print(s.recv(64).decode(), end="")
except OSError:
    pass
'""").stdout


def ports(vm: VM, image: str, registry: Registry) -> None:
    assert data_of(vm, "app", "ports") == "80/tcp ok\n5432/tcp ok\n443/udp ok\n53/udp ok\n", \
        "a container opens any port inside its own network"
    lan = vm.run("ip -4 -o route get 1.1.1.1").stdout.split(" src ")[1].split()[0]
    assert reach(vm, "tcp", 80) == "1.0.0", "a declared low TCP port works"
    assert reach(vm, "tcp", 80, lan) == "1.0.0", "a public port answers on the LAN"
    assert reach(vm, "udp", 443) == "1.0.0", "a declared low UDP port works"
    assert reach(vm, "tcp", 5432) == reach(vm, "tcp", 5432, "::1") == "1.0.0", "a local port answers"
    assert reach(vm, "tcp", 5432, lan) == "", "a local port doesn't answer on the LAN"
    assert reach(vm, "udp", 53, "10.99.0.1") == "1.0.0", "a port facing an interface answers there"
    for host in [lan, "127.0.0.1"]:
        assert reach(vm, "udp", 53, host) == "", f"a port facing an interface is closed on {host}"
    expect(vm.run("sudo ss -ulnp"), "127.0.0.53%lo:53", "10.99.0.1:53")
    denied = vm.run("python3 -c 'import socket; socket.socket().bind((\"0.0.0.0\", 81))'", check=False)
    expect(denied, "Permission denied", code=1)
    reference = f"{image}:1.0.0@{registry.digest('1.0.0')}"
    for published in ["8080:80", "444:80", "80:80/udp", "80"]:
        refused(vm, reference, published)
    outside = image.split("/")[0]
    expect(vm.run(f"{AS_SERVICES} run --rm --entrypoint /app {reference} reach {outside}"), "reached")


def refused(vm: VM, reference: str, published: str) -> None:
    out = vm.run(f"{AS_SERVICES} run --detach --name stray --publish {published} {reference}",
                 check=False)
    vm.run(f"{AS_SERVICES} rm --force stray", check=False)
    expect(out, "bind: operation not permitted", code=125)


def running(vm: VM, service: str) -> bool:
    states = vm.run(f"{AS_SERVICES} ps --all --filter label=com.docker.compose.project={service} "
                    "--format '{{.State}}'").stdout.split()
    return bool(states) and all(state == "running" for state in states)


def data_of(vm: VM, service: str, name: str) -> str:
    return vm.run(f"sudo cat /srv/{service}/{name}").stdout


WORKFLOW = HERE.parent.parent / "src/services/recipes.yml"


def recipes(vm: VM, image: str, commit, apply) -> None:
    tool = image.replace("/app", "/tool")
    folder = "types/server/services/tool"

    def dockerfile(version: str) -> str:
        return f'FROM {image}:1.0.1\nLABEL org.opencontainers.image.version="{version}"\n'

    commit("Add tool", f"""mkdir -p {folder}
cat > {folder}/Dockerfile <<'END'
{dockerfile("2.0.0")}END
cat > {folder}/service.toml <<'END'
[images.tool]
image = "{tool}"
version = "2.0.0"
recipe = true
END
cat > {folder}/compose.yaml <<'END'
services:
  tool:
    restart: unless-stopped
    volumes:
      - /srv/tool:/data
END""")
    expect(vm.run("sudo mothership scan"), "not built yet", "mothership-recipes.yml")
    workflow = WORKFLOW.read_text()
    vm.run(f"mkdir -p {FLEET}/.github/workflows && "
           f"cat > {FLEET}/.github/workflows/mothership-recipes.yml", input=workflow)
    expect(vm.run("sudo mothership commit 'Add the recipes workflow'", input="y\n"), "Committed")
    assert "mothership-recipes.yml" not in vm.run("sudo mothership scan").stdout
    steps = yaml.safe_load(workflow)["jobs"]["build"]["steps"]
    script = next(step["run"] for step in steps if step.get("name") == "Build the recipes")

    vm.run("sudo systemctl restart docker")
    head = vm.run(f"git -C {FLEET} rev-parse HEAD").stdout.strip()

    def build(sha: str) -> subprocess.CompletedProcess:
        return vm.run(f"cd {FLEET} && sudo env EVENT=workflow_dispatch GITHUB_SHA={sha} "
                      "GITHUB_SERVER_URL=https://github.com GITHUB_REPOSITORY=me/fleet bash -s",
                      input=script)

    expect(build(head), f"Building server/tool as {tool}:2.0.0", "Built 1 recipes")
    vm.run("sudo systemctl start mothership-probe.service", check=False, timeout=600)
    expect(vm.run(f"git -C {FLEET} fetch -q origin && git -C {FLEET} log -1 --format=%B origin/main"),
           "server/tool tool: 2.0.0 with a new digest")
    expect(apply(), "Started tool")
    assert vm.run("sudo cat /srv/tool/version").stdout == "1.0.1", "the recipe's base runs"

    vm.run(f"cd {FLEET} && cat > {folder}/Dockerfile <<'END'\n{dockerfile('2.0.1')}END")
    expect(build("f" * 40), f"{tool}:2.0.1")
    vm.run(f"git -C {FLEET} checkout -- {folder}/Dockerfile")
    vm.run("sudo systemctl start mothership-probe.service", check=False, timeout=600)
    expect(vm.run("sudo journalctl -u mothership-probe --no-pager"),
           "was built from ffffffffffff, which isn't on the fleet's main")
    pinned = vm.run(f"git -C {FLEET} fetch -q origin && "
                    f"git -C {FLEET} show origin/main:{folder}/service.toml").stdout
    assert 'version = "2.0.0"' in pinned, pinned


if __name__ == "__main__":
    sys.exit(main())

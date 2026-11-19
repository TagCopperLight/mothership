#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["pexpect"]
# ///



import http.server
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from collections.abc import Iterator
from contextlib import contextmanager
from functools import partial
from pathlib import Path

import pexpect

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
CACHE = Path.home() / ".cache/mothership-vm"
ISO = CACHE / "archlinux-x86_64.iso"
OVMF = Path("/usr/share/edk2/x64")
MIRRORS = ["https://mirror.xtom.com.hk/archlinux", "https://mirrors.aliyun.com/archlinux"]
PACKAGES = CACHE / "packages"
PASSPHRASE = "disk-passphrase"
PASSWORD = "tester-password"
HOST = "10.0.2.2"
INSTALL_TIMEOUT = 3600


def main() -> int:
    if not ISO.exists():
        sys.exit(f"{ISO} is missing")
    subprocess.run(["cargo", "build", "--release", "--locked", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="mothership-vm-") as tmp:
        tmp = Path(tmp)
        port = serve(site(tmp / "site"))
        boot_files = iso_boot_files(tmp)
        shutil.copy(OVMF / "OVMF_VARS.4m.fd", tmp / "vars.fd")
        subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", str(tmp / "disk.qcow2"), "40G"],
                       check=True)
        with open(CACHE / "run.log", "w") as log:
            with tpm(tmp):
                install(qemu(tmp, boot_files), port, log)
            with tpm(tmp):
                first_start(qemu(tmp), log)
            with tpm(tmp):
                tpm_start(qemu(tmp), log)
    print("The VM installs, finishes at its first start, and opens its disk with the TPM.")
    return 0


def site(dir: Path) -> Path:
    dir.mkdir()
    work = dir.parent / "fleet"
    shutil.copytree(HERE / "fleet", work)
    git = ["git", "-c", "user.name=Test", "-c", "user.email=test@example.com"]
    subprocess.run([*git, "init", "-q", "-b", "main"], cwd=work, check=True)
    subprocess.run([*git, "add", "-A"], cwd=work, check=True)
    subprocess.run([*git, "commit", "-qm", "The fleet"], cwd=work, check=True)
    subprocess.run(["git", "clone", "-q", "--bare", str(work), str(dir / "fleet.git")], check=True)
    subprocess.run(["git", "update-server-info"], cwd=dir / "fleet.git", check=True)
    shutil.copy(ROOT / "target/release/mothership", dir / "mothership")
    PACKAGES.mkdir(exist_ok=True)
    for repo in ["core", "extra"]:
        packages = dir / repo / "os/x86_64"
        packages.mkdir(parents=True)
        url = f"{MIRRORS[0]}/{repo}/os/x86_64/{repo}.db"
        with urllib.request.urlopen(url, timeout=60) as db:
            (packages / f"{repo}.db").write_bytes(db.read())
        for package in Path("/var/cache/pacman/pkg").iterdir():
            if not package.name.endswith(".part"):
                (packages / package.name).symlink_to(package)
    return dir


class Handler(http.server.SimpleHTTPRequestHandler):

    def log_message(self, *args) -> None:
        pass

    def do_GET(self) -> None:
        parts = self.path.lstrip("/").split("/")
        local = Path(self.translate_path(self.path))
        if len(parts) != 4 or parts[1:3] != ["os", "x86_64"] or local.exists():
            return super().do_GET()
        cached = PACKAGES / parts[3]
        if cached.exists():
            return self.send(cached)
        for mirror in MIRRORS:
            try:
                upstream = urllib.request.urlopen(f"{mirror}{self.path}", timeout=30)
            except OSError:
                continue
            with upstream, tempfile.NamedTemporaryFile(dir=PACKAGES, delete=False) as part:
                self.send_response(200)
                if length := upstream.headers["Content-Length"]:
                    self.send_header("Content-Length", length)
                self.end_headers()
                try:
                    while chunk := upstream.read(1 << 16):
                        part.write(chunk)
                        self.wfile.write(chunk)
                except OSError:
                    Path(part.name).unlink()
                    raise
            Path(part.name).rename(cached)
            return
        self.send_error(404)

    def send(self, path: Path) -> None:
        self.send_response(200)
        self.send_header("Content-Length", str(path.stat().st_size))
        self.end_headers()
        with open(path, "rb") as file:
            shutil.copyfileobj(file, self.wfile)


def serve(dir: Path) -> int:
    handler = partial(Handler, directory=str(dir))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server.server_address[1]


def iso_boot_files(tmp: Path) -> tuple[Path, Path]:
    members = ["arch/boot/x86_64/vmlinuz-linux", "arch/boot/x86_64/initramfs-linux.img"]
    subprocess.run(["bsdtar", "-xf", str(ISO), "-C", str(tmp), *members], check=True)
    return tmp / members[0], tmp / members[1]


@contextmanager
def tpm(tmp: Path) -> Iterator[None]:
    (tmp / "tpm").mkdir(exist_ok=True)
    socket = tmp / "swtpm.sock"
    process = subprocess.Popen(
        ["swtpm", "socket", "--tpm2", "--tpmstate", f"dir={tmp / 'tpm'}",
         "--ctrl", f"type=unixio,path={socket}"])
    try:
        for _ in range(100):
            if socket.exists():
                break
            time.sleep(0.1)
        else:
            sys.exit("swtpm didn't start")
        yield
    finally:
        process.terminate()
        process.wait(timeout=10)


def qemu(tmp: Path, boot_files=None) -> list[str]:
    args = [
        "qemu-system-x86_64", "-enable-kvm", "-machine", "q35,smm=on", "-cpu", "host",
        "-m", "4096", "-smp", "4", "-nographic", "-serial", "mon:stdio", "-no-reboot",
        "-global", "driver=cfi.pflash01,property=secure,value=on",
        "-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={OVMF / 'OVMF_CODE.secboot.4m.fd'}",
        "-drive", f"if=pflash,format=raw,unit=1,file={tmp / 'vars.fd'}",
        "-chardev", f"socket,id=tpm,path={tmp / 'swtpm.sock'}",
        "-tpmdev", "emulator,id=tpm0,chardev=tpm", "-device", "tpm-tis,tpmdev=tpm0",
        "-drive", f"file={tmp / 'disk.qcow2'},if=virtio,format=qcow2",
        "-nic", "user,model=virtio-net-pci",
    ]
    if boot_files:
        kernel, initrd = boot_files
        args += ["-cdrom", str(ISO), "-kernel", str(kernel), "-initrd", str(initrd),
                 "-append", "archisobasedir=arch archisodevice=/dev/sr0 console=ttyS0,115200"]
    return args


def spawn(args: list[str], log) -> pexpect.spawn:
    vm = pexpect.spawn(args[0], args[1:], encoding="utf-8", codec_errors="replace", timeout=600)
    vm.logfile_read = log
    return vm


def own_prompt(vm: pexpect.spawn) -> None:
    vm.sendline("exec bash --norc --noprofile")
    vm.sendline("PS1='V''M# '")
    vm.expect("VM# ")


def wait(vm: pexpect.spawn, text: str, timeout: int = 600) -> None:
    if vm.expect([text, r"\nError: [^\r\n]+\r*\n"], timeout=timeout) == 1:
        sys.exit(f"install failed: {vm.after.strip()}")


def shell(vm: pexpect.spawn, command: str, timeout: int = 600) -> None:
    vm.sendline(command)
    vm.expect("VM# ", timeout=timeout)


def install(args: list[str], port: int, log) -> None:
    with spawn(args, log) as vm:
        vm.expect("archiso login:", timeout=300)
        vm.sendline("root")
        vm.expect("# ")
        own_prompt(vm)
        site = f"http://{HOST}:{port}"
        shell(vm, f"echo 'Server = {site}/$repo/os/$arch' > /etc/pacman.d/mirrorlist")
        shell(vm, f"curl -fsS -o /root/mothership {site}/mothership && chmod +x /root/mothership")
        vm.sendline(f"/root/mothership install --fleet {site}/fleet.git --type vm --option encrypted"
                    " --hostname vmtest --timezone Europe/Paris --disk /dev/vda --layout wipe")
        wait(vm, "Type vda to go on:")
        vm.sendline("vda")
        for question in ["Disk passphrase:", "Again:"]:
            wait(vm, question)
            vm.sendline(PASSPHRASE)
        wait(vm, "tester's password:", timeout=INSTALL_TIMEOUT)
        for question in ["New password:", "Retype new password:"]:
            wait(vm, question)
            vm.sendline(PASSWORD)
        wait(vm, "r type the recovery key")
        vm.send("r")
        wait(vm, "Recovery key:")
        secret = next(line for line in (HERE / "recovery.txt").read_text().splitlines()
                      if line.startswith("AGE-SECRET-KEY-"))
        vm.sendline(secret)
        wait(vm, "Secure Boot holds this machine's keys now.", timeout=INSTALL_TIMEOUT)
        wait(vm, "The system is installed.", timeout=300)
        vm.expect("VM# ")
        vm.sendline("poweroff")
        vm.expect(pexpect.EOF, timeout=120)


def first_start(args: list[str], log) -> None:
    with spawn(args, log) as vm:
        vm.expect("[Pp]assphrase for", timeout=300)
        vm.sendline(PASSPHRASE)
        wait(vm, "Type the passphrase once more:", timeout=1200)
        wait(vm, "[Pp]assphrase", timeout=120)
        vm.sendline(PASSPHRASE)
        wait(vm, "Nothing differs from the vm type")
        vm.expect("vmtest login:", timeout=300)
        login(vm)
        shell(vm, "cat ~/.secret && sudo -S cat /etc/secret.conf <<< " + PASSWORD)
        if "tester's secret" not in vm.before or "root-only secret" not in vm.before:
            sys.exit("the secrets didn't open")
        shell(vm, "sudo systemd-cryptenroll /dev/vda2")
        if "tpm2" not in vm.before:
            sys.exit("the TPM holds no key for the disk")
        vm.sendline("sudo poweroff")
        vm.expect(pexpect.EOF, timeout=120)


def tpm_start(args: list[str], log) -> None:
    with spawn(args, log) as vm:
        found = vm.expect(["vmtest login:", "[Pp]assphrase for"], timeout=300)
        if found == 1:
            sys.exit("the disk asked for its passphrase: the TPM didn't open it")
        login(vm)
        vm.sendline(f"sudo -S poweroff <<< {PASSWORD}")
        vm.expect(pexpect.EOF, timeout=120)


def login(vm: pexpect.spawn) -> None:
    vm.sendline("tester")
    vm.expect("Password:")
    vm.sendline(PASSWORD)
    vm.expect(r"\$ ")
    own_prompt(vm)


if __name__ == "__main__":
    sys.exit(main())

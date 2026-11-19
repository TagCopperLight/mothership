#!/usr/bin/env -S uv run --script
# /// script
# dependencies = ["pexpect"]
# ///
"""Installs fleet/ from the Arch ISO into a VM with Secure Boot in Setup Mode and a TPM, then
checks the first start finishes the install and the second opens the disk with the TPM alone.

The ISO is ~/.cache/mothership-vm/archlinux-x86_64.iso. pacman downloads from this machine's
package cache first, then from a mirror. Needs qemu-desktop, edk2-ovmf and swtpm.

Usage: e2e/vm/run.py
"""

import http.server
import shutil
import subprocess
import sys
import tempfile
import threading
from functools import partial
from pathlib import Path

import pexpect

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
CACHE = Path.home() / ".cache/mothership-vm"
ISO = CACHE / "archlinux-x86_64.iso"
OVMF = Path("/usr/share/edk2/x64")
MIRROR = "https://mirror.xtom.com.hk/archlinux/$repo/os/$arch"
PASSPHRASE = "disk-passphrase"
PASSWORD = "tester-password"
# qemu's user network reaches the machine running it there.
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
        tpm = start_tpm(tmp)
        try:
            shutil.copy(OVMF / "OVMF_VARS.4m.fd", tmp / "vars.fd")
            subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", str(tmp / "disk.qcow2"), "40G"],
                           check=True)
            log = open(CACHE / "run.log", "w")
            install(qemu(tmp, boot_files), port, log)
            first_start(qemu(tmp), log)
            tpm_start(qemu(tmp), log)
        finally:
            tpm.terminate()
            tpm.wait(timeout=10)
    print("The VM installs, finishes at its first start, and opens its disk with the TPM.")
    return 0


def site(dir: Path) -> Path:
    """The fleet as a bare repository git reads over plain HTTP, the binary, and a mirror."""
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
    for repo in ["core", "extra"]:
        packages = dir / repo / "os/x86_64"
        packages.mkdir(parents=True)
        (packages / f"{repo}.db").symlink_to(f"/var/lib/pacman/sync/{repo}.db")
        for package in Path("/var/cache/pacman/pkg").iterdir():
            if not package.name.endswith(".part"):
                (packages / package.name).symlink_to(package)
    return dir


def serve(dir: Path) -> int:
    handler = partial(http.server.SimpleHTTPRequestHandler, directory=str(dir))
    handler.log_message = lambda *args: None
    # qemu's user network reaches the host's loopback, so nothing else on the network can.
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server.server_address[1]


def iso_boot_files(tmp: Path) -> tuple[Path, Path]:
    """The ISO's kernel and initramfs, started directly so its console is the serial port."""
    members = ["arch/boot/x86_64/vmlinuz-linux", "arch/boot/x86_64/initramfs-linux.img"]
    subprocess.run(["bsdtar", "-xf", str(ISO), "-C", str(tmp), *members], check=True)
    return tmp / members[0], tmp / members[1]


def start_tpm(tmp: Path) -> subprocess.Popen:
    (tmp / "tpm").mkdir()
    return subprocess.Popen(
        ["swtpm", "socket", "--tpm2", "--tpmstate", f"dir={tmp / 'tpm'}",
         "--ctrl", f"type=unixio,path={tmp / 'swtpm.sock'}"])


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
    """bash with a prompt the harness waits for. The quotes keep the echoed command from
    matching it."""
    vm.sendline("exec bash --norc --noprofile")
    vm.sendline("PS1='V''M# '")
    vm.expect("VM# ")


def shell(vm: pexpect.spawn, command: str, timeout: int = 600) -> None:
    vm.sendline(command)
    vm.expect("VM# ", timeout=timeout)


def install(args: list[str], port: int, log) -> None:
    vm = spawn(args, log)
    vm.expect("archiso login:", timeout=300)
    vm.sendline("root")
    vm.expect("# ")
    own_prompt(vm)
    site = f"http://{HOST}:{port}"
    shell(vm, f"printf 'Server = {site}/$repo/os/$arch\\nServer = {MIRROR}\\n' > /etc/pacman.d/mirrorlist")
    shell(vm, f"curl -fsS -o /root/mothership {site}/mothership && chmod +x /root/mothership")
    vm.sendline(f"/root/mothership install --fleet {site}/fleet.git --type vm --option encrypted"
                " --hostname vmtest --timezone Europe/Paris --disk /dev/vda --layout wipe")
    vm.expect("Type vda to go on:")
    vm.sendline("vda")
    for question in ["Disk passphrase:", "Again:"]:
        vm.expect(question)
        vm.sendline(PASSPHRASE)
    vm.expect("tester's password:", timeout=INSTALL_TIMEOUT)
    for question in ["New password:", "Retype new password:"]:
        vm.expect(question)
        vm.sendline(PASSWORD)
    vm.expect("r type the recovery key")
    vm.send("r")
    vm.expect("Recovery key:")
    secret = next(line for line in (HERE / "recovery.txt").read_text().splitlines()
                  if line.startswith("AGE-SECRET-KEY-"))
    vm.sendline(secret)
    vm.expect("Secure Boot holds this machine's keys now.", timeout=INSTALL_TIMEOUT)
    vm.expect("The system is installed.", timeout=300)
    vm.expect("VM# ")
    vm.sendline("poweroff")
    vm.expect(pexpect.EOF, timeout=120)


def first_start(args: list[str], log) -> None:
    vm = spawn(args, log)
    vm.expect("[Pp]assphrase for", timeout=300)
    vm.sendline(PASSPHRASE)
    vm.expect("Type the passphrase once more:", timeout=1200)
    vm.expect("[Pp]assphrase", timeout=120)
    vm.sendline(PASSPHRASE)
    vm.expect("Nothing differs from the vm type", timeout=600)
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
    vm = spawn(args, log)
    found = vm.expect(["vmtest login:", "[Pp]assphrase for"], timeout=300)
    if found == 1:
        sys.exit("the disk asked for its passphrase: the TPM didn't open it")
    login(vm)
    vm.sendline("sudo poweroff")
    vm.expect(pexpect.EOF, timeout=120)


def login(vm: pexpect.spawn) -> None:
    vm.expect("vmtest login:", timeout=300)
    vm.sendline("tester")
    vm.expect("Password:")
    vm.sendline(PASSWORD)
    vm.expect(r"\$ ")
    own_prompt(vm)


if __name__ == "__main__":
    sys.exit(main())

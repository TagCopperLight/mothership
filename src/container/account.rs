use super::Port;
use crate::{
    cmd::{self, As},
    file,
    user::{self, User},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

pub const USER: &str = "services";
const HOME: &str = "/var/lib/services";
pub const SYSCTL: &str = "/etc/sysctl.d/60-mothership-services.conf";
const PORT_START: &str = "net.ipv4.ip_unprivileged_port_start";
const OLD_UNIT: &str = ".config/systemd/user/docker.service";
pub const UNIT_NAME: &str = "mothership-services.service";
pub const UNIT: &str = "/etc/systemd/system/mothership-services.service";
pub const FIREWALL_NAME: &str = "mothership-services-firewall.service";
pub const FIREWALL: &str = "/etc/systemd/system/mothership-services-firewall.service";
pub const RULES: &str = "/etc/mothership/services.nft";
pub const WRITTEN: [&str; 3] = [UNIT, FIREWALL, RULES];
pub const UNIT_NAMES: [&str; 2] = [UNIT_NAME, FIREWALL_NAME];
const TABLE: &str = "inet mothership";
const NFT: [&str; 2] = ["/usr/sbin/nft", "/usr/bin/nft"];
const LINGER: &str = "/var/lib/systemd/linger";
const ROOTLESS: [&str; 2] = [
    "/usr/bin/dockerd-rootless.sh",
    "/usr/share/docker.io/contrib/dockerd-rootless.sh",
];
const NOLOGIN: [&str; 2] = ["/usr/sbin/nologin", "/usr/bin/nologin"];
const SUBIDS: [&str; 2] = ["/etc/subuid", "/etc/subgid"];
const FIRST_SUBID: u64 = 100_000;
const SUBID_COUNT: u64 = 65_536;
const START_LIMIT: Duration = Duration::from_secs(90);

pub fn user() -> Result<Option<User>> {
    user::by_name(USER)
}

pub fn runtime(user: &User) -> PathBuf {
    PathBuf::from(format!("/run/user/{}", user.uid))
}

pub fn socket(user: &User) -> String {
    format!("unix://{}/docker.sock", runtime(user).display())
}

pub fn snapshot(user: &User, service: &str) -> PathBuf {
    user.home.join("snapshots").join(service)
}

fn first(paths: &[&'static str]) -> Option<&'static str> {
    paths.iter().copied().find(|path| Path::new(path).exists())
}

fn rootless() -> Result<&'static str> {
    first(&ROOTLESS).context(
        "this machine has no rootless Docker: the type must list it, such as docker.io, \
             docker-compose, rootlesskit, uidmap, slirp4netns and dbus-user-session on Debian",
    )
}

fn nft() -> Result<&'static str> {
    first(&NFT).context("this machine has no nft: the type must list nftables")
}

fn stale(user: &User, ports: &BTreeSet<Port>) -> Result<Vec<(&'static str, String)>> {
    let wanted = [
        (UNIT, unit(rootless()?, user, ports)),
        (FIREWALL, firewall(nft()?, user)),
        (RULES, rules(user)),
    ];
    let mut stale = Vec::new();
    for (path, text) in wanted {
        if file::read_if_any(Path::new(path))?.as_ref() != Some(&text) {
            stale.push((path, text));
        }
    }
    Ok(stale)
}

fn links(user: &User) -> [PathBuf; 2] {
    let wants =
        Path::new(crate::systemd::SYSTEM_DIRS[0]).join(format!("user@{}.service.wants", user.uid));
    UNIT_NAMES.map(|name| wants.join(name))
}

pub fn rules(user: &User) -> String {
    let uid = user.uid;
    format!(
        "table {TABLE}\ndelete table {TABLE}\ntable {TABLE} {{\n\tchain input {{\n\
         \t\ttype filter hook input priority filter; policy accept;\n\
         \t\tct state != {{ established, related }} meta l4proto {{ tcp, udp }} socket cgroupv2 level 3 \
         \"user.slice/user-{uid}.slice/user@{uid}.service\" reject\n\t}}\n}}\n"
    )
}

fn firewall(nft: &str, user: &User) -> String {
    let uid = user.uid;
    format!(
        "[Unit]\nDescription=Refuse connections to what services' user manager runs\n\
         After=user@{uid}.service\nBindsTo=user@{uid}.service\n\n[Service]\nType=oneshot\n\
         RemainAfterExit=yes\nExecStart={nft} -f {RULES}\nExecStop={nft} delete table {TABLE}\n\n\
         [Install]\nWantedBy=user@{uid}.service\n"
    )
}

pub fn unit(rootless: &str, user: &User, ports: &BTreeSet<Port>) -> String {
    let (uid, run) = (user.uid, runtime(user));
    let allowed: String = ports
        .iter()
        .map(|port| format!("SocketBindAllow={}\n", port.bind_rule()))
        .collect();
    format!(
        "[Unit]\nDescription=Docker for the fleet's services\n\
         BindsTo=user@{uid}.service\nRequires={FIREWALL_NAME}\n\
         After=user@{uid}.service {FIREWALL_NAME} network-online.target\n\
         Wants=network-online.target\n\n\
         [Service]\nUser={USER}\nEnvironment=PATH=/usr/sbin:/usr/bin:/sbin:/bin\n\
         Environment=HOME={home}\nEnvironment=XDG_RUNTIME_DIR={run}\n\
         Environment=DBUS_SESSION_BUS_ADDRESS=unix:path={run}/bus\nExecStart={rootless}\n\
         ExecReload=/bin/kill -s HUP $MAINPID\nAmbientCapabilities=CAP_NET_BIND_SERVICE\n\
         {allowed}SocketBindDeny=any\nTimeoutSec=0\nRestartSec=2\nRestart=always\n\
         StartLimitBurst=3\nStartLimitInterval=60s\nLimitNOFILE=infinity\nLimitNPROC=infinity\n\
         LimitCORE=infinity\nTasksMax=infinity\nDelegate=yes\nType=notify\nNotifyAccess=all\n\
         KillMode=mixed\n\n[Install]\nWantedBy=user@{uid}.service\n",
        home = user.home.display(),
        run = run.display(),
    )
}

pub fn missing(ports: &BTreeSet<Port>) -> Result<Vec<String>> {
    let Some(user) = user()? else {
        return Ok(vec!["the account is missing".into()]);
    };
    let mut missing = Vec::new();
    for path in SUBIDS {
        if subids(path)?.iter().all(|(name, ..)| name != USER) {
            missing.push("it has no subordinate ids".into());
            break;
        }
    }
    if !Path::new(LINGER).join(USER).exists() {
        missing.push("its services don't start with the machine".into());
    }
    if Path::new(SYSCTL).exists() {
        missing.push("every account may open ports from 80 up".into());
    }
    if user.read(&user.home.join(OLD_UNIT))?.is_some() {
        missing.push("its Docker runs as its own unit".into());
    }
    let enabled = links(&user)
        .iter()
        .all(|link| std::fs::symlink_metadata(link).is_ok());
    match stale(&user, ports) {
        Ok(stale) if stale.is_empty() && enabled => {}
        Ok(_) => {
            missing.push("its Docker and firewall aren't set up as the fleet's ports ask".into())
        }
        Err(e) => missing.push(format!("{e:#}")),
    }
    let active = cmd::bytes_allowing_exit(As::Root, "systemctl", &["is-active", FIREWALL_NAME], 3)?;
    if active != b"active\n" {
        missing.push("its firewall isn't running".into());
    }
    Ok(missing)
}

pub fn setup(ports: &BTreeSet<Port>) -> Result<()> {
    rootless()?;
    nft()?;
    let user = match user()? {
        Some(user) => user,
        None => {
            let shell = first(&NOLOGIN).context("this machine has no nologin shell")?;
            cmd::root(
                "useradd",
                &[
                    "--create-home",
                    "--home-dir",
                    HOME,
                    "--shell",
                    shell,
                    "--user-group",
                    "--",
                    USER,
                ],
            )?;
            user()?.context("useradd created no services account")?
        }
    };
    add_subids()?;
    cmd::root("loginctl", &["enable-linger", USER])?;
    cmd::root(
        "systemctl",
        &["start", &format!("user@{}.service", user.uid)],
    )?;
    drop_old_unit(&user)?;
    if file::remove_if_found(Path::new(SYSCTL))? {
        cmd::root(
            "sysctl",
            &["--quiet", "--write", &format!("{PORT_START}=1024")],
        )?;
        cmd::root("sysctl", &["--quiet", "--system"])?;
    }
    let mut changed = Vec::new();
    for (path, text) in stale(&user, ports)? {
        file::write(Path::new(path), text.as_bytes(), 0o644)?;
        changed.push(path);
    }
    cmd::root("systemctl", &["daemon-reload"])?;
    let firewall_changed = changed.iter().any(|path| [FIREWALL, RULES].contains(path));
    for (name, again) in [
        (FIREWALL_NAME, firewall_changed),
        (UNIT_NAME, changed.contains(&UNIT)),
    ] {
        cmd::root("systemctl", &["enable", "--", name])?;
        let verb = if again { "restart" } else { "start" };
        cmd::root("systemctl", &[verb, "--", name])?;
    }
    let start = Instant::now();
    let info = ["info", "--format", "{{.ServerVersion}}"];
    loop {
        let mut docker = As::User(&user).command("docker", &info);
        docker.env("DOCKER_HOST", socket(&user));
        if cmd::output_within(docker, "docker", &info, Duration::from_secs(30)).is_ok() {
            return Ok(());
        }
        if start.elapsed() > START_LIMIT {
            bail!("the services account's Docker didn't start: `journalctl -u {UNIT_NAME}`");
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn drop_old_unit(user: &User) -> Result<()> {
    let path = user.home.join(OLD_UNIT);
    if user.read(&path)?.is_none() {
        return Ok(());
    }
    let machine = format!("{USER}@");
    let scope = ["--user", "--machine", &machine];
    cmd::root(
        "systemctl",
        &[&scope[..], &["disable", "--now", "docker.service"]].concat(),
    )?;
    user.with_rights(|| file::remove_if_found(&path))?;
    cmd::root("systemctl", &[&scope[..], &["daemon-reload"]].concat())
}

fn subids(path: &str) -> Result<Vec<(String, u64, u64)>> {
    let text = file::read_if_any(Path::new(path))?.unwrap_or_default();
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split(':').collect();
            let [name, start, count] = parts[..] else {
                bail!("a line in {path} is malformed: {line}");
            };
            Ok((name.to_owned(), start.parse()?, count.parse()?))
        })
        .collect()
}

fn add_subids() -> Result<()> {
    let (mut taken, mut have) = (FIRST_SUBID, [false; 2]);
    for (i, path) in SUBIDS.iter().enumerate() {
        for (name, start, count) in subids(path)? {
            have[i] |= name == USER;
            taken = taken.max(start + count);
        }
    }
    let range = format!("{taken}-{}", taken + SUBID_COUNT - 1);
    let mut args = Vec::new();
    if !have[0] {
        args.extend(["--add-subuids", &range]);
    }
    if !have[1] {
        args.extend(["--add-subgids", &range]);
    }
    if args.is_empty() {
        return Ok(());
    }
    args.extend(["--", USER]);
    cmd::root("usermod", &args)
}

use super::Service;
use crate::{
    cmd::{self, As},
    file, update,
    user::{self, User},
    walk,
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
const OLD_FIREWALL_NAME: &str = "mothership-services-firewall.service";
const OLD_FIREWALL: &str = "/etc/systemd/system/mothership-services-firewall.service";
const OLD_RULES: &str = "/etc/mothership/services.nft";
const START_NAME: &str = "mothership-services-start.service";
const START: &str = "/etc/systemd/system/mothership-services-start.service";
const START_AT: &str = "/etc/systemd/system/mothership-services-start@.service";
const START_PREFIX: &str = "mothership-services-start@";
const DEVICE_PREFIX: &str = "sys-subsystem-net-devices-";
pub const WRITTEN: [&str; 6] = [UNIT, START, START_AT, SYSCTL, OLD_FIREWALL, OLD_RULES];
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

pub fn ours(unit: &str) -> bool {
    [UNIT_NAME, START_NAME, OLD_FIREWALL_NAME].contains(&unit) || unit.starts_with(START_PREFIX)
}

fn stale(user: &User, services: &[Service]) -> Result<Vec<(&'static str, String)>> {
    let wanted = [
        (UNIT, unit(rootless()?, user, services)?),
        (START, start_unit(false)),
        (START_AT, start_unit(true)),
    ];
    let mut stale = Vec::new();
    for (path, text) in wanted {
        if file::read_if_any(Path::new(path))?.as_ref() != Some(&text) {
            stale.push((path, text));
        }
    }
    Ok(stale)
}

fn system_dir() -> &'static Path {
    Path::new(crate::systemd::SYSTEM_DIRS[0])
}

fn links(user: &User) -> [PathBuf; 2] {
    [
        system_dir()
            .join(format!("user@{}.service.wants", user.uid))
            .join(UNIT_NAME),
        system_dir()
            .join(format!("{UNIT_NAME}.wants"))
            .join(START_NAME),
    ]
}

fn escape(name: &str) -> String {
    name.replace('-', "\\x2d")
}

fn instance(interface: &str) -> String {
    format!("{START_PREFIX}{}.service", escape(interface))
}

fn device_wants(interface: &str) -> PathBuf {
    system_dir()
        .join(format!("{DEVICE_PREFIX}{}.device.wants", escape(interface)))
        .join(instance(interface))
}

fn enabled_instances() -> Result<BTreeSet<PathBuf>> {
    let mut found = BTreeSet::new();
    for dir in walk::children(system_dir())? {
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !(name.starts_with(DEVICE_PREFIX) && name.ends_with(".device.wants")) {
            continue;
        }
        for link in walk::children(&dir)? {
            if link
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(START_PREFIX))
            {
                found.insert(link);
            }
        }
    }
    Ok(found)
}

fn wanted_instances(services: &[Service]) -> BTreeSet<PathBuf> {
    super::interfaces(services)
        .into_iter()
        .map(device_wants)
        .collect()
}

pub fn start_unit(per_interface: bool) -> String {
    let (what, args, install) = match per_interface {
        false => ("every service", "", UNIT_NAME.to_owned()),
        true => (
            "the services that face %I",
            " %I",
            format!("{DEVICE_PREFIX}%i.device"),
        ),
    };
    format!(
        "[Unit]\nDescription=Start {what} of the fleet\nAfter={UNIT_NAME}\n\n[Service]\n\
         Type=oneshot\nExecStart={} start{args}\n\n[Install]\nWantedBy={install}\n",
        update::BINARY
    )
}

pub fn unit(rootless: &str, user: &User, services: &[Service]) -> Result<String> {
    let ports = super::published(services)?;
    let (uid, run) = (user.uid, runtime(user));
    let allowed: String = ports
        .iter()
        .map(|port| format!("SocketBindAllow={}\n", port.bind_rule()))
        .collect();
    Ok(format!(
        "[Unit]\nDescription=Docker for the fleet's services\n\
         BindsTo=user@{uid}.service\n\
         After=user@{uid}.service network-online.target\n\
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
    ))
}

pub fn missing(services: &[Service]) -> Result<Vec<String>> {
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
    match stale(&user, services) {
        Ok(stale) if stale.is_empty() && enabled => {}
        Ok(_) => missing.push("its Docker isn't set up as the fleet's ports ask".into()),
        Err(e) => missing.push(format!("{e:#}")),
    }
    if enabled_instances()? != wanted_instances(services) {
        missing.push("its services don't start when the interfaces they face come up".into());
    }
    if [OLD_FIREWALL, OLD_RULES]
        .iter()
        .any(|p| Path::new(p).exists())
    {
        missing.push("its old firewall is still set up".into());
    }
    Ok(missing)
}

pub fn setup(services: &[Service]) -> Result<()> {
    rootless()?;
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
    drop_sysctl()?;
    let stale = stale(&user, services)?;
    for (path, text) in &stale {
        file::write(Path::new(path), text.as_bytes(), 0o644)?;
    }
    cmd::root("systemctl", &["daemon-reload"])?;
    drop_firewall()?;
    let wanted = wanted_instances(services);
    for link in enabled_instances()?.difference(&wanted) {
        let name = link.file_name().and_then(|n| n.to_str()).unwrap_or("");
        cmd::root("systemctl", &["disable", "--", name])?;
    }
    let mut units = vec![UNIT_NAME.to_owned(), START_NAME.to_owned()];
    units.extend(super::interfaces(services).into_iter().map(instance));
    cmd::root(
        "systemctl",
        &[
            &["enable", "--"][..],
            &units.iter().map(String::as_str).collect::<Vec<_>>(),
        ]
        .concat(),
    )?;
    let verb = match stale.iter().any(|(path, _)| *path == UNIT) {
        true => "restart",
        false => "start",
    };
    cmd::root("systemctl", &[verb, "--", UNIT_NAME])?;
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

fn drop_sysctl() -> Result<()> {
    if file::remove_if_found(Path::new(SYSCTL))? {
        cmd::root(
            "sysctl",
            &["--quiet", "--write", &format!("{PORT_START}=1024")],
        )?;
        cmd::root("sysctl", &["--quiet", "--system"])?;
    }
    Ok(())
}

pub fn leftovers() -> Result<bool> {
    Ok(user()?.is_some()
        || Path::new(LINGER).join(USER).exists()
        || WRITTEN.iter().any(|path| Path::new(path).exists())
        || !enabled_instances()?.is_empty())
}

pub fn teardown() -> Result<()> {
    for link in enabled_instances()? {
        let name = link.file_name().and_then(|n| n.to_str()).unwrap_or("");
        cmd::root("systemctl", &["disable", "--", name])?;
    }
    for (path, name) in [(UNIT, UNIT_NAME), (START, START_NAME)] {
        if Path::new(path).exists() {
            cmd::root("systemctl", &["disable", "--now", "--", name])?;
        }
    }
    drop_firewall()?;
    for path in [UNIT, START, START_AT] {
        file::remove_if_found(Path::new(path))?;
    }
    cmd::root("systemctl", &["daemon-reload"])?;
    drop_sysctl()?;
    if Path::new(LINGER).join(USER).exists() {
        cmd::root("loginctl", &["disable-linger", USER])?;
    }
    let Some(user) = user()? else {
        return Ok(());
    };
    cmd::root(
        "systemctl",
        &["stop", &format!("user@{}.service", user.uid)],
    )?;
    let mut args = Vec::new();
    for (path, flag) in SUBIDS.iter().zip(["--del-subuids", "--del-subgids"]) {
        for (name, start, count) in subids(path)? {
            if name == USER {
                args.extend([flag.to_owned(), format!("{start}-{}", start + count - 1)]);
            }
        }
    }
    if !args.is_empty() {
        args.extend(["--".to_owned(), USER.to_owned()]);
        cmd::root("usermod", &args)?;
    }
    cmd::root("userdel", &["--remove", "--", USER])?;
    let group = cmd::bytes_allowing_exit(As::Root, "getent", &["group", USER], 2)?;
    if !group.is_empty() {
        cmd::root("groupdel", &["--", USER])?;
    }
    Ok(())
}

fn drop_firewall() -> Result<()> {
    if Path::new(OLD_FIREWALL).exists() {
        cmd::root("systemctl", &["disable", "--now", "--", OLD_FIREWALL_NAME])?;
    }
    let mut removed = false;
    for path in [OLD_FIREWALL, OLD_RULES] {
        removed |= file::remove_if_found(Path::new(path))?;
    }
    if removed {
        cmd::root("systemctl", &["daemon-reload"])?;
    }
    Ok(())
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

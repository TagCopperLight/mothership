use super::state::State;
use crate::{
    fleet::{Fleet, checkout},
    services::{self, Action},
    sync::item::{Change, Item, Section},
    sync::kind::Kind,
    trust::update,
    util::cmd,
    util::file,
};
use anyhow::Result;
use std::path::{Path, PathBuf};

pub const DIR: &str = "/etc/mothership/probe";
pub const SIGNING: &str = "signing";
pub const SSH: &str = "ssh";
pub const GITHUB_TOKEN: &str = "github-token";
pub const LOGINS: &str = "docker-config.json";
pub const SERVICE: &str = "/etc/systemd/system/mothership-probe.service";
pub const TIMER: &str = "/etc/systemd/system/mothership-probe.timer";
pub const TIMER_NAME: &str = "mothership-probe.timer";
pub const FLEET_VAR: &str = "MOTHERSHIP_FLEET";
pub const HOST_VAR: &str = "MOTHERSHIP_HOST";
const TIMER_TEXT: &str = "[Unit]\nDescription=Run mothership's probe every 5 minutes\n\n\
    [Timer]\nOnCalendar=*:0/5\nRandomizedDelaySec=30s\nPersistent=true\n\n[Install]\n\
    WantedBy=timers.target\n";

pub fn runs_here(fleet: &Fleet) -> bool {
    fleet.probe_host.as_deref() == Some(fleet.name.as_str())
}

pub fn key_path(name: &str) -> PathBuf {
    Path::new(DIR).join(name)
}

pub fn known() -> Vec<PathBuf> {
    vec![
        key_path(SIGNING),
        key_path(SSH),
        key_path(LOGINS),
        PathBuf::from(SERVICE),
        PathBuf::from(TIMER),
    ]
}

fn service(fleet: &Fleet) -> Result<String> {
    let mut credentials = String::new();
    for name in [SIGNING, SSH, GITHUB_TOKEN] {
        let path = key_path(name);
        if path.exists() {
            credentials.push_str(&format!("LoadCredential={name}:{}\n", path.display()));
        }
    }
    if services::token_path(fleet).exists() {
        let path = key_path(LOGINS);
        credentials.push_str(&format!("LoadCredential={LOGINS}:{}\n", path.display()));
    }
    let origin = checkout::origin(fleet.admin())?;
    Ok(format!(
        "[Unit]\nDescription=mothership's probe, which keeps the fleet's images up to date\n\
         After=network-online.target\nWants=network-online.target\n\n[Service]\nType=oneshot\n\
         ExecStart={} probe\nDynamicUser=yes\nStateDirectory=mothership-probe\n\
         Environment={FLEET_VAR}={origin}\nEnvironment={HOST_VAR}={}\n{credentials}\
         ProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\nPrivateDevices=yes\n\
         InaccessiblePaths=-/var/lib/services -/srv\nNoNewPrivileges=yes\n\
         ProtectKernelTunables=yes\nProtectKernelModules=yes\nProtectControlGroups=yes\n\
         RestrictSUIDSGID=yes\nLockPersonality=yes\nRestrictNamespaces=yes\n\
         CapabilityBoundingSet=\nSystemCallArchitectures=native\n",
        update::BINARY,
        fleet.name
    ))
}

fn written(path: &str) -> Result<Option<String>> {
    file::read_if_any(Path::new(path))
}

fn fleet_item(mut item: Item) -> Item {
    item.section = Section::Fleet;
    item
}

pub fn items(fleet: &Fleet) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    let here = runs_here(fleet);
    let present = written(SERVICE)?.is_some() || written(TIMER)?.is_some();
    if here {
        let missing: Vec<&str> = [SIGNING, SSH]
            .into_iter()
            .filter(|name| !key_path(name).exists())
            .collect();
        if !missing.is_empty() {
            items.push(
                Item::new(Kind::Container, Change::Unreadable, "probe", "no keys")
                    .noting(format!("{DIR} lacks {}", missing.join(" and ")))
                    .because("`sudo mothership apply` creates them"),
            );
        } else {
            let logins_current = !fleet.etc_open()
                || file::read(&key_path(LOGINS))? == services::docker_config(fleet)?;
            let current = written(SERVICE)?.as_deref() == Some(service(fleet)?.as_str())
                && written(TIMER)?.as_deref() == Some(TIMER_TEXT)
                && logins_current
                && Path::new("/etc/systemd/system/timers.target.wants")
                    .join(TIMER_NAME)
                    .exists();
            if !current {
                let change = match present {
                    true => Change::Edited,
                    false => Change::Removed,
                };
                items.push(fleet_item(
                    Item::new(Kind::Container, change, "probe", "the probe's unit")
                        .noting("mothership.toml names this machine")
                        .reverting(Action::Probe),
                ));
            }
        }
        let state = State::load_as_root()?;
        for notice in state.notices.into_values() {
            items.push(
                Item::new(Kind::Container, Change::Unreadable, "probe", "notice")
                    .noting(notice)
                    .because("move the service to its new major in the fleet"),
            );
        }
        if let Some(problem) = state.problem {
            items.push(
                Item::new(Kind::Container, Change::Unreadable, "probe", "failed")
                    .noting(problem)
                    .because("`journalctl -u mothership-probe` shows its runs"),
            );
        }
    } else if present {
        items.push(fleet_item(
            Item::new(Kind::Container, Change::Added, "probe", "the probe's unit")
                .noting("mothership.toml names another machine")
                .reverting(Action::Unprobe),
        ));
    }
    Ok(items)
}

const NSSWITCH: &str = "/etc/nsswitch.conf";

fn resolves_dynamic_users() -> Result<bool> {
    let text = file::read_if_any(Path::new(NSSWITCH))?.unwrap_or_default();
    Ok(text.lines().any(|line| {
        line.split_once(':').is_some_and(|(database, sources)| {
            database.trim() == "passwd" && sources.split_whitespace().any(|s| s == "systemd")
        })
    }))
}

pub fn install(fleet: &Fleet) -> Result<()> {
    if !resolves_dynamic_users()? {
        anyhow::bail!(
            "the probe's ssh needs its temporary user to have a name, and {NSSWITCH} doesn't ask \
             systemd for it: the type must list nss-systemd (libnss-systemd on Debian)"
        );
    }
    match services::docker_config(fleet)? {
        Some(bytes) => file::write(&key_path(LOGINS), &bytes, 0o600)?,
        None => drop(file::remove_if_found(&key_path(LOGINS))?),
    }
    file::write(Path::new(SERVICE), service(fleet)?.as_bytes(), 0o644)?;
    file::write(Path::new(TIMER), TIMER_TEXT.as_bytes(), 0o644)?;
    cmd::root("systemctl", &["daemon-reload"])?;
    cmd::root("systemctl", &["enable", "--now", "--", TIMER_NAME])
}

pub fn remove() -> Result<()> {
    cmd::root("systemctl", &["disable", "--now", "--", TIMER_NAME])?;
    file::remove_if_found(&key_path(LOGINS))?;
    file::remove_if_found(Path::new(SERVICE))?;
    file::remove_if_found(Path::new(TIMER))?;
    cmd::root("systemctl", &["daemon-reload"])
}

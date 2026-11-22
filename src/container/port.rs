use crate::cmd::{self, As};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{collections::BTreeMap, fmt, net::IpAddr, path::Path};

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(try_from = "String")]
pub struct Port {
    pub number: u16,
    pub protocol: Protocol,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(try_from = "String", into = "String")]
pub enum Network {
    Public,
    Local,
    Interface(String),
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Published {
    pub port: Port,
    pub network: Network,
}

pub enum Bound {
    Ready(Vec<serde_json::Value>),
    Waiting(String),
}

const PUBLIC: &str = "public";
const LOCAL: &str = "local";
const LOOPBACK_INTERFACE: &str = "lo";
const LOOPBACK: [&str; 2] = ["127.0.0.1", "::1"];
const INTERFACES: &str = "/sys/class/net";
const NAME_LIMIT: usize = 15;

impl Protocol {
    fn name(self) -> &'static str {
        match self {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
        }
    }

    fn parse(name: &str) -> Option<Protocol> {
        [Protocol::Tcp, Protocol::Udp]
            .into_iter()
            .find(|p| p.name() == name)
    }
}

impl TryFrom<String> for Port {
    type Error = anyhow::Error;

    fn try_from(text: String) -> Result<Port> {
        let wrong = || format!("{text} isn't a port like \"443/tcp\" or \"443/udp\"");
        let (number, protocol) = text.split_once('/').with_context(wrong)?;
        let protocol = Protocol::parse(protocol).with_context(wrong)?;
        let number: u16 = number.parse().ok().filter(|n| *n > 0).with_context(wrong)?;
        Ok(Port { number, protocol })
    }
}

impl Serialize for Port {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.number, self.protocol.name())
    }
}

impl Port {
    pub fn bind_rule(&self) -> String {
        format!("{}:{}", self.protocol.name(), self.number)
    }

    fn compose(&self, host: Option<&str>) -> serde_json::Value {
        let mut entry = serde_json::json!({
            "target": self.number,
            "published": self.number.to_string(),
            "protocol": self.protocol.name(),
        });
        if let Some(host) = host {
            entry["host_ip"] = host.into();
        }
        entry
    }
}

impl TryFrom<String> for Network {
    type Error = anyhow::Error;

    fn try_from(name: String) -> Result<Network> {
        Ok(match name.as_str() {
            PUBLIC => Network::Public,
            LOCAL => Network::Local,
            LOOPBACK_INTERFACE => bail!("{name} is the machine itself: name it {LOCAL}"),
            _ => {
                let plain = !name.is_empty()
                    && name.len() <= NAME_LIMIT
                    && !name.starts_with(['.', '-'])
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
                if !plain {
                    bail!(
                        "{name} isn't a network: ports face {PUBLIC}, {LOCAL} or a network \
                         interface such as tailscale0"
                    );
                }
                Network::Interface(name)
            }
        })
    }
}

impl From<Network> for String {
    fn from(network: Network) -> String {
        network.to_string()
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Network::Public => PUBLIC,
            Network::Local => LOCAL,
            Network::Interface(name) => name,
        })
    }
}

impl Network {
    pub fn clashes(&self, other: &Network) -> bool {
        self == other || *self == Network::Public || *other == Network::Public
    }

    pub fn interface(&self) -> Option<&str> {
        match self {
            Network::Interface(name) => Some(name),
            _ => None,
        }
    }
}

impl fmt::Display for Published {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} on {}", self.port, self.network)
    }
}

pub fn by_network<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Published>, D::Error> {
    struct Visitor;

    impl<'de> de::Visitor<'de> for Visitor {
        type Value = Vec<Published>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "ports grouped by the network they face")
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, _: A) -> Result<Self::Value, A::Error> {
            Err(de::Error::custom(format!(
                "name the network each port faces: ports.{PUBLIC}, ports.{LOCAL} or \
                 ports.<interface>, such as ports.tailscale0 = [\"53/udp\"]"
            )))
        }

        fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut published = Vec::new();
            while let Some((network, ports)) = map.next_entry::<String, Vec<Port>>()? {
                let network = Network::try_from(network).map_err(de::Error::custom)?;
                published.extend(ports.into_iter().map(|port| Published {
                    port,
                    network: network.clone(),
                }));
            }
            Ok(published)
        }
    }

    deserializer.deserialize_any(Visitor)
}

pub fn addresses(interface: &str) -> Result<Vec<IpAddr>> {
    if !Path::new(INTERFACES).join(interface).exists() {
        return Ok(Vec::new());
    }
    let args = [
        "-json", "address", "show", "dev", interface, "scope", "global",
    ];
    let out = cmd::bytes(As::Root, "ip", &args)?;
    #[derive(Deserialize)]
    struct Link {
        addr_info: Vec<Address>,
    }
    #[derive(Deserialize)]
    struct Address {
        local: Option<IpAddr>,
        #[serde(default)]
        tentative: bool,
    }
    let links: Vec<Link> = serde_json::from_slice(&out).context("reading ip's addresses")?;
    Ok(links
        .into_iter()
        .flat_map(|link| link.addr_info)
        .filter(|address| !address.tentative)
        .filter_map(|address| address.local)
        .collect())
}

pub fn bind(ports: &[Published], lookup: &dyn Fn(&str) -> Result<Vec<IpAddr>>) -> Result<Bound> {
    let mut found: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut entries = Vec::new();
    for published in ports {
        let hosts: Vec<Option<String>> = match &published.network {
            Network::Public => vec![None],
            Network::Local => LOOPBACK.map(|h| Some(h.to_owned())).into(),
            Network::Interface(name) => {
                if !found.contains_key(name.as_str()) {
                    let addresses =
                        lookup(name).with_context(|| format!("reading the addresses of {name}"))?;
                    if addresses.is_empty() {
                        return Ok(Bound::Waiting(name.clone()));
                    }
                    found.insert(name, addresses.iter().map(IpAddr::to_string).collect());
                }
                found[name.as_str()].iter().cloned().map(Some).collect()
            }
        };
        entries.extend(hosts.iter().map(|h| published.port.compose(h.as_deref())));
    }
    Ok(Bound::Ready(entries))
}

use anyhow::{Context, Result};
use serde::Deserialize;
use std::fmt;

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

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.number, self.protocol.name())
    }
}

impl Port {
    pub fn bind_rule(&self) -> String {
        format!("{}:{}", self.protocol.name(), self.number)
    }

    pub fn compose(&self) -> serde_json::Value {
        serde_json::json!({
            "target": self.number,
            "published": self.number.to_string(),
            "protocol": self.protocol.name(),
        })
    }
}

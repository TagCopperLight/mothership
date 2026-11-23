use crate::util::cmd;
use anyhow::{Context, Result, bail};
use std::{fs, process::Command, time::Duration};

const LIMIT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .rev()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn ok(self, what: &str) -> Result<Response> {
        if !(200..300).contains(&self.status) {
            let body = String::from_utf8_lossy(&self.body);
            bail!(
                "{what} answered {}: {}",
                self.status,
                crate::util::style::clean(body.trim())
            );
        }
        Ok(self)
    }
}

pub struct Request<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: Vec<String>,
    pub body: Option<&'a [u8]>,
}

pub trait Http: Send + Sync {
    fn send(&self, request: &Request) -> Result<Response>;
}

pub struct Curl;

fn quoted(text: &str) -> Result<String> {
    if text.contains(['\n', '\r']) {
        bail!("a request can't hold a line break");
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

impl Http for Curl {
    fn send(&self, request: &Request) -> Result<Response> {
        let dir = tempfile::TempDir::new()?;
        let (head, body, input) = (
            dir.path().join("head"),
            dir.path().join("body"),
            dir.path().join("input"),
        );
        let mut config = vec![
            format!("url = {}", quoted(request.url)?),
            format!("request = {}", quoted(request.method)?),
            format!("dump-header = {}", quoted(&head.to_string_lossy())?),
            format!("output = {}", quoted(&body.to_string_lossy())?),
            "write-out = \"%{http_code}\"".to_owned(),
            "silent".to_owned(),
            "show-error".to_owned(),
            format!("max-time = {}", LIMIT.as_secs()),
        ];
        if request.method == "HEAD" {
            config.push("head".to_owned());
        }
        for header in &request.headers {
            config.push(format!("header = {}", quoted(header)?));
        }
        if let Some(bytes) = request.body {
            fs::write(&input, bytes)?;
            config.push(format!(
                "data-binary = {}",
                quoted(&format!("@{}", input.display()))?
            ));
        }
        let text = config.join("\n") + "\n";
        let args = ["--config", "-"];
        let mut command = Command::new("curl");
        command.args(args);
        let out = cmd::output_with_input(command, "curl", &args, text.as_bytes())
            .with_context(|| format!("reaching {}", request.url))?;
        let status: u16 = String::from_utf8(out)?
            .trim()
            .parse()
            .context("curl printed no status")?;
        let headers = fs::read_to_string(&head)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect();
        let body = crate::util::file::if_found(fs::read(&body))?.unwrap_or_default();
        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

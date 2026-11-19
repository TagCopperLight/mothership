use anyhow::Result;
use std::{
    net::{TcpStream, ToSocketAddrs},
    thread,
    time::Duration,
};

const PROBE: &str = "github.com:443";

fn online() -> bool {
    let Ok(addresses) = PROBE.to_socket_addrs() else {
        return false;
    };
    addresses
        .into_iter()
        .any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(5)).is_ok())
}

pub fn wait() {
    if online() {
        return;
    }
    println!("Waiting for the network…");
    while !online() {
        thread::sleep(Duration::from_secs(10));
    }
}

pub fn retry<T>(what: &str, mut step: impl FnMut() -> Result<T>) -> Result<T> {
    loop {
        match step() {
            Err(e) if !online() => {
                println!("{what} failed with the network down: {e:#}");
                wait();
            }
            result => return result,
        }
    }
}

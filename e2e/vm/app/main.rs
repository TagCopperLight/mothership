use std::{
    env, fs,
    io::Write,
    net::{TcpListener, TcpStream, ToSocketAddrs, UdpSocket},
    thread,
    time::Duration,
};

const VERSION: &str = env!("APP_VERSION");
const BROKEN: &str = "1.0.2";
const TCP: [u16; 2] = [80, 5432];
const UDP: [u16; 2] = [443, 53];

fn main() {
    if env::args().nth(1).as_deref() == Some("health") {
        std::process::exit(if VERSION == BROKEN { 1 } else { 0 });
    }
    if env::args().nth(1).as_deref() == Some("reach") {
        let address = env::args().nth(2).unwrap();
        let found = address.to_socket_addrs().map(|mut a| a.next());
        match found.map(|a| a.map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)))) {
            Ok(Some(Ok(_))) => println!("reached"),
            Ok(Some(Err(e))) | Err(e) => fail(&format!("{address}: {e}")),
            Ok(None) => fail(&format!("{address}: no such host")),
        }
        return;
    }
    let read = |path: &str| fs::read_to_string(path).unwrap_or_default();
    fs::write("/data/version", VERSION).unwrap();
    fs::write("/data/token", read("/run/secrets/token")).unwrap();
    fs::write("/data/conf", read("/etc/app.conf")).unwrap();
    if VERSION == BROKEN {
        fs::write("/data/precious", "ruined").unwrap();
    } else if fs::metadata("/data/precious").is_err() {
        fs::write("/data/precious", "kept").unwrap();
    }
    let mut bound = String::new();
    for port in TCP {
        match TcpListener::bind(("0.0.0.0", port)) {
            Ok(listener) => {
                bound.push_str(&format!("{port}/tcp ok\n"));
                thread::spawn(move || {
                    for mut stream in listener.incoming().flatten() {
                        let _ = stream.write_all(VERSION.as_bytes());
                    }
                });
            }
            Err(e) => bound.push_str(&format!("{port}/tcp {e}\n")),
        }
    }
    for port in UDP {
        match UdpSocket::bind(("0.0.0.0", port)) {
            Ok(socket) => {
                bound.push_str(&format!("{port}/udp ok\n"));
                thread::spawn(move || {
                    let mut buffer = [0; 64];
                    while let Ok((_, from)) = socket.recv_from(&mut buffer) {
                        let _ = socket.send_to(VERSION.as_bytes(), from);
                    }
                });
            }
            Err(e) => bound.push_str(&format!("{port}/udp {e}\n")),
        }
    }
    fs::write("/data/ports", bound).unwrap();
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

fn fail(why: &str) {
    eprintln!("{why}");
    std::process::exit(1);
}

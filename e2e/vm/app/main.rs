use std::{
    env, fs,
    io::Write,
    net::{TcpListener, UdpSocket},
    thread,
    time::Duration,
};

const VERSION: &str = env!("APP_VERSION");
const BROKEN: &str = "1.0.2";
const TCP: [u16; 2] = [80, 5432];
const UDP: u16 = 443;

fn main() {
    if env::args().nth(1).as_deref() == Some("health") {
        std::process::exit(if VERSION == BROKEN { 1 } else { 0 });
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
    match UdpSocket::bind(("0.0.0.0", UDP)) {
        Ok(socket) => {
            bound.push_str(&format!("{UDP}/udp ok\n"));
            thread::spawn(move || {
                let mut buffer = [0; 64];
                while let Ok((_, from)) = socket.recv_from(&mut buffer) {
                    let _ = socket.send_to(VERSION.as_bytes(), from);
                }
            });
        }
        Err(e) => bound.push_str(&format!("{UDP}/udp {e}\n")),
    }
    fs::write("/data/ports", bound).unwrap();
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

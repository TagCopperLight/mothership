use std::{env, fs, thread, time::Duration};

const VERSION: &str = env!("APP_VERSION");
const BROKEN: &str = "1.0.2";

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
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

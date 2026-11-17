mod cmd;
mod fleet;
mod ignore;
mod scan;
mod walk;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command] if command == "scan" => scan::run(),
        _ => {
            eprintln!("usage: mothership scan");
            std::process::exit(2);
        }
    }
}

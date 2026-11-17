mod cmd;
mod edit;
mod fleet;
mod ignore;
mod pacman;
mod preview;
mod prompt;
mod review;
mod scan;
mod walk;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command] if command == "scan" => scan::run(),
        [command] if command == "review" => review::run(),
        _ => {
            eprintln!("usage: mothership scan | review");
            std::process::exit(2);
        }
    }
}

mod apply;
mod cmd;
mod edit;
mod fleet;
mod hardware;
mod ignore;
mod pacman;
mod preview;
mod prompt;
mod record;
mod revert;
mod review;
mod scan;
mod secrets;
mod tools;
mod user;
mod walk;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    user::init()?;
    match args.as_slice() {
        [command] if command == "scan" => scan::run(),
        [command] if command == "review" => review::run(),
        [command] if command == "apply" => apply::run(),
        [command] if command == "key" => secrets::key(),
        [command, name, key] if command == "trust" => secrets::trust(name, key),
        [command, name] if command == "retire" => secrets::retire(name),
        _ => {
            eprintln!(
                "usage: mothership scan | review | apply | key | trust <name> <key> | retire <name>"
            );
            std::process::exit(2);
        }
    }
}

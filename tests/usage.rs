use super::*;

#[test]
fn the_help_lists_every_command_on_its_own_line() {
    let text = text();
    for (name, _, about) in COMMANDS {
        assert!(
            text.lines()
                .any(|line| line.trim_start().starts_with(name) && line.ends_with(about)),
            "{name}"
        );
    }
    assert!(text.lines().all(|line| line.chars().count() <= 80));
}

#[test]
fn fish_completes_commands_and_install_values() {
    let fish = fish();
    assert!(fish.contains("-n __fish_use_subcommand -a review -d"));
    assert!(fish.contains("-l layout -x -a 'wipe beside'"));
    assert!(fish.contains("-l fleet -x\n"));
}

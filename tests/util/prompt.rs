use super::*;

fn choices() -> Vec<Choice> {
    vec![
        Choice::new("wipe", "erase the whole disk"),
        Choice::new("beside", "next to Windows"),
    ]
}

#[test]
fn a_choice_answers_by_number_or_by_name_and_asks_again_otherwise() {
    let mut prompt = Prompt::answering("3\nhalf\nbeside\n");
    assert_eq!(prompt.choose("Layout", &choices()).unwrap(), 1);
    let mut prompt = Prompt::answering("1\n");
    assert_eq!(prompt.choose("Layout", &choices()).unwrap(), 0);
}

#[test]
fn a_single_choice_needs_no_answer_and_none_is_an_error() {
    let mut prompt = Prompt::answering("");
    assert_eq!(
        prompt.choose("Admin", &[Choice::new("tag", "")]).unwrap(),
        0
    );
    assert!(prompt.choose("Admin", &[]).is_err());
}

#[test]
fn a_hidden_answer_keeps_its_spaces_and_a_shown_one_loses_them() {
    let mut prompt = Prompt::answering(" pass phrase \n  box  \n");
    assert_eq!(prompt.hidden("Passphrase:").unwrap(), " pass phrase ");
    assert_eq!(prompt.line("Hostname:").unwrap(), "box");
    assert!(prompt.line("Hostname:").is_err());
}

#[test]
fn a_key_answers_with_its_value_and_other_keys_ask_again() {
    let mut prompt = Prompt::answering("x\nab\nb\n");
    let keys = [('a', 1), ('b', 2)];
    assert_eq!(prompt.ask("›", &keys).unwrap(), Some(2));
    assert_eq!(prompt.ask("›", &keys).unwrap(), None);
}

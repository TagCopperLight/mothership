use super::sign;
use crate::{
    fleet::{
        Fleet, checkout,
        stage::{self, Advance, Outcome},
    },
    util::prompt::Prompt,
};
use anyhow::Result;

pub fn run(message: &str) -> Result<()> {
    let mut prompt = Prompt::new();
    let admin = checkout::admin()?;
    let Some(stage) = stage::stage(&mut prompt)? else {
        return Ok(());
    };
    stage.take_checkout(&admin)?;
    let key = sign::require_machine_key()?;
    let who = checkout::author(&admin)?;
    let outcome = stage.commit(&admin, &who, message, &key, &mut prompt, Advance::KeepEdits)?;
    if let Outcome::Unchanged(_) = outcome {
        println!("Nothing to commit.");
    }
    Ok(())
}

pub fn edit_fleet(
    message: &str,
    command: impl FnOnce(&Fleet, &mut Prompt) -> Result<()>,
) -> Result<Outcome> {
    let key = sign::require_machine_key()?;
    let mut prompt = Prompt::new();
    let Some(stage) = stage::stage(&mut prompt)? else {
        return Ok(Outcome::Declined);
    };
    let fleet = Fleet::load_at(&stage.dir)?;
    command(&fleet, &mut prompt)?;
    let who = checkout::author(fleet.admin())?;
    stage.commit(
        fleet.admin(),
        &who,
        message,
        &key,
        &mut prompt,
        Advance::Merge,
    )
}

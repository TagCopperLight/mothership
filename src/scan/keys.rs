use crate::{
    fleet::Fleet,
    host,
    item::{Change, Item, Section},
    kind::Kind,
    trust::{self, Sort},
};
use anyhow::Result;

pub fn untrusted(fleet: &Fleet) -> Result<Vec<Item>> {
    let mut needed = false;
    for key in trust::machine_keys(fleet)? {
        needed |= !key.trusted
            && match key.sort {
                Sort::Age => trust::guards_secrets(fleet, &key.target)?,
                Sort::Ssh | Sort::Signing => true,
            };
    }
    if !needed {
        return Ok(Vec::new());
    }
    let hostname = host::name()?;
    let Some(command) = trust::trust_command(fleet, &hostname)? else {
        return Ok(Vec::new());
    };
    let mut item = Item::new(Kind::Key, Change::Added, hostname, "not trusted").noting(format!(
        "run `{command}` on a machine that signs for the fleet"
    ));
    item.section = Section::Attention;
    Ok(vec![item])
}

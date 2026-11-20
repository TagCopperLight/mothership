use crate::{
    fleet::Fleet,
    host,
    item::{Change, Item, Section},
    kind::Kind,
    trust,
};
use anyhow::Result;

pub fn untrusted(fleet: &Fleet) -> Result<Vec<Item>> {
    let hostname = host::name()?;
    let mut items = Vec::new();
    for key in trust::machine_keys(fleet)? {
        if key.trusted || !trust::guards_secrets(fleet, &key.target)? {
            continue;
        }
        let command = trust::trust_command(&key.target, &hostname, &key.public);
        let mut item = Item::new(Kind::Key, Change::Added, key.target, "not trusted")
            .noting(format!("run `{command}` where its secrets open"));
        item.section = Section::Attention;
        items.push(item);
    }
    Ok(items)
}

use crate::{
    fleet::{Fleet, homes},
    sync::item::{Change, Item, Section},
    sync::kind::Kind,
};
use anyhow::Result;

pub fn unlinked(fleet: &Fleet) -> Result<Vec<Item>> {
    Ok(homes::shared(&fleet.dir)?
        .into_iter()
        .filter(|file| file.readers.is_empty())
        .map(|file| {
            let subject = file.rel.display().to_string();
            let mut item = Item::new(Kind::Shared, Change::Added, subject, "linked by nothing")
                .noting("link it from a home or a root secret, or delete it");
            item.section = Section::Attention;
            item
        })
        .collect())
}

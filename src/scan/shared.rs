use crate::{
    fleet::{Fleet, homes},
    item::{Change, Item, Section},
    kind::Kind,
};
use anyhow::Result;

pub fn unlinked(fleet: &Fleet) -> Result<Vec<Item>> {
    Ok(homes::shared(&fleet.dir)?
        .into_iter()
        .filter(|file| file.homes.is_empty())
        .map(|file| {
            let subject = file.rel.display().to_string();
            let mut item = Item::new(Kind::Shared, Change::Added, subject, "linked by no home")
                .noting("link it from a home, or delete it");
            item.section = Section::Attention;
            item
        })
        .collect())
}

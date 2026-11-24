use crate::{
    fleet::checked::{Refused, short},
    sync::item::{Change, Item, Section},
    sync::kind::Kind,
};

pub fn refused(commit: Refused) -> Item {
    let subject = format!("{} {}", short(&commit.commit), commit.subject);
    let detail = format!(
        "refused, {}, claims to be by {}",
        commit.date, commit.author
    );
    let mut item = Item::new(Kind::Commit, Change::Added, subject, detail).noting(format!(
        "{}: sudo mothership commit asks whether to vouch for it or drop it",
        commit.why
    ));
    item.section = Section::Attention;
    item
}

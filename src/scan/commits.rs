use crate::{
    fleet::checked::{Refused, short},
    item::{Change, Item, Section},
    kind::Kind,
};

pub fn refused(commit: Refused) -> Item {
    let subject = format!("{} {}", short(&commit.commit), commit.subject);
    let detail = format!("refused, by {}", commit.author);
    let mut item = Item::new(Kind::Commit, Change::Added, subject, detail).noting(format!(
        "{}: sign it again or drop it, then push",
        commit.why
    ));
    item.section = Section::Attention;
    item
}

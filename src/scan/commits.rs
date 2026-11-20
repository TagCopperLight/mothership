use crate::{
    fleet::checked::{Refused, short},
    item::{Change, Item, Section},
    kind::Kind,
};

pub fn refused(commit: Refused) -> Item {
    let subject = format!("{} {}", short(&commit.commit), commit.subject);
    let detail = format!(
        "refused, {}, claims to be by {}",
        commit.date, commit.author
    );
    let mut item = Item::new(Kind::Commit, Change::Added, subject, detail)
        .noting(format!("{}: remove it from main, then push", commit.why));
    item.section = Section::Attention;
    item
}

use crate::{edit::Add, fleet::Fleet, kind::Kind, preview::Preview, revert::Revert, style::Paint};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Change {
    InClear,
    Unreadable,
    Added,
    Edited,
    Removed,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Section {
    Attention,
    Machine,
    Fleet,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Attention, Section::Machine, Section::Fleet];

    pub fn title(self) -> &'static str {
        match self {
            Section::Attention => "Needs attention",
            Section::Machine => "Changed on this machine",
            Section::Fleet => "Coming from the fleet",
        }
    }

    pub fn paint(self) -> Paint {
        match self {
            Section::Attention => Paint::Red,
            Section::Machine => Paint::Yellow,
            Section::Fleet => Paint::Cyan,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Secret {
    No,
    Guessed,
    Fleet,
    Locked,
}

pub enum Adding {
    Offered(Add),
    Refused(&'static str),
    Never,
}

pub struct Item {
    pub kind: Kind,
    pub change: Change,
    pub section: Section,
    pub member: Option<usize>,
    pub subject: String,
    pub detail: String,
    pub note: String,
    pub secret: Secret,
    pub add: Adding,
    pub revert: Option<Revert>,
    pub preview: Preview,
    pub guess: Option<PathBuf>,
}

impl Item {
    pub fn new(
        kind: Kind,
        change: Change,
        subject: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        let section = match change {
            Change::InClear | Change::Unreadable => Section::Attention,
            Change::Added | Change::Edited | Change::Removed => Section::Machine,
        };
        Item {
            kind,
            change,
            section,
            member: None,
            subject: subject.into(),
            detail: detail.into(),
            note: String::new(),
            secret: Secret::No,
            add: Adding::Never,
            revert: None,
            preview: Preview::None,
            guess: None,
        }
    }

    pub fn locked(kind: Kind, subject: impl Into<String>) -> Self {
        let mut item = Item::new(kind, Change::Unreadable, subject, "locked")
            .noting("this machine's key isn't on its list")
            .showing(Preview::Locked);
        item.secret = Secret::Locked;
        item
    }

    pub fn secret(mut self, secret: Secret) -> Self {
        self.secret = secret;
        self.guess = None;
        if let Adding::Offered(add) = &mut self.add
            && let Some(encrypted) = add.encrypted()
        {
            *add = encrypted;
        }
        self
    }

    pub fn reverting(mut self, revert: impl Into<Revert>) -> Self {
        self.revert = Some(revert.into());
        self
    }

    pub fn showing(mut self, preview: Preview) -> Self {
        self.preview = preview;
        self
    }

    pub fn noting(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    pub fn with(mut self, add: Add) -> Self {
        self.add = Adding::Offered(add);
        self
    }

    pub fn guessing(mut self, path: &Path) -> Self {
        self.guess = Some(path.to_path_buf());
        self
    }

    pub fn because(mut self, why: &'static str) -> Self {
        self.add = Adding::Refused(why);
        self
    }

    pub fn sign(&self) -> (&'static str, Paint) {
        let change = match (self.section, self.change) {
            (Section::Fleet, Change::Added) => Change::Removed,
            (Section::Fleet, Change::Removed) => Change::Added,
            (_, change) => change,
        };
        match change {
            Change::InClear => ("!", Paint::Red),
            Change::Unreadable => ("?", Paint::Dim),
            Change::Added => ("+", Paint::Green),
            Change::Edited => ("~", Paint::Yellow),
            Change::Removed => ("-", Paint::Red),
        }
    }

    pub fn verb(&self) -> String {
        match (&self.revert, self.section) {
            (Some(revert), Section::Fleet) if revert.writes_fleet_version() => match self.change {
                Change::Added => "delete",
                Change::Removed => "create",
                _ => "update",
            }
            .into(),
            (Some(revert), Section::Fleet) => revert.describe(),
            _ => self.detail.clone(),
        }
    }
}

impl Item {
    pub fn ignore_line(&self) -> String {
        format!("{} {}", self.kind.name(), globset::escape(&self.subject))
    }

    pub fn key(&self, fleet: &Fleet) -> String {
        match self.member {
            None => self.ignore_line(),
            Some(i) => format!("{}: {}", fleet.members[i].name, self.ignore_line()),
        }
    }
}

#[cfg(test)]
#[path = "../tests/item.rs"]
mod tests;

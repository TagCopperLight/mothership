use strum::{EnumString, IntoStaticStr};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, IntoStaticStr, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum Kind {
    Package,
    Tool,
    File,
    Home,
    Etc,
    Service,
    UserService,
    Account,
    Dconf,
    Pref,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Machine,
    User,
}

impl Kind {
    pub fn level(self) -> Level {
        match self {
            Kind::Package | Kind::Etc | Kind::Service => Level::Machine,
            _ => Level::User,
        }
    }

    pub fn name(self) -> &'static str {
        self.into()
    }
}

#[cfg(test)]
#[path = "../tests/kind.rs"]
mod tests;

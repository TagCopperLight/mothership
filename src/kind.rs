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
    User,
    Shared,
    Key,
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
            Kind::Package | Kind::Etc | Kind::Service | Kind::User | Kind::Shared | Kind::Key => {
                Level::Machine
            }
            _ => Level::User,
        }
    }

    pub fn name(self) -> &'static str {
        self.into()
    }
}

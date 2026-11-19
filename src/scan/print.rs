use super::Report;
use crate::{
    fleet::Fleet,
    item::{Change, Item, Secret, Section},
    kind::Kind,
    style::{self, Paint, Style},
};

impl Kind {
    pub fn area(self) -> Area {
        match self {
            Kind::Package => Area::Packages,
            Kind::Tool => Area::Tools,
            Kind::File | Kind::Home | Kind::Etc => Area::Files,
            Kind::Service => Area::Services,
            Kind::UserService => Area::UserServices,
            Kind::Account => Area::Account,
            Kind::Dconf | Kind::Pref => Area::Settings,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Area {
    Packages,
    Tools,
    Files,
    Services,
    UserServices,
    Account,
    Settings,
}

impl Area {
    pub fn title(self) -> &'static str {
        match self {
            Area::Packages => "Packages",
            Area::Tools => "Tools",
            Area::Files => "Files",
            Area::Services => "Services",
            Area::UserServices => "User services",
            Area::Account => "Account",
            Area::Settings => "Settings",
        }
    }
}

pub fn heading(fleet: &Fleet, item: &Item) -> String {
    let area = item.kind.area().title();
    match owner(fleet, item) {
        Some(i) => format!("{area} · {}", fleet.members[i].name),
        None => area.to_owned(),
    }
}

pub fn owner(fleet: &Fleet, item: &Item) -> Option<usize> {
    item.member.filter(|_| fleet.members.len() > 1)
}

pub fn summary(report: &Report, style: Style, waiting: &dyn Fn(&Item) -> Option<u64>) {
    let count = |section| {
        report
            .shown
            .iter()
            .filter(|item| item.section == section)
            .count()
    };
    let put_off = report
        .shown
        .iter()
        .filter(|item| item.section != Section::Fleet && waiting(item).is_some())
        .count();
    let review = count(Section::Attention) + count(Section::Machine) - put_off;
    let apply = count(Section::Fleet);
    let bold = |n: usize| style.paint(n, &[Paint::Bold]);
    if review + apply + put_off == 0 {
        println!(
            "{} Nothing differs from the {} type",
            style.paint("✓", &[Paint::Green, Paint::Bold]),
            report.fleet.type_name
        );
    }
    if review > 0 {
        println!("{} to review     {}", bold(review), style.command("review"));
    }
    if apply > 0 {
        println!("{} to apply      {}", bold(apply), style.command("apply"));
    }
    if put_off > 0 {
        let later = format!("{put_off} put off for later");
        println!("{}", style.paint(later, &[Paint::Blue]));
    }
}

pub fn print_section(
    fleet: &Fleet,
    section: Section,
    items: &[&Item],
    style: Style,
    waiting: &dyn Fn(&Item) -> Option<u64>,
) {
    if items.is_empty() {
        return;
    }
    let title = format!("{} · {}", section.title(), items.len());
    println!("{}", style.paint(title, &[Paint::Bold, section.paint()]));
    print_groups(fleet, items, style, waiting);
}

pub fn print_groups(
    fleet: &Fleet,
    items: &[&Item],
    style: Style,
    waiting: &dyn Fn(&Item) -> Option<u64>,
) {
    let columns = Columns::of(items);
    for group in items.chunk_by(|a, b| heading(fleet, a) == heading(fleet, b)) {
        println!(
            "  {}",
            style.paint(heading(fleet, group[0]), &[Paint::Bold])
        );
        for item in group {
            println!("{}", line(item, style, columns, waiting(item)));
        }
    }
    println!();
}

#[derive(Clone, Copy)]
pub struct Columns {
    subject: usize,
    verb: usize,
}

impl Columns {
    const SUBJECT: usize = 40;

    pub fn of(items: &[&Item]) -> Self {
        let widest = |text: &dyn Fn(&Item) -> usize| items.iter().map(|i| text(i)).max();
        Columns {
            subject: widest(&|i| style::visible(&style::clean(&i.subject)))
                .unwrap_or(0)
                .min(Columns::SUBJECT),
            verb: widest(&|i| style::visible(&i.verb())).unwrap_or(0),
        }
    }
}

pub fn line(item: &Item, style: Style, columns: Columns, waiting: Option<u64>) -> String {
    let (sign, paint) = item.sign();
    let verb_paint = match item.section {
        Section::Fleet => Paint::Cyan,
        _ => paint,
    };
    let mut extras = Vec::new();
    if matches!(item.secret, Secret::Guessed | Secret::Fleet) && item.change != Change::InClear {
        extras.push(style.paint("secret", &[Paint::Magenta, Paint::Bold]));
    }
    if let Some(days) = waiting {
        let left = format!("later, back in {}", style::count(days as usize, "day"));
        extras.push(style.paint(left, &[Paint::Blue]));
    }
    if !item.note.is_empty() {
        extras.push(style.paint(style::clean(&item.note), &[Paint::Dim]));
    }
    let subject = style::clean(&item.subject);
    let subject = if waiting.is_some() {
        style.paint(subject, &[Paint::Dim])
    } else {
        subject
    };
    let text = format!(
        "    {} {}  {}  {}",
        style.paint(sign, &[paint, Paint::Bold]),
        style::pad(&subject, columns.subject),
        style::pad(&style.paint(item.verb(), &[verb_paint]), columns.verb),
        extras.join("  "),
    );
    let text = text.trim_end();
    if style.terminal {
        style::clip(text, style::width())
    } else {
        text.to_owned()
    }
}

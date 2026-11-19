use std::collections::BTreeMap;

pub fn line(key: &str, value: &str) -> String {
    format!("user_pref(\"{key}\", {value});")
}

pub fn parse_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix("user_pref(")?;
    let (key, rest) = rest.trim_start().strip_prefix('"')?.split_once('"')?;
    let value = rest.trim_start().strip_prefix(',')?;
    let value = value
        .trim()
        .strip_suffix(';')?
        .trim_end()
        .strip_suffix(')')?;
    let value = value.trim();
    is_literal(value).then_some((key, value))
}

fn is_literal(value: &str) -> bool {
    if matches!(value, "true" | "false") {
        return true;
    }
    let digits = value.strip_prefix('-').unwrap_or(value);
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return false;
    };
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return false,
            '\\' if !matches!(chars.next(), Some('"' | '\\' | 'n' | 'r')) => return false,
            _ => {}
        }
    }
    true
}

pub fn parse(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(parse_line)
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[cfg(test)]
#[path = "../tests/pref.rs"]
mod tests;

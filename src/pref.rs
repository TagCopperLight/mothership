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
    Some((key, value.trim()))
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

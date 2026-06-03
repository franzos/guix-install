fn parse(raw: &'static str) -> Vec<&'static str> {
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect()
}

pub struct Locale {
    pub label: &'static str,
    pub code: &'static str,
}

pub fn locales() -> Vec<Locale> {
    include_str!("data/locales.txt")
        .lines()
        .filter_map(|line| {
            let (label, code) = line.trim_end().split_once('\t')?;
            Some(Locale { label, code })
        })
        .collect()
}

pub fn timezones() -> Vec<&'static str> {
    parse(include_str!("data/timezones.txt"))
}

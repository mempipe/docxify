//! Dates as Word writes them: a `YYYY-MM-DD` value, or one with a time of
//! day, shown through a date picture such as `dddd, MMMM d, yyyy`.
//!
//! Names are English whatever the document's language.

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// The picture Word uses when a control or a `DATE` field names none.
pub(crate) const DEFAULT_PICTURE: &str = "M/d/yyyy";

/// The picture Word uses when a `TIME` field names none.
pub(crate) const DEFAULT_TIME_PICTURE: &str = "h:mm AM/PM";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Date {
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

impl Date {
    /// A real date written `YYYY-MM-DD`, in years 1 to 9999, at midnight.
    pub(crate) fn parse(text: &str) -> Option<Date> {
        Date::parse_time(&format!("{text}T00:00:00"))
    }

    /// A real date and time written `YYYY-MM-DDThh:mm:ss`.
    pub(crate) fn parse_time(text: &str) -> Option<Date> {
        let b = text.as_bytes();
        let digits = |r: std::ops::Range<usize>| {
            b[r.clone()]
                .iter()
                .all(u8::is_ascii_digit)
                .then(|| text[r].parse::<u32>().ok())
                .flatten()
        };
        let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
        if b.len() != 19 || separators.iter().any(|&(i, c)| b[i] != c) {
            return None;
        }
        let date = Date {
            year: digits(0..4)?,
            month: digits(5..7)?,
            day: digits(8..10)?,
            hour: digits(11..13)?,
            minute: digits(14..16)?,
            second: digits(17..19)?,
        };
        let valid = date.year >= 1
            && (1..=12).contains(&date.month)
            && (1..=days_in(date.year, date.month)).contains(&date.day)
            && date.hour < 24
            && date.minute < 60
            && date.second < 60;
        valid.then_some(date)
    }

    /// The value Word stores, as in `w:fullDate`.
    pub(crate) fn full(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// Sunday is 0.
    fn weekday(&self) -> usize {
        // Zeller's congruence, with January and February as months 13 and
        // 14 of the year before.
        let (m, y) = match self.month {
            1 | 2 => (self.month + 12, self.year - 1),
            m => (m, self.year),
        };
        let h = (self.day + 13 * (m + 1) / 5 + y + y / 4 - y / 100 + y / 400) % 7;
        // Zeller counts from Saturday.
        ((h + 6) % 7) as usize
    }

    /// The date through a picture: `d`, `dd`, `ddd` and `dddd` for the day,
    /// `M` to `MMMM` for the month, `yy` and `yyyy` for the year, `h` and
    /// `H` for the hour on a 12- or 24-hour clock, `m` and `s`, each doubled
    /// for two digits, `AM/PM` or `am/pm`, and text in single quotes as is.
    pub(crate) fn format(&self, picture: &str) -> String {
        let chars: Vec<char> = picture.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\'' {
                let end = chars[i + 1..]
                    .iter()
                    .position(|&q| q == '\'')
                    .map_or(chars.len(), |p| i + 1 + p);
                out.extend(&chars[i + 1..end]);
                i = end + 1;
                continue;
            }
            for ampm in ["AM/PM", "am/pm"] {
                if picture_at(&chars, i, ampm) {
                    let half = if self.hour < 12 { 0..2 } else { 3..5 };
                    out.push_str(&ampm[half]);
                    i += ampm.len();
                }
            }
            if i >= chars.len() {
                break;
            }
            let c = chars[i];
            let run = chars[i..].iter().take_while(|&&r| r == c).count();
            let text = match (c, run) {
                ('d', 1) => self.day.to_string(),
                ('d', 2) => format!("{:02}", self.day),
                ('d', 3) => DAYS[self.weekday()][..3].to_string(),
                ('d', _) => DAYS[self.weekday()].to_string(),
                ('M', 1) => self.month.to_string(),
                ('M', 2) => format!("{:02}", self.month),
                ('M', 3) => MONTHS[self.month as usize - 1][..3].to_string(),
                ('M', _) => MONTHS[self.month as usize - 1].to_string(),
                ('y', 1 | 2) => format!("{:02}", self.year % 100),
                ('y', _) => format!("{:04}", self.year),
                ('h' | 'H' | 'm' | 's', _) => {
                    let value = match c {
                        'h' if self.hour.is_multiple_of(12) => 12,
                        'h' => self.hour % 12,
                        'H' => self.hour,
                        'm' => self.minute,
                        _ => self.second,
                    };
                    match run {
                        1 => value.to_string(),
                        _ => format!("{value:02}"),
                    }
                }
                _ => chars[i..i + run].iter().collect(),
            };
            out.push_str(&text);
            i += run;
        }
        out
    }
}

fn picture_at(chars: &[char], at: usize, token: &str) -> bool {
    token
        .chars()
        .enumerate()
        .all(|(k, t)| chars.get(at + k) == Some(&t))
}

fn days_in(year: u32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_real_dates() {
        assert!(Date::parse("2024-02-29").is_some());
        assert!(Date::parse("2000-02-29").is_some());
        for bad in [
            "2023-02-29",
            "1900-02-29",
            "2026-13-01",
            "2026-04-31",
            "0000-01-01",
            "2026-1-01",
            "26-01-01xx",
            "2026/01/01",
            "+026-01-01",
            "2026-01-01T00:00:00",
        ] {
            assert_eq!(Date::parse(bad), None, "{bad}");
        }
        assert!(Date::parse_time("2026-11-16T23:59:59").is_some());
        for bad in [
            "2026-11-16T24:00:00",
            "2026-11-16T12:60:00",
            "2026-11-16T12:00:60",
            "2026-11-16 12:00:00",
            "2026-11-16T12:00",
            "2026-02-30T12:00:00",
        ] {
            assert_eq!(Date::parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn formats_pictures_as_word_does() {
        let date = Date::parse("2026-11-16").expect("a date");
        assert_eq!(date.full(), "2026-11-16T00:00:00Z");
        for (picture, shown) in [
            (DEFAULT_PICTURE, "11/16/2026"),
            ("dddd, MMMM d, yyyy", "Monday, November 16, 2026"),
            ("ddd d MMM yy", "Mon 16 Nov 26"),
            ("dd/MM/yyyy", "16/11/2026"),
            ("yyyy-MM-dd", "2026-11-16"),
            ("d 'de' MMMM", "16 de November"),
            ("h:mm AM/PM", "12:00 AM"),
            ("HH:mm", "00:00"),
        ] {
            assert_eq!(date.format(picture), shown, "{picture}");
        }
        let time = Date::parse_time("2026-11-16T14:05:09").expect("a time");
        assert_eq!(time.full(), "2026-11-16T14:05:09Z");
        for (picture, shown) in [
            (DEFAULT_TIME_PICTURE, "2:05 PM"),
            ("hh:mm:ss am/pm", "02:05:09 pm"),
            ("H:m:s", "14:5:9"),
            ("HH:mm", "14:05"),
            ("d/M/yyyy h:mm AM/PM", "16/11/2026 2:05 PM"),
        ] {
            assert_eq!(time.format(picture), shown, "{picture}");
        }
        let noon = Date::parse_time("2026-11-16T12:00:00").expect("a time");
        assert_eq!(noon.format("h AM/PM"), "12 PM");
        let days = ["2026-11-15", "2000-03-01", "1900-02-28", "2024-01-01"];
        let names: Vec<String> = days
            .iter()
            .map(|d| Date::parse(d).expect("a date").format("dddd"))
            .collect();
        assert_eq!(names, ["Sunday", "Wednesday", "Wednesday", "Monday"]);
    }
}

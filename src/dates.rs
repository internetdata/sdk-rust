//! The two date forms the spec names, read as written and nothing else.
//!
//! `scripts/generate.sh` points every date member of the generated models here.
//! chrono's own serde reader is a relaxed one: through 2.7.2 it took
//! `2026-10-5`, ` 2026-10-20`, `+2026-10-20`, a space for the `T`, a lowercase
//! `t` or `z`, `+0500` and a leap second, each as some date, where the spec
//! promises RFC 3339's `full-date` for `format: date` and its `date-time`, with
//! a zone, for `format: date-time`. Anything else is an answer this client
//! cannot read, the same retried `server_error` as a member of the wrong type.

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::Deserialize;
use serde::de::{Deserializer, Error as _};

/// A `format: date` member.
pub(crate) fn date<'de, D: Deserializer<'de>>(deserializer: D) -> Result<NaiveDate, D::Error> {
    let value = String::deserialize(deserializer)?;
    parse_date(&value).ok_or_else(|| D::Error::custom(format!("{value:?} is not a date")))
}

/// A required `format: date-time` member.
pub(crate) fn date_time<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<DateTime<FixedOffset>, D::Error> {
    let value = String::deserialize(deserializer)?;
    parse_date_time(&value).ok_or_else(|| D::Error::custom(format!("{value:?} is not a date-time")))
}

/// A required, nullable `format: date-time` member: present, and null or a
/// date-time. Missing is refused as it is for `Option::deserialize`, which this
/// replaces.
pub(crate) fn option_date_time<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<DateTime<FixedOffset>>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|value| {
            parse_date_time(&value)
                .ok_or_else(|| D::Error::custom(format!("{value:?} is not a date-time")))
        })
        .transpose()
}

/// `YYYY-MM-DD`, on a day that exists.
fn parse_date(value: &str) -> Option<NaiveDate> {
    let b = value.as_bytes();
    if b.len() != 10 || !is_date_shape(b) {
        return None;
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

/// `YYYY-MM-DDTHH:MM:SS`, an optional fraction, then `Z` or `+HH:MM`/`-HH:MM`,
/// at a time that exists: no hour 24, no minute or second 60.
fn parse_date_time(value: &str) -> Option<DateTime<FixedOffset>> {
    let b = value.as_bytes();
    if b.len() < 20 || !is_date_shape(&b[..10]) || b[10] != b'T' {
        return None;
    }
    let time = &b[11..19];
    if !(digits(&time[0..2]) && time[2] == b':' && digits(&time[3..5]) && time[5] == b':')
        || !digits(&time[6..8])
        || &time[6..8] >= b"60".as_slice()
    {
        return None;
    }
    let mut rest = &b[19..];
    if let Some(fraction) = rest.strip_prefix(b".") {
        let n = fraction.iter().take_while(|c| c.is_ascii_digit()).count();
        if n == 0 {
            return None;
        }
        rest = &fraction[n..];
    }
    let zoned = rest == b"Z"
        || (rest.len() == 6
            && matches!(rest[0], b'+' | b'-')
            && digits(&rest[1..3])
            && rest[3] == b':'
            && digits(&rest[4..6]));
    if !zoned {
        return None;
    }
    DateTime::parse_from_rfc3339(value).ok()
}

fn is_date_shape(b: &[u8]) -> bool {
    digits(&b[0..4]) && b[4] == b'-' && digits(&b[5..7]) && b[7] == b'-' && digits(&b[8..10])
}

fn digits(b: &[u8]) -> bool {
    b.iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::{parse_date, parse_date_time};

    #[test]
    fn a_date_is_a_full_date_on_a_day_that_exists() {
        assert_eq!(parse_date("2026-10-19").map(|d| d.to_string()), Some("2026-10-19".to_owned()));
        assert_eq!(parse_date("2028-02-29").map(|d| d.to_string()), Some("2028-02-29".to_owned()));
        for value in [
            "",
            "x",
            "tomorrow",
            "monday",
            "2026-02-30",
            "2026-11-31",
            "2027-02-29",
            "2026-13-01",
            "2026-10-5",
            "2026-1-05",
            "+2026-10-20",
            " 2026-10-20",
            "2026-10-20 ",
            "-0001-10-20",
            "+12026-10-20",
            "20261020",
            "2026-W43-2",
            "2026-293",
            "2026-10-20T10:00:00Z",
            "２026-10-20",
        ] {
            assert_eq!(parse_date(value), None, "{value:?}");
        }
    }

    #[test]
    fn a_date_time_is_rfc_3339_with_a_zone_at_a_time_that_exists() {
        for (value, read) in [
            ("2026-09-04T10:00:00.123Z", "2026-09-04T10:00:00.123+00:00"),
            ("2026-09-04T10:00:00Z", "2026-09-04T10:00:00+00:00"),
            ("2026-09-04T10:00:00+05:00", "2026-09-04T10:00:00+05:00"),
            ("2026-09-04T10:00:00.5-08:30", "2026-09-04T10:00:00.500-08:30"),
            ("2028-02-29T23:59:59Z", "2028-02-29T23:59:59+00:00"),
        ] {
            assert_eq!(
                parse_date_time(value).map(|d| d.to_rfc3339()),
                Some(read.to_owned()),
                "{value:?}"
            );
        }
        for value in [
            "",
            "x",
            "tomorrow",
            "monday",
            "10:00",
            "2026-10-20",
            "2026-10-20T10:00:00",
            "2026-10-20T10:00Z",
            "2026-02-30T00:00:00Z",
            "2026-11-31T00:00:00Z",
            "2026-10-20T24:00:00Z",
            "2026-10-20T23:60:00Z",
            "2026-10-20T23:59:60Z",
            "2026-10-20 10:00:00Z",
            "2026-10-20t10:00:00Z",
            "2026-10-20T10:00:00z",
            "2026-10-20T10:00:00+0500",
            "2026-10-20T10:00:00+05",
            "2026-10-20T10:00:00.Z",
            "  2026-10-20T10:00:00Z",
            "2026-10-20T10:00:00Z ",
            "+002026-10-20T10:00:00Z",
            "2026-10-20T10:00:00+24:00",
            "2026-10-20T10:00:00 +05:00",
            "2026-10-20T10:00:00+05:00x",
            "2026-10-20T10:00:00.１Z",
        ] {
            assert_eq!(parse_date_time(value), None, "{value:?}");
        }
    }
}

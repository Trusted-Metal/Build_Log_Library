//! Dates for the headers of a ZIP file and for the manifest, from the system clock, without a date library.

use std::time::{SystemTime, UNIX_EPOCH};

/// A civil date and time in UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// Seconds since the Unix epoch now (0 if the clock is before it).
pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The civil date of a Unix time (the days-to-date algorithm of H. Hinnant).
pub(crate) fn civil(unix: u64) -> Civil {
    let days = (unix / 86_400) as i64;
    let secs = (unix % 86_400) as u32;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    Civil {
        year,
        month,
        day,
        hour: secs / 3_600,
        minute: secs % 3_600 / 60,
        second: secs % 60,
    }
}

/// The MS-DOS time and date words of a ZIP header (two-second resolution; years before 1980 are 1980).
pub(crate) fn dos(unix: u64) -> (u16, u16) {
    let c = civil(unix);
    let year = c.year.clamp(1980, 2107) as u32;
    let time = (c.hour << 11) | (c.minute << 5) | (c.second / 2);
    let date = ((year - 1980) << 9) | (c.month << 5) | c.day;
    (time as u16, date as u16)
}

/// `2026-11-13T15:28:13Z`.
pub(crate) fn iso8601(unix: u64) -> String {
    let c = civil(unix);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unix_time_becomes_its_civil_date_in_utc() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00Z"); // a leap day
        assert_eq!(iso8601(1_794_583_693), "2026-11-13T15:28:13Z");
        assert_eq!(iso8601(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn dos_words_hold_the_date_and_the_time_in_two_second_steps() {
        let (time, date) = dos(1_794_583_693);
        assert_eq!(time >> 11, 15);
        assert_eq!((time >> 5) & 63, 28);
        assert_eq!(time & 31, 6); // 13 s -> 6 steps of 2 s
        assert_eq!(date >> 9, 2026 - 1980);
        assert_eq!((date >> 5) & 15, 11);
        assert_eq!(date & 31, 13);
        // before 1980 the words say 1980
        assert_eq!(dos(0).1 >> 9, 0);
    }
}

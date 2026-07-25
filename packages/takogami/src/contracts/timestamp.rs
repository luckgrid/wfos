//! Strict RFC 3339 timestamp parsing shared by durable record semantic validation (S6.1-10)
//! and session query ordering (S6.1-08). Both must agree on the same parsed instant so that
//! validation and ordering never disagree about "which record is newer."

/// Parse a strict `YYYY-MM-DDTHH:MM:SS(.fraction)?(Z|+HH:MM|-HH:MM)` timestamp into whole
/// seconds since the Unix epoch, normalized to UTC. Sub-second fractions are validated as
/// digits but not retained (second resolution is sufficient for record ordering).
pub fn parse_rfc3339_utc_seconds(input: &str) -> Result<i64, String> {
    let bytes = input.as_bytes();
    if bytes.len() < 20 {
        return Err(format!("timestamp too short: {input:?}"));
    }
    let digits = |s: &str| -> Result<i64, String> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("expected digits, got {s:?}"));
        }
        s.parse::<i64>()
            .map_err(|_| format!("invalid numeric field: {s:?}"))
    };

    let mut rest = input;
    let take = |rest: &mut &str, n: usize, ctx: &str| -> Result<String, String> {
        if rest.len() < n {
            return Err(format!("truncated {ctx} in timestamp: {input:?}"));
        }
        let (head, tail) = rest.split_at(n);
        *rest = tail;
        Ok(head.to_string())
    };
    let expect = |rest: &mut &str, ch_options: &[char], ctx: &str| -> Result<(), String> {
        let Some(c) = rest.chars().next() else {
            return Err(format!("missing {ctx} in timestamp: {input:?}"));
        };
        if !ch_options.contains(&c) {
            return Err(format!("expected {ctx} in timestamp: {input:?}"));
        }
        *rest = &rest[c.len_utf8()..];
        Ok(())
    };

    let year = digits(&take(&mut rest, 4, "year")?)?;
    expect(&mut rest, &['-'], "date separator")?;
    let month = digits(&take(&mut rest, 2, "month")?)?;
    expect(&mut rest, &['-'], "date separator")?;
    let day = digits(&take(&mut rest, 2, "day")?)?;
    expect(&mut rest, &['T', 't'], "date/time separator")?;
    let hour = digits(&take(&mut rest, 2, "hour")?)?;
    expect(&mut rest, &[':'], "time separator")?;
    let minute = digits(&take(&mut rest, 2, "minute")?)?;
    expect(&mut rest, &[':'], "time separator")?;
    let second = digits(&take(&mut rest, 2, "second")?)?;

    if let Some(stripped) = rest.strip_prefix('.') {
        let frac_len = stripped.bytes().take_while(|b| b.is_ascii_digit()).count();
        if frac_len == 0 {
            return Err(format!("empty fractional seconds: {input:?}"));
        }
        rest = &stripped[frac_len..];
    }

    let offset_seconds = if rest == "Z" || rest == "z" {
        0
    } else {
        let mut off = rest;
        let sign = match off.chars().next() {
            Some('+') => 1,
            Some('-') => -1,
            _ => return Err(format!("missing UTC offset/Z in timestamp: {input:?}")),
        };
        off = &off[1..];
        let oh = digits(&take(&mut off, 2, "offset hour")?)?;
        expect(&mut off, &[':'], "offset separator")?;
        let om = digits(&take(&mut off, 2, "offset minute")?)?;
        if !off.is_empty() {
            return Err(format!("trailing content in timestamp: {input:?}"));
        }
        if !(0..=23).contains(&oh) || !(0..=59).contains(&om) {
            return Err(format!("offset out of range: {input:?}"));
        }
        sign * (oh * 3600 + om * 60)
    };

    if !(1..=9999).contains(&year) {
        return Err(format!("year out of range: {input:?}"));
    }
    if !(1..=12).contains(&month) {
        return Err(format!("month out of range: {input:?}"));
    }
    if day < 1 || day > days_in_month(year, month) {
        return Err(format!("day out of range: {input:?}"));
    }
    if !(0..=23).contains(&hour) {
        return Err(format!("hour out of range: {input:?}"));
    }
    if !(0..=59).contains(&minute) {
        return Err(format!("minute out of range: {input:?}"));
    }
    // Allow a positive leap second (60) as RFC 3339 permits; treat it as second 59 for ordering.
    if !(0..=60).contains(&second) {
        return Err(format!("second out of range: {input:?}"));
    }

    let days = days_from_civil(year, month as u32, day as u32);
    let local_seconds = days * 86400 + hour * 3600 + minute * 60 + second.min(59);
    Ok(local_seconds - offset_seconds)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Howard Hinnant's `days_from_civil`, the exact inverse of `civil_from_days` used to render
/// generated timestamps.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_z_and_zero_offset_agree() {
        let z = parse_rfc3339_utc_seconds("2026-07-21T09:00:00Z").unwrap();
        let plus = parse_rfc3339_utc_seconds("2026-07-21T09:00:00+00:00").unwrap();
        assert_eq!(z, plus);
    }

    #[test]
    fn same_instant_different_offsets_are_equal() {
        let a = parse_rfc3339_utc_seconds("2026-07-21T09:00:00Z").unwrap();
        let b = parse_rfc3339_utc_seconds("2026-07-21T11:00:00+02:00").unwrap();
        let c = parse_rfc3339_utc_seconds("2026-07-21T04:00:00-05:00").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn lexically_later_but_chronologically_earlier() {
        // "T23" sorts lexically after "T01", but the +10 offset pushes it before UTC noon.
        let lexically_later = parse_rfc3339_utc_seconds("2026-07-21T23:00:00+10:00").unwrap();
        let lexically_earlier = parse_rfc3339_utc_seconds("2026-07-21T14:00:00Z").unwrap();
        assert!(lexically_later < lexically_earlier);
    }

    #[test]
    fn rejects_invalid_calendar_dates() {
        assert!(parse_rfc3339_utc_seconds("2026-02-30T00:00:00Z").is_err());
        assert!(parse_rfc3339_utc_seconds("2026-13-01T00:00:00Z").is_err());
        assert!(parse_rfc3339_utc_seconds("2026-00-01T00:00:00Z").is_err());
    }

    #[test]
    fn rejects_missing_offset_and_malformed_shapes() {
        assert!(parse_rfc3339_utc_seconds("2026-07-21T09:00:00").is_err());
        assert!(parse_rfc3339_utc_seconds("not-a-timestamp").is_err());
        assert!(parse_rfc3339_utc_seconds("2026-07-21 09:00:00Z").is_err());
    }

    #[test]
    fn accepts_fractional_seconds() {
        assert!(parse_rfc3339_utc_seconds("2026-07-21T09:00:00.123Z").is_ok());
    }
}

use crate::model::Recurrence;
use anyhow::{Context, Result, bail};
use chrono::Weekday;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime};

pub fn parse_datetime(date: &str, time: &str) -> Result<DateTime<Local>> {
    let today = Local::now().date_naive();
    let date = match date.trim().to_ascii_lowercase().as_str() {
        "today" => today,
        "tomorrow" => today + Duration::days(1),
        "yesterday" => today - Duration::days(1),
        other => NaiveDate::parse_from_str(other, "%Y-%m-%d").with_context(|| {
            format!("invalid date '{other}', expected today/tomorrow/yesterday/YYYY-MM-DD")
        })?,
    };

    let time = parse_time(time)?;
    date.and_time(time)
        .and_local_timezone(Local)
        .single()
        .context("scheduled local time is ambiguous or invalid")
}

pub fn parse_time(value: &str) -> Result<NaiveTime> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed == "now" {
        return Ok(Local::now().time());
    }
    NaiveTime::parse_from_str(&trimmed, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(&trimmed, "%H:%M:%S"))
        .with_context(|| format!("invalid time '{value}', expected HH:MM or now"))
}

pub fn parse_every(value: &str) -> Result<Recurrence> {
    let duration = humantime::parse_duration(value)
        .with_context(|| format!("invalid interval '{value}', try '10 minutes' or '2 hours'"))?;
    let seconds = i64::try_from(duration.as_secs()).context("interval is too large")?;
    if seconds < 60 {
        bail!("intervals shorter than 60 seconds are intentionally not supported");
    }
    Ok(Recurrence::Every { seconds })
}

pub fn parse_weekdays(value: &str, time: NaiveTime) -> Result<Recurrence> {
    let mut weekdays = Vec::new();
    for part in value.split(',') {
        let weekday = match part.trim().to_ascii_lowercase().as_str() {
            "mon" | "monday" => Weekday::Mon,
            "tue" | "tues" | "tuesday" => Weekday::Tue,
            "wed" | "wednesday" => Weekday::Wed,
            "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
            "fri" | "friday" => Weekday::Fri,
            "sat" | "saturday" => Weekday::Sat,
            "sun" | "sunday" => Weekday::Sun,
            "" => continue,
            other => bail!("unknown weekday '{other}'"),
        };
        weekdays.push(weekday);
    }
    weekdays.sort_by_key(|day| day.num_days_from_monday());
    weekdays.dedup();
    if weekdays.is_empty() {
        bail!("at least one weekday is required");
    }
    Ok(Recurrence::WeeklyAt { weekdays, time })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_relative_dates() {
        parse_datetime("today", "now").unwrap();
        parse_datetime("yesterday", "09:00").unwrap();
        parse_datetime("tomorrow", "23:59").unwrap();
    }

    #[test]
    fn parses_interval() {
        assert!(matches!(
            parse_every("10 minutes").unwrap(),
            Recurrence::Every { .. }
        ));
    }
}

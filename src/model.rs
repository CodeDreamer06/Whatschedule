use anyhow::{Result, bail};
use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecipientKind {
    Contact,
    Group,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipient {
    pub jid: String,
    pub name: String,
    pub kind: RecipientKind,
}

impl Recipient {
    pub fn from_input(input: &str, name: Option<String>, force_group: bool) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            bail!("recipient cannot be empty");
        }

        let kind = if force_group || trimmed.ends_with("@g.us") {
            RecipientKind::Group
        } else {
            RecipientKind::Contact
        };

        let jid = if trimmed.contains('@') {
            trimmed.to_string()
        } else {
            let mut phone: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
            if phone.len() < 7 {
                bail!("phone numbers must include country code, e.g. +15551234567");
            }

            if !trimmed.starts_with('+') {
                if phone.len() == 10 {
                    phone = format!("91{phone}");
                } else if phone.len() == 11 && phone.starts_with('0') {
                    phone = format!("91{}", &phone[1..]);
                }
            }

            format!("{phone}@s.whatsapp.net")
        };

        Ok(Self {
            name: name.unwrap_or_else(|| trimmed.to_string()),
            jid,
            kind,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Sent,
    Cancelled,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Pending => write!(f, "pending"),
            JobStatus::Sent => write!(f, "sent"),
            JobStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl TryFrom<&str> for JobStatus {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self> {
        Ok(match value {
            "pending" => JobStatus::Pending,
            "sent" => JobStatus::Sent,
            "cancelled" => JobStatus::Cancelled,
            other => bail!("unknown job status: {other}"),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Recurrence {
    None,
    Every {
        seconds: i64,
    },
    DailyAt {
        time: NaiveTime,
    },
    WeeklyAt {
        weekdays: Vec<Weekday>,
        time: NaiveTime,
    },
}

impl Recurrence {
    pub fn next_after(&self, after: DateTime<Local>) -> Option<DateTime<Local>> {
        match self {
            Recurrence::None => None,
            Recurrence::Every { seconds } => Some(after + Duration::seconds(*seconds)),
            Recurrence::DailyAt { time } => {
                let today = after.date_naive().and_time(*time);
                let today = today.and_local_timezone(Local).single();
                match today {
                    Some(dt) if dt > after => Some(dt),
                    _ => (after.date_naive() + Duration::days(1))
                        .and_time(*time)
                        .and_local_timezone(Local)
                        .single(),
                }
            }
            Recurrence::WeeklyAt { weekdays, time } => {
                for offset in 0..=7 {
                    let date = after.date_naive() + Duration::days(offset);
                    if weekdays.contains(&date.weekday())
                        && let Some(dt) = date.and_time(*time).and_local_timezone(Local).single()
                        && dt > after
                    {
                        return Some(dt);
                    }
                }
                None
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: Option<i64>,
    pub message: String,
    pub file_path: Option<PathBuf>,
    pub run_at: DateTime<Local>,
    pub recurrence: Recurrence,
    pub recipients: Vec<Recipient>,
    pub status: JobStatus,
}

impl Job {
    pub fn validate(&self) -> Result<()> {
        if self.message.trim().is_empty() && self.file_path.is_none() {
            bail!("a job needs message text, a file, or both");
        }
        if self.recipients.is_empty() {
            bail!("a job needs at least one recipient");
        }
        if let Some(path) = &self.file_path
            && !path.exists()
        {
            bail!("file does not exist: {}", path.display());
        }
        Ok(())
    }
}

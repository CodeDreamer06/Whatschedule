use crate::model::{Job, JobStatus, Recipient, Recurrence};
use crate::timeparse;
use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "whatschedule")]
#[command(about = "Schedule WhatsApp Web messages from a resilient Rust CLI.")]
pub struct Cli {
    #[arg(long, global = true, env = "WHATSCHEDULE_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    #[arg(long, global = true, env = "WHATSCHEDULE_PHONE")]
    pub phone: Option<String>,

    #[arg(long, global = true, env = "WHATSCHEDULE_PAIR_CODE")]
    pub pair_code: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Interactive,
    Run,
    SendDue,
    Schedule(ScheduleArgs),
    List,
    Cancel {
        id: i64,
    },
    #[command(subcommand)]
    Contacts(ContactsCommand),
}

#[derive(Debug, Subcommand)]
pub enum ContactsCommand {
    Add(AddContactArgs),
    List,
    Remove { name: String },
}

#[derive(Debug, Args)]
pub struct AddContactArgs {
    pub name: String,
    pub to: String,
    #[arg(long)]
    pub group: bool,
}

#[derive(Debug, Args)]
pub struct ScheduleArgs {
    #[arg(long = "to", required = true)]
    pub to: Vec<String>,

    #[arg(long)]
    pub message: Option<String>,

    #[arg(long)]
    pub file: Option<PathBuf>,

    #[arg(long, default_value = "today")]
    pub date: String,

    #[arg(long, default_value = "now")]
    pub time: String,

    #[arg(long, conflicts_with_all = ["daily_at", "weekdays"])]
    pub every: Option<String>,

    #[arg(long = "daily-at", conflicts_with_all = ["every", "weekdays"])]
    pub daily_at: Option<String>,

    #[arg(long, conflicts_with = "every")]
    pub weekdays: Option<String>,

    #[arg(long = "group")]
    pub group: bool,
}

impl ScheduleArgs {
    pub fn into_job(self) -> Result<Job> {
        let run_at = timeparse::parse_datetime(&self.date, &self.time)?;
        let recurrence = if let Some(every) = self.every {
            timeparse::parse_every(&every)?
        } else if let Some(daily_at) = self.daily_at {
            Recurrence::DailyAt {
                time: timeparse::parse_time(&daily_at)?,
            }
        } else if let Some(weekdays) = self.weekdays {
            timeparse::parse_weekdays(&weekdays, timeparse::parse_time(&self.time)?)?
        } else {
            Recurrence::None
        };

        let message = self.message.unwrap_or_default();
        if message.trim().is_empty() && self.file.is_none() {
            bail!("provide --message, --file, or both");
        }

        let recipients = self
            .to
            .iter()
            .map(|to| Recipient::from_input(to, None, self.group))
            .collect::<Result<Vec<_>>>()?;

        let job = Job {
            id: None,
            message,
            file_path: self.file,
            run_at,
            recurrence,
            recipients,
            status: JobStatus::Pending,
        };
        job.validate()?;
        Ok(job)
    }
}

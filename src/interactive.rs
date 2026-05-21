use crate::model::{Job, JobStatus, Recipient, Recurrence};
use crate::store::Store;
use crate::timeparse;
use crate::whatsapp::WhatsApp;
use anyhow::{Context, Result};
use colored::Colorize;
use inquire::{Confirm, MultiSelect, Select, Text};
use std::path::PathBuf;

pub async fn run(store: &Store, wa: &WhatsApp) -> Result<()> {
    loop {
        let recipients = select_recipients(store, wa).await?;
        let message = Text::new("Message")
            .with_help_message("Leave empty only if you attach a file.")
            .prompt()?;
        let file = Text::new("File path")
            .with_help_message("Optional: ./image.jpg, ./invoice.pdf, ./voice.m4a, ./clip.mp4")
            .with_default("")
            .prompt()?;
        let file_path = if file.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(file.trim()))
        };

        let date = Text::new("Date")
            .with_default("today")
            .with_help_message("today, tomorrow, yesterday, or YYYY-MM-DD")
            .prompt()?;
        let time = Text::new("Time")
            .with_default("now")
            .with_help_message("now or HH:MM")
            .prompt()?;
        let run_at = timeparse::parse_datetime(&date, &time)?;
        let recurrence = ask_recurrence(&time)?;

        let job = Job {
            id: None,
            message,
            file_path,
            run_at,
            recurrence,
            recipients,
            status: JobStatus::Pending,
        };
        job.validate()?;
        let id = store.insert_job(&job)?;
        println!(
            "{} Job #{id} scheduled for {}.",
            "✅".green(),
            job.run_at.format("%Y-%m-%d %H:%M:%S")
        );

        if !Confirm::new("Schedule another message?")
            .with_default(false)
            .prompt()?
        {
            break;
        }
    }
    Ok(())
}

async fn select_recipients(store: &Store, wa: &WhatsApp) -> Result<Vec<Recipient>> {
    println!(
        "\n{}\n{}",
        "💡 Note: WhatsApp groups are automatically synchronized.".dimmed(),
        "   Personal contacts must be added to your saved contacts to appear here.".dimmed()
    );

    let mut choices = store.list_contacts()?;
    match wa.groups().await {
        Ok(groups) => {
            for group in groups {
                if !choices.iter().any(|c| c.jid == group.jid) {
                    choices.push(group);
                }
            }
        }
        Err(err) => {
            println!("{} Could not load groups: {err}", "⚠️".yellow());
        }
    }

    choices.sort_by_key(|r| format!("{}{}", kind_prefix(r), r.name.to_ascii_lowercase()));

    let add_new_label = "➕ [Add a new personal contact...]".to_string();

    if choices.is_empty() {
        println!("{}", "📇 No saved contacts yet. Add one now.".cyan());
        let name = Text::new("Name").prompt()?;
        let to = Text::new("Phone or JID").prompt()?;
        let recipient = Recipient::from_input(&to, Some(name.clone()), false)?;
        store.upsert_contact(&name, &recipient)?;
        return Ok(vec![recipient]);
    }

    let mut labels = vec![add_new_label.clone()];
    labels.extend(
        choices
            .iter()
            .map(|r| format!("{} {} <{}>", kind_prefix(r), r.name, r.jid)),
    );

    let selected = MultiSelect::new("Recipients", labels)
        .with_help_message("Use space to select. Select 'Add a new personal contact...' to enter details inline.")
        .prompt()?;

    let mut recipients = Vec::new();
    let mut added_any_new = false;

    for label in &selected {
        if label == &add_new_label {
            added_any_new = true;
            continue;
        }
        let idx = choices
            .iter()
            .position(|r| label.ends_with(&format!("<{}>", r.jid)))
            .context("selected recipient disappeared")?;
        recipients.push(choices[idx].clone());
    }

    if added_any_new {
        loop {
            println!("{}", "\n📇 Add a new personal contact:".cyan());
            let name = Text::new("Name").prompt()?;
            if name.trim().is_empty() {
                println!("{} Contact name cannot be empty.", "⚠️".yellow());
                continue;
            }
            let to = Text::new("Phone or JID").prompt()?;
            if to.trim().is_empty() {
                println!("{} Phone or JID cannot be empty.", "⚠️".yellow());
                continue;
            }
            match Recipient::from_input(&to, Some(name.clone()), false) {
                Ok(recipient) => {
                    if let Err(e) = store.upsert_contact(&name, &recipient) {
                        println!("{} Could not save contact to database: {e}", "⚠️".yellow());
                    } else {
                        println!("{} Saved {} <{}> successfully.", "✅".green(), name, recipient.jid);
                        recipients.push(recipient);
                    }
                }
                Err(e) => {
                    println!("{} Invalid recipient format: {e}", "⚠️".yellow());
                }
            }

            if !Confirm::new("Add another new personal contact?")
                .with_default(false)
                .prompt()?
            {
                break;
            }
        }
    }

    Ok(recipients)
}

fn ask_recurrence(default_time: &str) -> Result<Recurrence> {
    let mode = Select::new(
        "Repeat",
        vec![
            "Never",
            "Every interval",
            "Every day at a time",
            "Certain weekdays at a time",
        ],
    )
    .prompt()?;

    match mode {
        "Never" => Ok(Recurrence::None),
        "Every interval" => {
            let every = Text::new("Interval")
                .with_default("30 minutes")
                .with_help_message("Examples: 10 minutes, 2 hours, 3 days")
                .prompt()?;
            timeparse::parse_every(&every)
        }
        "Every day at a time" => {
            let at = Text::new("Time")
                .with_default(if default_time == "now" {
                    "09:00"
                } else {
                    default_time
                })
                .prompt()?;
            Ok(Recurrence::DailyAt {
                time: timeparse::parse_time(&at)?,
            })
        }
        "Certain weekdays at a time" => {
            let weekdays = Text::new("Weekdays")
                .with_default("mon,tue,wed,thu,fri")
                .prompt()?;
            let at = Text::new("Time")
                .with_default(if default_time == "now" {
                    "09:00"
                } else {
                    default_time
                })
                .prompt()?;
            timeparse::parse_weekdays(&weekdays, timeparse::parse_time(&at)?)
        }
        _ => Ok(Recurrence::None),
    }
}

fn kind_prefix(recipient: &Recipient) -> &'static str {
    match recipient.kind {
        crate::model::RecipientKind::Contact => "👤",
        crate::model::RecipientKind::Group => "👥",
    }
}

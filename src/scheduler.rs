use crate::store::Store;
use crate::whatsapp::WhatsApp;
use anyhow::Result;
use chrono::Local;
use colored::Colorize;
use std::time::Duration;

pub async fn run_loop(store: &Store, wa: &WhatsApp) -> Result<()> {
    loop {
        send_due_once(store, wa).await?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("{} Scheduler stopped cleanly.", "👋".cyan());
                break;
            }
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
        }
    }
    Ok(())
}

pub async fn send_due_once(store: &Store, wa: &WhatsApp) -> Result<()> {
    loop {
        let due = store.due_deliveries(Local::now(), 10)?;
        if due.is_empty() {
            break;
        }

        for (job, delivery) in due {
            store.mark_sending(delivery.id)?;
            println!(
                "{} Sending job #{} to {}...",
                "💬".cyan(),
                delivery.job_id,
                delivery.recipient.name.bold()
            );

            match wa
                .send(&delivery.recipient, &job.message, job.file_path.as_deref())
                .await
            {
                Ok(message_id) => {
                    store.mark_sent(delivery.id, &message_id)?;
                    store.complete_or_advance_job(delivery.job_id)?;
                    println!(
                        "{} Delivered to {} ({message_id}).",
                        "✅".green(),
                        delivery.recipient.name
                    );
                }
                Err(err) => {
                    let attempts = delivery.attempts + 1;
                    store.mark_retry(delivery.id, attempts, &err.to_string())?;
                    println!(
                        "{} Delivery failed for {}. Retry #{attempts} queued: {}",
                        "⚠️".yellow(),
                        delivery.recipient.name,
                        err
                    );
                }
            }
        }
    }
    Ok(())
}

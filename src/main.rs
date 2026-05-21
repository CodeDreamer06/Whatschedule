mod app;
mod cli;
mod interactive;
mod model;
mod scheduler;
mod store;
mod timeparse;
mod whatsapp;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};
use colored::Colorize;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let cli = Cli::parse();
    let ctx = app::AppContext::new(cli.data_dir.clone())?;
    let store = store::Store::open(&ctx.scheduler_db)?;

    match cli.command.unwrap_or(Command::Interactive) {
        Command::Interactive => {
            println!("{}", "📱 Connecting to WhatsApp...".bold());
            let wa =
                whatsapp::WhatsApp::connect(&ctx.whatsapp_db, cli.phone, cli.pair_code).await?;
            interactive::run(&store, &wa).await?;
            scheduler::send_due_once(&store, &wa).await?;
        }
        Command::Run => {
            println!(
                "{}",
                "🕒 Scheduler is running. Press Ctrl-C to stop.".bold()
            );
            let wa =
                whatsapp::WhatsApp::connect(&ctx.whatsapp_db, cli.phone, cli.pair_code).await?;
            scheduler::run_loop(&store, &wa).await?;
        }
        Command::SendDue => {
            println!("{}", "🕒 Sending due messages once...".bold());
            let wa =
                whatsapp::WhatsApp::connect(&ctx.whatsapp_db, cli.phone, cli.pair_code).await?;
            scheduler::send_due_once(&store, &wa).await?;
        }
        Command::Schedule(args) => {
            let job = args.into_job()?;
            let id = store.insert_job(&job)?;
            println!("{} Job #{id} scheduled.", "✅".green());
        }
        Command::List => {
            let jobs = store.list_jobs()?;
            if jobs.is_empty() {
                println!("{} No scheduled messages yet.", "🗓️".cyan());
            } else {
                for job in jobs {
                    println!(
                        "{} #{} [{}] {} -> {} recipient(s){}",
                        "💬".cyan(),
                        job.id.unwrap_or_default(),
                        job.status,
                        job.run_at.format("%Y-%m-%d %H:%M:%S"),
                        job.recipients.len(),
                        job.file_path
                            .as_ref()
                            .map(|p| format!(" 📎 {}", p.display()))
                            .unwrap_or_default()
                    );
                    println!("   {}", job.message.replace('\n', " "));
                }
            }
        }
        Command::Cancel { id } => {
            store.cancel_job(id)?;
            println!("{} Job #{id} cancelled.", "✅".green());
        }
        Command::Contacts(command) => match command {
            cli::ContactsCommand::Add(args) => {
                store.upsert_contact(
                    &args.name,
                    &model::Recipient::from_input(&args.to, Some(args.name.clone()), args.group)?,
                )?;
                println!("{} Saved {}.", "✅".green(), args.name);
            }
            cli::ContactsCommand::List => {
                let contacts = store.list_contacts()?;
                if contacts.is_empty() {
                    println!("{} No saved contacts yet.", "📇".cyan());
                } else {
                    for contact in contacts {
                        println!("{} {} <{}>", "📇".cyan(), contact.name, contact.jid);
                    }
                }
            }
            cli::ContactsCommand::Remove { name } => {
                store.remove_contact(&name)?;
                println!("{} Removed {name}.", "✅".green());
            }
        },
    }

    Ok(())
}

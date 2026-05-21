use crate::model::{Recipient, RecipientKind};
use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use colored::Colorize;
use qrcode::QrCode;
use qrcode::render::unicode;
use ruwa::Jid;
use ruwa::bot::Bot;
use ruwa::pair_code::{PairCodeOptions, PlatformId};
use ruwa::store::SqliteStore;
use ruwa::types::events::Event;
use ruwa_tokio_transport::TokioWebSocketTransportFactory;
use ruwa_ureq_http_client::UreqHttpClient;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use wacore_ng::download::MediaType;
use waproto_ng::whatsapp as wa;

pub struct WhatsApp {
    client: Arc<ruwa::Client>,
    _handle: tokio::task::JoinHandle<()>,
    connected: watch::Receiver<bool>,
}

impl WhatsApp {
    pub async fn connect(
        session_db: &Path,
        phone: Option<String>,
        pair_code: Option<String>,
    ) -> Result<Self> {
        let backend = Arc::new(SqliteStore::new(&session_db.to_string_lossy()).await?);
        let transport_factory = TokioWebSocketTransportFactory::new();
        let http_client = UreqHttpClient::new();
        let (connected_tx, connected_rx) = watch::channel(false);

        let mut builder = Bot::builder()
            .with_backend(backend)
            .with_transport_factory(transport_factory)
            .with_http_client(http_client)
            .with_push_name("Whatschedule")
            .skip_history_sync();

        if let Some(phone) = phone {
            builder = builder.with_pair_code(PairCodeOptions {
                phone_number: phone,
                show_push_notification: true,
                custom_code: pair_code,
                platform_id: PlatformId::Chrome,
                platform_display: "Whatschedule Rust CLI".to_string(),
            });
        }

        let mut bot = builder
            .on_event(move |event, _client| {
                let connected_tx = connected_tx.clone();
                async move {
                    match event {
                        Event::PairingQrCode { code, timeout } => {
                            println!(
                                "{} Scan this QR in WhatsApp Linked Devices. Valid for {}s.",
                                "📱".cyan(),
                                timeout.as_secs()
                            );
                            print_qr(&code);
                        }
                        Event::PairingCode { code, timeout } => {
                            println!(
                                "{} Enter pairing code {} within {}s.",
                                "🔐".cyan(),
                                code.bold(),
                                timeout.as_secs()
                            );
                        }
                        Event::Connected(_) => {
                            let _ = connected_tx.send(true);
                            println!("{} WhatsApp connected.", "✅".green());
                        }
                        Event::LoggedOut(_) => {
                            let _ = connected_tx.send(false);
                            println!(
                                "{} WhatsApp logged out. Link again on next run.",
                                "⚠️".yellow()
                            );
                        }
                        Event::Disconnected(_) => {
                            let _ = connected_tx.send(false);
                            println!(
                                "{} WhatsApp disconnected; scheduler will retry.",
                                "⚠️".yellow()
                            );
                        }
                        _ => {}
                    }
                }
            })
            .build()
            .await?;

        let client = bot.client();
        let handle = bot.run().await?;
        let wa = Self {
            client,
            _handle: handle,
            connected: connected_rx,
        };
        wa.wait_ready(Duration::from_secs(120)).await?;
        Ok(wa)
    }

    pub async fn wait_ready(&self, timeout: Duration) -> Result<()> {
        if *self.connected.borrow() || self.client.is_logged_in() {
            return Ok(());
        }

        let mut rx = self.connected.clone();
        tokio::time::timeout(timeout, async move {
            loop {
                rx.changed().await.context("connection watcher closed")?;
                if *rx.borrow() {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        })
        .await
        .context("timed out waiting for WhatsApp connection")??;
        Ok(())
    }

    pub async fn groups(&self) -> Result<Vec<Recipient>> {
        let groups = self.client.groups().get_participating().await?;
        let mut recipients = groups
            .into_values()
            .map(|group| Recipient {
                jid: group.id.to_string(),
                name: group.subject,
                kind: RecipientKind::Group,
            })
            .collect::<Vec<_>>();
        recipients.sort_by_key(|r| r.name.to_ascii_lowercase());
        Ok(recipients)
    }

    pub async fn send(
        &self,
        recipient: &Recipient,
        message: &str,
        file_path: Option<&Path>,
    ) -> Result<String> {
        self.wait_ready(Duration::from_secs(60)).await?;
        let jid: Jid = recipient
            .jid
            .parse()
            .with_context(|| format!("invalid recipient JID {}", recipient.jid))?;

        let delay_ms = simulated_delay_ms(message, file_path);
        let _ = self.client.chatstate().send_composing(&jid).await;
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        let _ = self.client.chatstate().send_paused(&jid).await;

        let wa_message = match file_path {
            Some(path) => self.media_message(path, message).await?,
            None => wa::Message {
                conversation: Some(message.to_string()),
                ..Default::default()
            },
        };

        self.client.send_message(jid, wa_message).await
    }

    async fn media_message(&self, path: &Path, caption: &str) -> Result<wa::Message> {
        let data = tokio::fs::read(path)
            .await
            .with_context(|| format!("could not read {}", path.display()))?;
        let media_type = detect_media_type(path)?;
        let upload = self.client.upload(data, media_type).await?;
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .essence_str()
            .to_string();
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("attachment")
            .to_string();

        Ok(match media_type {
            MediaType::Image => wa::Message {
                image_message: Some(Box::new(wa::message::ImageMessage {
                    url: Some(upload.url),
                    direct_path: Some(upload.direct_path),
                    media_key: Some(upload.media_key),
                    file_sha256: Some(upload.file_sha256),
                    file_enc_sha256: Some(upload.file_enc_sha256),
                    file_length: Some(upload.file_length),
                    mimetype: Some(mime),
                    caption: non_empty(caption),
                    ..Default::default()
                })),
                ..Default::default()
            },
            MediaType::Video => wa::Message {
                video_message: Some(Box::new(wa::message::VideoMessage {
                    url: Some(upload.url),
                    direct_path: Some(upload.direct_path),
                    media_key: Some(upload.media_key),
                    file_sha256: Some(upload.file_sha256),
                    file_enc_sha256: Some(upload.file_enc_sha256),
                    file_length: Some(upload.file_length),
                    mimetype: Some(mime),
                    caption: non_empty(caption),
                    ..Default::default()
                })),
                ..Default::default()
            },
            MediaType::Audio => wa::Message {
                audio_message: Some(Box::new(wa::message::AudioMessage {
                    url: Some(upload.url),
                    direct_path: Some(upload.direct_path),
                    media_key: Some(upload.media_key),
                    file_sha256: Some(upload.file_sha256),
                    file_enc_sha256: Some(upload.file_enc_sha256),
                    file_length: Some(upload.file_length),
                    mimetype: Some(mime),
                    ptt: Some(false),
                    ..Default::default()
                })),
                ..Default::default()
            },
            MediaType::Document => wa::Message {
                document_message: Some(Box::new(wa::message::DocumentMessage {
                    url: Some(upload.url),
                    direct_path: Some(upload.direct_path),
                    media_key: Some(upload.media_key),
                    file_sha256: Some(upload.file_sha256),
                    file_enc_sha256: Some(upload.file_enc_sha256),
                    file_length: Some(upload.file_length),
                    mimetype: Some(mime),
                    file_name: Some(file_name),
                    caption: non_empty(caption),
                    ..Default::default()
                })),
                ..Default::default()
            },
            other => bail!("unsupported media type: {other:?}"),
        })
    }
}

fn print_qr(code: &str) {
    match QrCode::new(code.as_bytes()) {
        Ok(qr) => {
            let image = qr.render::<unicode::Dense1x2>().quiet_zone(true).build();
            println!("{image}");
        }
        Err(_) => println!("{code}"),
    }
}

fn detect_media_type(path: &Path) -> Result<MediaType> {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let top = mime.type_().as_str();
    match top {
        "image" => Ok(MediaType::Image),
        "video" => Ok(MediaType::Video),
        "audio" => Ok(MediaType::Audio),
        "application" | "text" => Ok(MediaType::Document),
        other => Err(anyhow!(
            "unsupported file MIME family '{other}' for {}",
            path.display()
        )),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn simulated_delay_ms(message: &str, file_path: Option<&Path>) -> u64 {
    let chars = message.chars().count() as u64;
    let base = if file_path.is_some() { 1800 } else { 900 };
    let reading = (chars * 35).min(4500);
    let jitter = Local::now().timestamp_subsec_millis() as u64 % 750;
    base + reading + jitter
}

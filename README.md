# Whatschedule

<p align="center">
  <strong>A resilient Rust CLI for scheduling WhatsApp messages, groups, and media.</strong>
</p>

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-1.95%2B-f46623?style=for-the-badge&logo=rust&logoColor=white">
  <img alt="WhatsApp Web" src="https://img.shields.io/badge/WhatsApp-Web-25D366?style=for-the-badge&logo=whatsapp&logoColor=white">
  <img alt="SQLite" src="https://img.shields.io/badge/SQLite-Durable-044a64?style=for-the-badge&logo=sqlite&logoColor=white">
</p>

Whatschedule is a terminal-first scheduler for WhatsApp Web. It lets you pick saved contacts or groups, write one or many messages, attach local media, choose natural dates like `today` and `tomorrow`, add recurring rules, then close the CLI. The next launch catches up safely: overdue messages are retried, sent messages are not duplicated, and recurring jobs advance only after delivery.

> WhatsApp Web is not a public stable API. This project uses [RuWa](https://crates.io/crates/ruwa), a Rust WhatsApp Web client, and should be treated like any unofficial WhatsApp automation: keep volume human, avoid spam, and expect upstream protocol changes.

## Highlights

- Interactive TUI prompts for contacts, groups, message body, schedule date/time, recurrence, and media path.
- Persistent WhatsApp session in `~/.local/share/whatschedule/whatsapp-session.db`.
- Crash-safe schedule database in `~/.local/share/whatschedule/scheduler.db`.
- Catch-up mode for missed deliveries after laptop sleep, shutdown, or app crashes.
- Recurrence support:
  - every `10 minutes`, `2 hours`, `3 days`
  - daily at `09:30`
  - selected weekdays at `18:00`
- Multiple recipients per job.
- Multiple messages in one interactive session.
- Typing indicator and human-like delay before each send.
- Media scheduling via `--file ./invoice.pdf` for images, PDFs/documents, audio, and video.
- Offline-aware retry with exponential backoff.
- macOS-style terminal responses: `✅`, `📱`, `🕒`, `💬`, `📎`, `🔁`, `⚠️`.

## Install

```bash
brew install protobuf
cargo build --release
```

The binary will be at:

```bash
./target/release/whatschedule
```

## First Login

Run the scheduler. Scan the QR code with WhatsApp:

```bash
whatschedule run
```

Or request a phone pairing code:

```bash
whatschedule run --phone +15551234567
```

Session data is persisted, so later launches reuse the same linked device.

## Interactive Scheduling

```bash
whatschedule interactive
```

The CLI will:

1. Connect to WhatsApp.
2. Load saved contacts and WhatsApp groups.
3. Let you select one or more recipients.
4. Ask for message text and optional file.
5. Ask for date/time and optional recurrence.
6. Save the job and immediately send anything due.

Add contacts once, then they appear in the picker:

```bash
whatschedule contacts add "Anika" +919876543210
whatschedule contacts add "Design Group" 120363000000000000@g.us --group
```

## Non-Interactive Usage

Schedule a text:

```bash
whatschedule schedule \
  --to +919876543210 \
  --message "Standup in 10?" \
  --date today \
  --time 09:20
```

Schedule a PDF:

```bash
whatschedule schedule \
  --to +919876543210 \
  --message "Invoice attached" \
  --date tomorrow \
  --time 11:00 \
  --file ./invoice.pdf
```

Every 30 minutes:

```bash
whatschedule schedule \
  --to +919876543210 \
  --message "Hydrate" \
  --date today \
  --time now \
  --every "30 minutes"
```

Weekdays at 18:00:

```bash
whatschedule schedule \
  --to +919876543210 \
  --message "Leaving office" \
  --date today \
  --time 18:00 \
  --weekdays mon,tue,wed,thu,fri
```

Run due jobs and keep watching:

```bash
whatschedule run
```

Send overdue jobs once, then exit:

```bash
whatschedule send-due
```

List or cancel jobs:

```bash
whatschedule list
whatschedule cancel 7
```

## Date And Recurrence Syntax

Dates:

- `today`
- `tomorrow`
- `yesterday` (useful for immediate catch-up testing)
- `2026-05-21`

Times:

- `now`
- `09:30`
- `18:05`

Intervals:

- `10 minutes`
- `2 hours`
- `3 days`

Weekdays:

- `mon,tue,wed,thu,fri`
- `saturday,sunday`

## Data Layout

By default:

```text
~/Library/Application Support/whatschedule/
├── scheduler.db
└── whatsapp-session.db
```

Override with:

```bash
whatschedule --data-dir ./local-state run
```

## Safety Model

Whatschedule records each recipient delivery separately. A delivery is only marked sent after RuWa returns a WhatsApp message ID. If the process crashes mid-send, that recipient is retried on the next launch. Recurring jobs only advance after all recipients for the current occurrence are sent.

Retries use exponential backoff with jitter and keep the original schedule. If your machine is offline or WhatsApp disconnects, jobs remain pending and are retried when `run` is active again.

## Project Status

This is an implementation-oriented starter. The scheduler, persistence, interactive flow, recurrence engine, typing delay, and media-message construction are local and testable. Real delivery depends on RuWa and the current WhatsApp Web protocol.

## Responsible Use

Use this for personal scheduling and small trusted groups. Do not use it for spam, scraping, bulk unsolicited messaging, or bypassing WhatsApp policies.

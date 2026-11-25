//! userland/desktop/apps/nova-mail/src/main.rs
//!
//! # NovaOS Mail Application CLI Runner (nova-mail / Hermes)
//!
//! Sovereign email and messaging CLI, inbox reader, account manager,
//! and autonomous qualification test runner.

use std::env;
use std::process::exit;

use nova_mail::{
    run_nova_mail_selftest, MailCategory, MailFolder, MailboxDatabase,
};

fn print_usage() {
    println!("NovaOS Sovereign Mail Studio (nova-mail / Hermes) v1.0.0");
    println!("Synthesizing macOS Mail Acrylic Shell & Mozilla Thunderbird Protocol Engine");
    println!();
    println!("Usage:");
    println!("  nova-mail                        Display mailbox telemetry, status & summary");
    println!("  nova-mail --selftest             Execute autonomous qualification test suite");
    println!("  nova-mail --inbox                List incoming emails with read/star/file badges");
    println!("  nova-mail --accounts             List configured email accounts and sync state");
    println!("  nova-mail --categories           Summarize Sequoia category breakdown (Primary, etc.)");
    println!("  nova-mail --search <QUERY>       Instant search across sender, subject, body");
    println!("  nova-mail --info                 Print email engine architecture subsystems");
    println!("  nova-mail --version              Print version information");
    println!("  nova-mail --help                 Display this reference manual");
    println!();
    println!("Subsystems:");
    println!("  1. Protocols: RFC 5322 / RFC 2822 / MIME Multipart / RFC 2047 standard streams");
    println!("  2. Categories: macOS Sequoia Smart Tabs (Primary, Transactions, Updates, Promotions)");
    println!("  3. Accounts: Multi-account routing (iCloud, Google, Exchange, Yahoo, IMAP/SMTP)");
    println!("  4. Layout: 3-pane sovereign layout (Translucent Sidebar, Message List, Reading Pane)");
    println!("  5. Security: DKIM/SPF cryptographic verification shields, heuristic spam filter");
    println!("  6. Graphics: Pure 2D software double-buffered vector rasterizer (zero-syscall)");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let db = MailboxDatabase::default();

    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                print_usage();
                exit(0);
            }
            "--selftest" | "-t" => {
                println!("[*] Launching NovaOS Mail Autonomous Qualification Suite...");
                let ok = run_nova_mail_selftest();
                if ok {
                    println!("[PASS] NovaOS Mail (nova-mail / Hermes) passed all qualification checks.");
                    exit(0);
                } else {
                    eprintln!("[FAIL] One or more nova-mail tests failed.");
                    exit(1);
                }
            }
            "--inbox" => {
                println!("NovaOS Hermes Inbox Messages:");
                let msgs = db.get_messages(&MailFolder::Inbox, None, "", false, false, false);
                for m in msgs {
                    let read = if m.is_read { " " } else { "●" };
                    let star = if m.is_starred { "★" } else { " " };
                    let att = if m.has_attachments() { "📎" } else { "  " };
                    println!("  {} {} {} [{}] {:<22} | {:<32} ({})", read, star, att, m.category.name(), m.from.name, m.subject, m.date_rfc2822);
                }
                exit(0);
            }
            "--accounts" => {
                println!("NovaOS Configured Email Accounts ({} total):", db.accounts.len());
                for a in &db.accounts {
                    println!("  * [{}] {} ({}) - {}", a.provider.icon(), a.name, a.email, a.provider.label());
                    println!("    IMAP: {}:{} | SMTP: {}:{}", a.imap_server, a.imap_port, a.smtp_server, a.smtp_port);
                }
                exit(0);
            }
            "--categories" => {
                println!("macOS Sequoia Category Breakdown (Inbox):");
                for cat in [MailCategory::Primary, MailCategory::Transactions, MailCategory::Updates, MailCategory::Promotions] {
                    let count = db.get_messages(&MailFolder::Inbox, Some(cat), "", false, false, false).len();
                    let unread = db.unread_count(&MailFolder::Inbox, Some(cat));
                    println!("  {} {:<14} : {} messages ({} unread)", cat.icon(), cat.name(), count, unread);
                }
                exit(0);
            }
            "--search" | "-s" => {
                if let Some(query) = args.get(2) {
                    println!("Search results for '{}':", query);
                    let res = db.get_messages(&MailFolder::Inbox, None, query, false, false, false);
                    if res.is_empty() {
                        println!("  No matching emails found.");
                    } else {
                        for m in res {
                            println!("  * [{}] {} - {}", m.from.name, m.subject, m.snippet);
                        }
                    }
                    exit(0);
                } else {
                    eprintln!("Error: --search requires a query string");
                    exit(1);
                }
            }
            "--info" | "-i" => {
                println!("NovaOS Hermes Mail Subsystems:");
                println!("  * Protocols:   RFC 5322 / RFC 2822 / MIME Multipart / RFC 2047");
                println!("  * Layout:      3-Pane Layout (Sidebar, Message List, Reading Pane)");
                println!("  * Categories:  macOS Sequoia Smart Tabs (Primary, Transactions, Updates, Promotions)");
                println!("  * Onboarding:  macOS Add Account Wizard (iCloud, Exchange, Google, IMAP/SMTP)");
                println!("  * Security:    DKIM/SPF verification badges, heuristic spam/phishing filters");
                println!("  * Engine:      Pure 2D software double-buffered vector rasterizer (zero-syscall)");
                exit(0);
            }
            "--version" | "-v" => {
                println!("NovaOS Mail (Hermes) 1.0.0");
                println!("Sovereign Pure-Rust Email & Communication Studio");
                exit(0);
            }
            _ => {}
        }
    }

    println!("============================================================");
    println!("        NOVAOS MAIL (HERMES) - SOVEREIGN EMAIL STUDIO       ");
    println!("============================================================");
    println!("Status:      Active (Pure-Rust Cross-Platform & NovaOS Ring-3)");
    println!("Accounts:    firoze.uhl@gmail.com (Google), developer@novaos.org (IMAP)");
    println!("Inbox:       {} total messages, {} unread", db.messages.len(), db.unread_count(&MailFolder::Inbox, None));
    println!("Categories:  Primary, Transactions, Updates, Promotions");
    println!("Commands:    --selftest, --inbox, --accounts, --categories, --search, --info");
    println!("Run with --selftest to execute the autonomous qualification suite.");
}

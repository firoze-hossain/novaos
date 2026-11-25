//! userland/desktop/apps/nova-mail/src/lib.rs
//!
//! # NovaOS Mail Application Library (nova-mail / Hermes)
//!
//! Sovereign, pure-Rust default Email & Communication Studio for NovaOS,
//! synthesizing macOS Mail (acrylic 3-pane layout, Sequoia category tabs,
//! onboarding Add Account sheets) with Mozilla Thunderbird (quick filters,
//! multi-account flexibility, and RFC 5322 MIME engine).

pub use hermes_core::{
    Address, Attachment, EmailMessage, MailCategory, MailFolder, Priority, Rfc5322Parser,
    SearchIndex, SpamFilter,
};
pub use hermes_render::{CanvasSurface, Color, Rect};
pub use hermes_storage::{AccountConfig, MailboxDatabase, ProviderType};
pub use hermes_ui::{HermesState, HermesViewMode, MailUiRenderer, UiHitTarget};

/// Autonomous qualification validation suite for nova-mail.
pub fn run_nova_mail_selftest() -> bool {
    // 1. RFC 5322 serialization and parsing roundtrip
    let test_msg = EmailMessage::new(
        "nova-selftest-001@novaos.org",
        "NovaOS Mail Engine Verification",
        Address::new("NovaOS Architecture", "arch@novaos.org"),
        vec![Address::new("Firoze Hossain", "firoze.uhl@gmail.com")],
        "Verifying sovereign email parsing and zero-syscall graphics rendering.",
    );
    let serialized = Rfc5322Parser::serialize(&test_msg);
    let parsed = match Rfc5322Parser::parse(&serialized) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if parsed.id != test_msg.id || parsed.subject != test_msg.subject {
        return false;
    }

    // 2. Category classification
    let cat = Rfc5322Parser::classify_category(
        "Order #98231 Receipt and Invoice",
        "Total amount paid: $120.00",
        &Address::new("Store", "order@store.com"),
    );
    if cat != MailCategory::Transactions {
        return false;
    }

    // 3. Mailbox database operations
    let mut db = MailboxDatabase::default();
    if db.accounts.is_empty() || db.messages.is_empty() {
        return false;
    }
    let inbox_msgs = db.get_messages(&MailFolder::Inbox, None, "", false, false, false);
    if inbox_msgs.is_empty() {
        return false;
    }

    // 4. Read / Unread toggle and search
    let target_id = &inbox_msgs[0].id.clone();
    db.mark_read(target_id, true);
    if !db.get_message(target_id).unwrap().is_read {
        return false;
    }

    // 5. 2D Vector Canvas rendering
    let mut canvas = CanvasSurface::new(640, 480);
    let state = HermesState::default();
    MailUiRenderer::render(&mut canvas, &state, &db);
    let modified = canvas.buffer.iter().filter(|&&px| px != Color::WHITE.0).count();
    if modified < 1000 {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nova_mail_qualification() {
        assert!(run_nova_mail_selftest());
    }

    #[test]
    fn test_mailbox_query() {
        let db = MailboxDatabase::default();
        let primary = db.get_messages(&MailFolder::Inbox, Some(MailCategory::Primary), "", false, false, false);
        assert!(!primary.is_empty());
    }
}

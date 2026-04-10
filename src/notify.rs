//! Native desktop notifications via `notify-rust`.

use notify_rust::Notification;

/// Send a desktop notification. Fire-and-forget — errors are silently ignored.
pub fn send(title: &str, message: &str) {
    let _ = Notification::new().summary(title).body(message).show();
}

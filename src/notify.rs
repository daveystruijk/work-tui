//! Desktop notifications via `terminal-notifier` (macOS).

use tokio::process::Command;

/// Send a desktop notification. Fire-and-forget — errors are silently ignored.
pub fn send(title: &str, message: &str) {
    let title = title.to_string();
    let message = message.to_string();
    tokio::spawn(async move {
        let _ = Command::new("terminal-notifier")
            .args(["-title", &title, "-message", &message, "-sound", "default"])
            .output()
            .await;
    });
}

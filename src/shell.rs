use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Run a command to completion and return its trimmed stdout; its stderr becomes the error.
pub fn run(program: &str, args: &[&str], dir: Option<&Path>) -> Result<String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let output = command
        .output()
        .with_context(|| format!("failed to execute {program}"))?;
    if !output.status.success() {
        let shown = args.iter().take(3).copied().collect::<Vec<_>>().join(" ");
        bail!(
            "{program} {shown}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Quote a value for use as a single word in `sh -c`.
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

pub fn open_in_browser(url: &str) -> Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    run(opener, &[url], None).map(drop)
}

/// Ring the terminal bell. tmux marks the window's tab for it until the window is visited.
pub fn ring_bell() {
    use std::io::Write;
    let mut terminal = std::io::stdout();
    let _ = terminal.write_all(b"\x07").and_then(|()| terminal.flush());
}

//! Opening a URL in the user's browser.
//!
//! Decision: replaces the `webbrowser` crate, which pulled an AppKit binding
//! (`objc2-app-kit`) for a single call. Login already prints the URL and
//! degrades gracefully when this fails, so spawning the platform opener is the
//! whole requirement.

use std::io;
use std::process::{Command, Stdio};

/// Spawns the platform's URL handler. Returns an error if it could not be
/// launched; callers are expected to fall back to printing the URL.
pub fn open(url: &str) -> io::Result<()> {
    // Refuse anything that is not a plain http(s) URL: the argument reaches a
    // shell-less spawn, but a leading `-` would still be read as a flag by the
    // opener itself.
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "refusing to open a non-http(s) URL",
        ));
    }

    let (program, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(windows) {
        // `start` is a cmd builtin; the empty string is its window-title slot.
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    };

    let status = Command::new(program)
        .args(args)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{program} exited with status {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_urls() {
        for url in ["file:///etc/passwd", "javascript:alert(1)", "-nope"] {
            assert_eq!(
                open(url).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{url}"
            );
        }
    }
}

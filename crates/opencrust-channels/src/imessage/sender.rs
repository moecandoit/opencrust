use tracing::debug;

/// Escape a string for use inside an AppleScript double-quoted literal.
///
/// Handles backslashes, double quotes, and newlines.
pub fn applescript_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

/// Send an iMessage to `to` (phone number or email) via Messages.app.
///
/// Uses `osascript` to execute an AppleScript that drives the Messages application.
pub async fn send_imessage(to: &str, text: &str) -> Result<(), String> {
    let escaped_to = applescript_escape(to);
    let escaped_text = applescript_escape(text);

    let script = format!(
        r#"tell application "Messages"
    set targetService to 1st account whose service type = iMessage
    set targetBuddy to participant targetService handle "{escaped_to}"
    send "{escaped_text}" to targetBuddy
end tell"#
    );

    debug!("imessage: sending to {to} ({} chars)", text.len());

    let output = tokio::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .await
        .map_err(|e| format!("failed to spawn osascript: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("osascript exited with {}: {stderr}", output.status));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_escape_handles_quotes() {
        assert_eq!(applescript_escape(r#"say "hello""#), r#"say \"hello\""#);
    }

    #[test]
    fn applescript_escape_handles_backslashes() {
        assert_eq!(applescript_escape(r"path\to\file"), r"path\\to\\file");
    }

    #[test]
    fn applescript_escape_handles_newlines() {
        assert_eq!(applescript_escape("line1\nline2"), r"line1\nline2");
    }

    #[test]
    fn applescript_escape_handles_carriage_returns() {
        assert_eq!(applescript_escape("a\rb"), r"a\rb");
    }

    #[test]
    fn applescript_escape_combined() {
        let input = "He said \"hi\"\nand\\left";
        let expected = r#"He said \"hi\"\nand\\left"#;
        assert_eq!(applescript_escape(input), expected);
    }

    #[test]
    fn applescript_escape_empty_string() {
        assert_eq!(applescript_escape(""), "");
    }

    #[test]
    fn applescript_escape_no_special_chars() {
        assert_eq!(applescript_escape("hello world"), "hello world");
    }
}

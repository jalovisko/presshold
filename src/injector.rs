use crate::detector::Session;
use log::{debug, warn};
use std::process::Command;

/// Inject a Unicode character into the focused application.
///
/// Uses synthetic input events (xdotool / wtype) which bypass our evdev
/// grab and reach the compositor / X server directly.
pub fn inject(ch: char, session: &Session) -> bool {
    let s: String = ch.into();

    if *session == Session::Wayland {
        // Clipboard paste first: it is the only method that reliably delivers
        // an exact Unicode codepoint to every client. `ydotool type` maps
        // characters to keycodes from a fixed (US) table and silently sends
        // the *wrong* character for anything outside it (observed: 'á' typed
        // as ']', while still reporting success) — so it can't be trusted for
        // accents. `wtype` extends the XKB keymap correctly, but Chromium and
        // other Electron/Wayland clients silently ignore its virtual-keyboard
        // events even though the command itself reports success.
        if try_clipboard_paste(ch) {
            debug!("injected {ch:?} via clipboard paste");
            return true;
        }
        debug!("clipboard paste failed, trying wtype");
        if try_wtype(&s) {
            debug!("injected {ch:?} via wtype");
            return true;
        }
        debug!("wtype failed, trying ydotool");
        if try_ydotool(&s) {
            debug!("injected {ch:?} via ydotool");
            return true;
        }
        debug!("ydotool failed, falling back to xdotool");
    }

    // X11 or XWayland fallback
    if try_xdotool(&s) {
        return true;
    }

    warn!("All injection methods failed for {:?}", ch);
    false
}

/// Paste `ch` via the clipboard: save the current clipboard, set it to `ch`,
/// send Ctrl+V through ydotool (a plain key combo, not a Unicode codepoint —
/// this is reliable), then restore the previous clipboard contents.
fn try_clipboard_paste(ch: char) -> bool {
    let previous = Command::new("wl-paste")
        .arg("--no-newline")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout);

    let mut copy = match Command::new("wl-copy").stdin(std::process::Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            debug!("wl-copy not available: {e}");
            return false;
        }
    };
    {
        use std::io::Write;
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        if let Some(stdin) = copy.stdin.as_mut() {
            if stdin.write_all(s.as_bytes()).is_err() {
                debug!("wl-copy: failed to write to stdin");
                return false;
            }
        }
    }
    if !copy.wait().map(|s| s.success()).unwrap_or(false) {
        debug!("wl-copy exited unsuccessfully");
        return false;
    }

    // KEY_LEFTCTRL = 29, KEY_V = 47. `ydotool key` combos are plain keycodes,
    // not the Unicode-mapping path that fails for accented characters.
    let pasted = Command::new("ydotool")
        .args(["key", "29:1", "47:1", "47:0", "29:0"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if let Some(prev) = previous {
        // Give the paste a moment to land before restoring the clipboard.
        std::thread::sleep(std::time::Duration::from_millis(150));
        if let Ok(mut restore) =
            Command::new("wl-copy").stdin(std::process::Stdio::piped()).spawn()
        {
            use std::io::Write;
            if let Some(stdin) = restore.stdin.as_mut() {
                let _ = stdin.write_all(&prev);
            }
            let _ = restore.wait();
        }
    }

    pasted
}

fn try_wtype(s: &str) -> bool {
    match Command::new("wtype").arg(s).status() {
        Ok(st) if st.success() => true,
        Ok(st) => {
            debug!("wtype exited with {st}");
            false
        }
        Err(e) => {
            debug!("wtype not available: {e}");
            false
        }
    }
}

fn try_ydotool(s: &str) -> bool {
    // ydotool uses uinput, works on GNOME Wayland and other compositors that
    // don't support zwp_virtual_keyboard_v1 (required by wtype).
    // Requires the ydotoold daemon to be running.
    match Command::new("ydotool").args(["type", "--", s]).output() {
        Ok(out) if out.status.success() => {
            // ydotool exits 0 even when ydotoold isn't running, printing
            // "ydotoold backend unavailable" to stderr. Treat that as failure.
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("unavailable") {
                debug!("ydotool: ydotoold not running");
                return false;
            }
            true
        }
        Ok(out) => {
            debug!("ydotool exited with {}", out.status);
            false
        }
        Err(e) => {
            debug!("ydotool not available: {e}");
            false
        }
    }
}

fn try_xdotool(s: &str) -> bool {
    match Command::new("xdotool")
        .args(["type", "--clearmodifiers", "--delay", "0", "--", s])
        .status()
    {
        Ok(st) if st.success() => true,
        Ok(st) => {
            debug!("xdotool exited with {st}");
            false
        }
        Err(e) => {
            debug!("xdotool not available: {e}");
            false
        }
    }
}

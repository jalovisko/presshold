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
        if try_wtype(&s) {
            return true;
        }
        debug!("wtype failed, trying ydotool");
        if try_ydotool(&s) {
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
    match Command::new("ydotool").args(["type", "--", s]).status() {
        Ok(st) if st.success() => true,
        Ok(st) => {
            debug!("ydotool exited with {st}");
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

use std::env;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Session {
    X11,
    Wayland,
    Unknown,
}

/// Detect the display server protocol in use.
pub fn session() -> Session {
    if env::var_os("WAYLAND_DISPLAY").is_some() {
        return Session::Wayland;
    }
    if env::var_os("DISPLAY").is_some() {
        return Session::X11;
    }
    match env::var("XDG_SESSION_TYPE").as_deref() {
        Ok("wayland") => Session::Wayland,
        Ok("x11") => Session::X11,
        _ => Session::Unknown,
    }
}

/// Detect the desktop environment / window manager name (lower-case).
/// Returns e.g. "hyprland", "sway", "i3", "plasma", "gnome", "xfce", "unknown".
pub fn desktop() -> String {
    for var in &["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"] {
        if let Ok(val) = env::var(var) {
            let lower = val.to_lowercase();
            if let Some(name) = match_de(&lower) {
                return name.to_string();
            }
        }
    }

    // Fall back to checking running process names
    if let Ok(out) = Command::new("ps").args(["-e", "-o", "comm="]).output() {
        let procs = String::from_utf8_lossy(&out.stdout).to_lowercase();
        if let Some(name) = match_de(&procs) {
            return name.to_string();
        }
    }

    "unknown".to_string()
}

fn match_de(hay: &str) -> Option<&'static str> {
    const DES: &[&str] = &[
        "hyprland", "sway", "i3", "plasma", "kde",
        "gnome", "xfce", "lxde", "lxqt", "mate",
        "cinnamon", "openbox", "awesome", "bspwm", "dwm",
        "herbstluftwm", "qtile", "xmonad", "river",
    ];
    DES.iter().copied().find(|de| hay.contains(de))
}

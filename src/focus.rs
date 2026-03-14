/// Focus tracker — knows whether a text-input context is active.
///
/// Strategy (tried in order):
///   1. Hyprland IPC socket  — native, zero extra deps.
///      flag=true  when a window is focused and NOT fullscreen.
///      flag=false when the desktop/wallpaper has focus, or a fullscreen
///                 window is active (games almost always run fullscreen).
///   2. AT-SPI (D-Bus)       — for GNOME/KDE and other a11y-enabled desktops.
///   3. Fallback: flag=true  — popup allowed everywhere (pre-AT-SPI behaviour).
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use log::{debug, warn};

pub fn spawn() -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    let flag2 = Arc::clone(&flag);

    std::thread::Builder::new()
        .name("focus-tracker".into())
        .spawn(move || {
            if let Ok(sig) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
                match hyprland_loop(&sig, Arc::clone(&flag2)) {
                    Ok(()) => return,
                    Err(e) => warn!("Hyprland focus tracker failed: {e}"),
                }
            }

            // Non-Hyprland: try AT-SPI.
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    warn!("AT-SPI: could not start runtime: {e}");
                    flag2.store(true, Ordering::Relaxed);
                    return;
                }
            };
            rt.block_on(atspi_run(Arc::clone(&flag2)));
        })
        .expect("failed to spawn focus-tracker thread");

    flag
}

// ─────────────────────────────────────────────────────────────────────────────
// Hyprland IPC
// ─────────────────────────────────────────────────────────────────────────────

fn hyprland_loop(sig: &str, flag: Arc<AtomicBool>) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    // Hyprland puts its sockets in $XDG_RUNTIME_DIR/hypr/<sig>/ (newer) or
    // /tmp/hypr/<sig>/ (older).
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let path = format!("{base}/hypr/{sig}/.socket2.sock");
    let stream = UnixStream::connect(&path)
        .or_else(|_| UnixStream::connect(format!("/tmp/hypr/{sig}/.socket2.sock")))?;

    let mut has_window = false;
    let mut fullscreen = false;

    // Seed from the current active window before listening to events.
    if let Ok(out) = std::process::Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout);
        has_window = s.contains("\"class\":")
            && !s.contains("\"class\": \"\"")
            && !s.trim_start().starts_with("Invalid");
        fullscreen = s.contains("\"fullscreen\": true");
    }
    update(&flag, has_window, fullscreen);

    for line in BufReader::new(stream).lines() {
        let line = line?;
        if let Some(data) = line.strip_prefix("activewindow>>") {
            let class = data.split(',').next().unwrap_or("").trim();
            has_window = !class.is_empty();
            debug!("Hyprland: activewindow class={class:?}");
        } else if let Some(data) = line.strip_prefix("fullscreen>>") {
            fullscreen = data.trim() == "1";
            debug!("Hyprland: fullscreen={fullscreen}");
        } else {
            continue;
        }
        update(&flag, has_window, fullscreen);
    }

    Ok(())
}

fn update(flag: &AtomicBool, has_window: bool, fullscreen: bool) {
    let allow = has_window && !fullscreen;
    debug!("focus: allow={allow} (window={has_window} fullscreen={fullscreen})");
    flag.store(allow, Ordering::Relaxed);
}

// ─────────────────────────────────────────────────────────────────────────────
// AT-SPI (GNOME / KDE fallback)
// ─────────────────────────────────────────────────────────────────────────────

const TEXT_ROLES: &[u32] = &[11, 12, 40, 52, 60, 61, 77, 79, 82, 94, 95];

async fn atspi_run(flag: Arc<AtomicBool>) {
    match atspi_listen(Arc::clone(&flag)).await {
        Ok(()) => {}
        Err(e) => {
            warn!("AT-SPI unavailable, popup will show everywhere: {e}");
            flag.store(true, Ordering::Relaxed);
        }
    }
}

async fn atspi_listen(flag: Arc<AtomicBool>) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};
    use zbus::Connection;

    let session = Connection::session().await?;
    let atspi_address: String = session
        .call_method(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            Some("org.a11y.Bus"),
            "GetAddress",
            &(),
        )
        .await?
        .body()
        .deserialize()?;

    let atspi = zbus::connection::Builder::address(&*atspi_address)?
        .build()
        .await?;

    atspi
        .call_method(
            Some("org.a11y.atspi.Registry"),
            "/org/a11y/atspi/registry",
            Some("org.a11y.atspi.Registry"),
            "RegisterEvent",
            &"object:state-changed:focused",
        )
        .await?;

    atspi
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "AddMatch",
            &"type='signal',interface='org.a11y.atspi.Event.Object',member='StateChanged'",
        )
        .await?;

    debug!("AT-SPI: subscribed to focus events");
    let mut stream = zbus::MessageStream::from(&atspi);

    while let Some(msg) = stream.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => { debug!("AT-SPI stream error: {e}"); continue; }
        };

        let hdr = msg.header();
        let iface = match hdr.interface() { Some(i) => i, None => continue };
        if iface.as_str() != "org.a11y.atspi.Event.Object" { continue; }
        let member = match hdr.member() { Some(m) => m, None => continue };
        if member.as_str() != "StateChanged" { continue; }

        // Body: siiv(so)
        let Ok((kind, detail1, _, _, _)) =
            msg.body().deserialize::<(String, i32, i32, OwnedValue, (String, OwnedObjectPath))>()
        else { continue };

        if kind != "focused" { continue; }

        if detail1 == 1 {
            let bus_name = hdr.sender().map(|s| s.as_str().to_owned()).unwrap_or_default();
            let obj_path = hdr.path().map(|p| p.as_str().to_owned()).unwrap_or_default();
            let is_text = atspi_role_is_text(&atspi, &bus_name, &obj_path)
                .await
                .unwrap_or(false);
            debug!("AT-SPI: focus gained is_text={is_text}");
            flag.store(is_text, Ordering::Relaxed);
        } else {
            debug!("AT-SPI: focus lost");
            flag.store(false, Ordering::Relaxed);
        }
    }

    Ok(())
}

async fn atspi_role_is_text(conn: &zbus::Connection, bus: &str, path: &str) -> anyhow::Result<bool> {
    if bus.is_empty() || path.is_empty() { return Ok(false); }
    let role: u32 = conn
        .call_method(Some(bus), path, Some("org.a11y.atspi.Accessible"), "GetRole", &())
        .await?
        .body()
        .deserialize()?;
    debug!("AT-SPI: role id={role}");
    Ok(TEXT_ROLES.contains(&role))
}

use anyhow::{Context, Result};
use evdev::{AttributeSet, Device, InputEvent, Key};
use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use log::{info, warn, debug};
use std::sync::mpsc::SyncSender;
use std::thread;

// ── Discovery ────────────────────────────────────────────────────────────────

/// Return all physical keyboard devices (devices that have A-Z keys).
/// Virtual/uinput devices are excluded — they have no physical path.
pub fn find_keyboards() -> Vec<Device> {
    evdev::enumerate()
        .filter_map(|(path, dev)| {
            let has_letters = dev
                .supported_keys()
                .map_or(false, |k| k.contains(Key::KEY_A) && k.contains(Key::KEY_Z));
            if !has_letters {
                return None;
            }
            // Skip uinput virtual devices — they have an empty physical path.
            let phys = dev.physical_path().unwrap_or_default();
            if phys.is_empty() {
                debug!("Skipping virtual device: {} ({})", dev.name().unwrap_or("?"), path.display());
                return None;
            }
            info!("Found keyboard: {} ({})", dev.name().unwrap_or("?"), path.display());
            Some(dev)
        })
        .collect()
}

// ── Virtual passthrough device ────────────────────────────────────────────────

/// Build a uinput virtual keyboard whose key set is the union of all grabbed
/// keyboards.  Events we want to pass through are re-emitted via this device.
pub fn create_passthrough(keyboards: &[Device]) -> Result<VirtualDevice> {
    let mut keys = AttributeSet::<Key>::new();
    for kb in keyboards {
        if let Some(supported) = kb.supported_keys() {
            for k in supported.iter() {
                keys.insert(k);
            }
        }
    }

    let vdev = VirtualDeviceBuilder::new()
        .context("failed to open /dev/uinput")?
        .name("presshold-passthrough")
        .with_keys(&keys)?
        .build()
        .context("failed to build virtual device")?;

    // Give the kernel/udev a moment to register the new device so that
    // applications see it before we start emitting events.
    std::thread::sleep(std::time::Duration::from_millis(100));

    info!("Created passthrough virtual device");
    Ok(vdev)
}

// ── Reader threads ────────────────────────────────────────────────────────────

/// Grab every keyboard and spawn one reader thread per device.
/// Each thread forwards raw `InputEvent`s to the GTK main thread via `tx`.
pub fn spawn_readers(mut devices: Vec<Device>, tx: SyncSender<InputEvent>) {
    for mut dev in devices.drain(..) {
        let name = dev.name().unwrap_or("?").to_string();
        if let Err(e) = dev.grab() {
            warn!("Could not grab {name}: {e}. Skipping");
            continue;
        }
        info!("Grabbed {name}");

        let tx = tx.clone();
        thread::Builder::new()
            .name(format!("kbd-reader:{name}"))
            .spawn(move || reader_loop(dev, tx))
            .expect("failed to spawn reader thread");
    }
}

fn reader_loop(mut dev: Device, tx: SyncSender<InputEvent>) {
    let name = dev.name().unwrap_or("?").to_string();
    loop {
        match dev.fetch_events() {
            Ok(events) => {
                for ev in events {
                    if tx.send(ev).is_err() {
                        debug!("Channel closed, reader thread for {name} exiting");
                        return;
                    }
                }
            }
            Err(e) => {
                warn!("Read error on {name}: {e}");
                break;
            }
        }
    }
}

// ── Key → char mapping ────────────────────────────────────────────────────────

/// Translate a hardware key into the character it represents, considering the
/// current shift / caps-lock state.  Returns `None` for non-letter keys.
pub fn key_to_char(key: Key, shift: bool, caps: bool) -> Option<char> {
    let upper = shift ^ caps;
    let ch = match key {
        Key::KEY_A => 'a', Key::KEY_B => 'b', Key::KEY_C => 'c',
        Key::KEY_D => 'd', Key::KEY_E => 'e', Key::KEY_F => 'f',
        Key::KEY_G => 'g', Key::KEY_H => 'h', Key::KEY_I => 'i',
        Key::KEY_J => 'j', Key::KEY_K => 'k', Key::KEY_L => 'l',
        Key::KEY_M => 'm', Key::KEY_N => 'n', Key::KEY_O => 'o',
        Key::KEY_P => 'p', Key::KEY_Q => 'q', Key::KEY_R => 'r',
        Key::KEY_S => 's', Key::KEY_T => 't', Key::KEY_U => 'u',
        Key::KEY_V => 'v', Key::KEY_W => 'w', Key::KEY_X => 'x',
        Key::KEY_Y => 'y', Key::KEY_Z => 'z',
        // Punctuation keys that have accent-like variants (shift state ignored).
        Key::KEY_MINUS => return Some('-'),
        _ => return None,
    };
    Some(if upper {
        ch.to_ascii_uppercase()
    } else {
        ch
    })
}

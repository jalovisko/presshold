use anyhow::{Context, Result};
use evdev::{AttributeSet, Device, InputEvent, Key};
use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use log::{info, warn, debug};
use std::collections::HashSet;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Directory where the kernel exposes input device nodes.
const INPUT_DIR: &str = "/dev/input";

/// Paths of the keyboards currently grabbed by a reader thread.  Shared with
/// the hotplug watcher so a node is never grabbed twice.
pub type Grabbed = Arc<Mutex<HashSet<PathBuf>>>;

// ── Discovery ────────────────────────────────────────────────────────────────

/// A physical keyboard is a device that has A-Z keys and a physical path.
/// Virtual/uinput devices (including our own passthrough) have no physical
/// path and are excluded.
fn is_physical_keyboard(path: &Path, dev: &Device) -> bool {
    let has_letters = dev
        .supported_keys()
        .map_or(false, |k| k.contains(Key::KEY_A) && k.contains(Key::KEY_Z));
    if !has_letters {
        return false;
    }
    let phys = dev.physical_path().unwrap_or_default();
    if phys.is_empty() {
        debug!("Skipping virtual device: {} ({})", dev.name().unwrap_or("?"), path.display());
        return false;
    }
    info!("Found keyboard: {} ({})", dev.name().unwrap_or("?"), path.display());
    true
}

/// Return all physical keyboard devices present right now, with their paths.
pub fn find_keyboards() -> Vec<(PathBuf, Device)> {
    evdev::enumerate()
        .filter(|(path, dev)| is_physical_keyboard(path, dev))
        .collect()
}

/// `/dev/input/eventN` nodes are the only ones evdev can open.
fn is_event_node(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map_or(false, |n| n.starts_with("event"))
}

// ── Output ───────────────────────────────────────────────────────────────────

/// Where synthesised key events go. The real daemon uses the uinput
/// passthrough device; tests record the events instead.
pub trait KeyOut {
    fn emit(&mut self, events: &[InputEvent]) -> std::io::Result<()>;
}

impl KeyOut for VirtualDevice {
    fn emit(&mut self, events: &[InputEvent]) -> std::io::Result<()> {
        VirtualDevice::emit(self, events)
    }
}

// ── Virtual passthrough device ────────────────────────────────────────────────

/// Every keyboard key code: the classic range below the BTN_ block plus the
/// extended KEY_ range above it.  Buttons (BTN_*) are left out so libinput
/// keeps seeing the passthrough as a keyboard rather than a mouse or tablet.
/// Advertising the full set means a keyboard hotplugged later can be
/// forwarded without rebuilding the device.
pub fn passthrough_keys() -> AttributeSet<Key> {
    let mut keys = AttributeSet::<Key>::new();
    for code in (1..Key::BTN_0.0).chain(Key::KEY_OK.0..=0x2ff) {
        keys.insert(Key(code));
    }
    keys
}

/// Build the uinput virtual keyboard.  Events we want to pass through are
/// re-emitted via this device.
pub fn create_passthrough() -> Result<VirtualDevice> {
    let vdev = VirtualDeviceBuilder::new()
        .context("failed to open /dev/uinput")?
        .name("presshold-passthrough")
        .with_keys(&passthrough_keys())?
        .build()
        .context("failed to build virtual device")?;

    // Give the kernel/udev a moment to register the new device so that
    // applications see it before we start emitting events.
    thread::sleep(Duration::from_millis(100));

    info!("Created passthrough virtual device");
    Ok(vdev)
}

// ── Reader threads ────────────────────────────────────────────────────────────

/// Grab every keyboard and spawn one reader thread per device.
/// Each thread forwards raw `InputEvent`s to the GTK main thread via `tx`.
pub fn spawn_readers(devices: Vec<(PathBuf, Device)>, tx: &SyncSender<InputEvent>, grabbed: &Grabbed) {
    for (path, dev) in devices {
        grab_and_spawn(path, dev, tx, grabbed);
    }
}

fn grab_and_spawn(path: PathBuf, mut dev: Device, tx: &SyncSender<InputEvent>, grabbed: &Grabbed) {
    let name = dev.name().unwrap_or("?").to_string();
    if let Err(e) = dev.grab() {
        warn!("Could not grab {name}: {e}. Skipping");
        return;
    }
    info!("Grabbed {name}");
    grabbed.lock().unwrap().insert(path.clone());

    let tx = tx.clone();
    let grabbed = Arc::clone(grabbed);
    thread::Builder::new()
        .name(format!("kbd-reader:{name}"))
        .spawn(move || {
            reader_loop(dev, tx);
            // Unplugged or unreadable: forget it so a re-plug at the same
            // node is grabbed again.
            grabbed.lock().unwrap().remove(&path);
        })
        .expect("failed to spawn reader thread");
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

// ── Hotplug ──────────────────────────────────────────────────────────────────

/// Watch `/dev/input` and grab keyboards that appear after startup.
pub fn spawn_hotplug_watcher(tx: SyncSender<InputEvent>, grabbed: Grabbed) {
    thread::Builder::new()
        .name("kbd-hotplug".into())
        .spawn(move || {
            let r = watch_dir(Path::new(INPUT_DIR), |path| {
                on_new_node(path, &tx, &grabbed);
                true
            });
            if let Err(e) = r {
                warn!("Keyboard hotplug watcher stopped: {e}. Keyboards plugged in from now on will be ignored");
            }
        })
        .expect("failed to spawn hotplug watcher thread");
}

/// How long to keep retrying an open that fails because udev has not yet
/// applied the node's group/ACL.
const OPEN_RETRIES: u32 = 20;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(100);

fn on_new_node(path: PathBuf, tx: &SyncSender<InputEvent>, grabbed: &Grabbed) {
    if !is_event_node(&path) || grabbed.lock().unwrap().contains(&path) {
        return;
    }
    for attempt in 1..=OPEN_RETRIES {
        match Device::open(&path) {
            Ok(dev) => {
                if is_physical_keyboard(&path, &dev) {
                    grab_and_spawn(path, dev, tx, grabbed);
                }
                return;
            }
            // The node exists before udev chmods it: wait and try again.
            Err(e) if matches!(e.kind(), std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound) => {
                debug!("Open {} attempt {attempt} failed: {e}", path.display());
                thread::sleep(OPEN_RETRY_DELAY);
            }
            Err(e) => {
                warn!("Could not open {}: {e}", path.display());
                return;
            }
        }
    }
    warn!("Gave up opening {} after {OPEN_RETRIES} attempts", path.display());
}

/// Block, calling `on_create` with the full path of every entry created in
/// `dir`, until `on_create` returns `false` or the watch fails.
pub fn watch_dir(dir: &Path, mut on_create: impl FnMut(PathBuf) -> bool) -> Result<()> {
    let c_dir = CString::new(dir.as_os_str().as_encoded_bytes())
        .context("directory path contains a NUL byte")?;

    // SAFETY: plain libc calls with valid arguments; the fd is owned here and
    // closed on every exit path below.
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("inotify_init1");
    }
    let wd = unsafe { libc::inotify_add_watch(fd, c_dir.as_ptr(), libc::IN_CREATE) };
    if wd < 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err).context(format!("inotify_add_watch {}", dir.display()));
    }

    let mut buf = vec![0u8; 64 * 1024];
    let result = loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break Err(err).context("inotify read");
        }
        let mut keep_going = true;
        for name in parse_inotify_names(&buf[..n as usize]) {
            if !on_create(dir.join(name)) {
                keep_going = false;
                break;
            }
        }
        if !keep_going {
            break Ok(());
        }
    };
    unsafe { libc::close(fd) };
    result
}

/// Extract the entry names from a buffer of raw `inotify_event` records.
fn parse_inotify_names(buf: &[u8]) -> Vec<String> {
    const HEADER: usize = std::mem::size_of::<libc::inotify_event>();
    let mut names = Vec::new();
    let mut off = 0;
    while off + HEADER <= buf.len() {
        // SAFETY: the kernel writes whole, correctly aligned records; we have
        // checked that the header fits in the remaining buffer.
        let ev = unsafe { std::ptr::read_unaligned(buf[off..].as_ptr() as *const libc::inotify_event) };
        let len = ev.len as usize;
        let name_bytes = &buf[off + HEADER..(off + HEADER + len).min(buf.len())];
        let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
        if name_end > 0 {
            names.push(String::from_utf8_lossy(&name_bytes[..name_end]).into_owned());
        }
        off += HEADER + len;
    }
    names
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "presshold-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn passthrough_key_set_covers_keyboard_keys_but_not_buttons() {
        let keys = passthrough_keys();
        assert!(keys.contains(Key::KEY_A));
        assert!(keys.contains(Key::KEY_F24));
        assert!(keys.contains(Key::KEY_OK));            // 0x160, first key above the BTN_ range
        assert!(keys.contains(Key(0x2ff)));             // last key code
        assert!(!keys.contains(Key::BTN_LEFT));         // would make libinput see a mouse
        assert!(!keys.contains(Key::BTN_TOUCH));
        assert!(!keys.contains(Key(0)));                // KEY_RESERVED
    }

    #[test]
    fn only_event_nodes_are_candidates() {
        assert!(is_event_node(Path::new("/dev/input/event7")));
        assert!(!is_event_node(Path::new("/dev/input/mouse1")));
        assert!(!is_event_node(Path::new("/dev/input/by-id")));
    }

    #[test]
    fn watch_dir_reports_every_new_entry() {
        let dir = temp_dir("watch");
        let (tx, rx) = mpsc::channel::<PathBuf>();
        let watched = dir.clone();
        thread::spawn(move || {
            watch_dir(&watched, |p| tx.send(p).is_ok()).unwrap();
        });
        // Give the watcher a moment to register before creating entries.
        thread::sleep(Duration::from_millis(100));
        std::fs::write(dir.join("event42"), b"").unwrap();
        std::fs::write(dir.join("mouse1"), b"").unwrap();

        let first = rx.recv_timeout(Duration::from_secs(2)).expect("first create event");
        let second = rx.recv_timeout(Duration::from_secs(2)).expect("second create event");
        assert_eq!(first, dir.join("event42"));
        assert_eq!(second, dir.join("mouse1"));
        std::fs::remove_dir_all(&dir).ok();
    }
}

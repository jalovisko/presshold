mod accents;
mod detector;
mod injector;
mod keyboard;
mod popup;

use anyhow::{Context, Result};
use detector::Session;
use evdev::{EventType, InputEvent, Key};
use evdev::uinput::VirtualDevice;
use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::Application;
use log::{debug, info, warn};
use std::time::Instant;
use popup::Popup;
use std::cell::RefCell;
use std::process;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// State machine
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug)]
enum State {
    Idle,
    /// A letter with accent variants was pressed; waiting to see if it is held.
    Pending { key: Key, base_char: char },
    /// The key was held long enough; the popup is visible.
    PopupActive {
        trigger_key: Key,
        base_char: char,
        accents: &'static [char],
        selected: usize,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// Daemon
// ─────────────────────────────────────────────────────────────────────────────

struct Daemon {
    state:   State,
    vdev:    VirtualDevice,
    popup:   Option<Popup>,
    session: Session,
    shift:   bool,
    caps:    bool,
}

impl Daemon {
    fn new(vdev: VirtualDevice, session: Session) -> Self {
        Self {
            state: State::Idle,
            vdev,
            popup: None,
            session,
            shift: false,
            caps: false,
        }
    }

    // ── Top-level event dispatcher ───────────────────────────────────────────

    fn handle_event(&mut self, event: InputEvent) {
        let is_key = event.event_type() == EventType::KEY;
        let t0 = is_key.then(Instant::now);
        // Track modifier state so we know the effective character of a keypress.
        if is_key {
            match Key::new(event.code()) {
                Key::KEY_LEFTSHIFT | Key::KEY_RIGHTSHIFT => {
                    self.shift = event.value() != 0;
                }
                Key::KEY_CAPSLOCK if event.value() == 1 => {
                    self.caps = !self.caps;
                }
                _ => {}
            }
        }

        // Take the current state to avoid simultaneous borrows.
        let state = std::mem::replace(&mut self.state, State::Idle);

        let reprocess = match state {
            State::Idle => {
                self.state = State::Idle;
                self.handle_idle(event);
                None
            }
            State::Pending { key, base_char } => {
                self.handle_pending(event, key, base_char)
            }
            State::PopupActive { trigger_key, base_char, accents, selected } => {
                self.handle_popup(event, trigger_key, base_char, accents, selected)
            }
        };

        // handle_popup can ask us to re-process an event in Idle state
        // (e.g. when another key is pressed while the popup is open).
        if let Some(ev) = reprocess {
            self.handle_idle(ev);
        }
        if let Some(t) = t0 {
            debug!("key event processed in {:?}", t.elapsed());
        }
    }

    // ── Idle ─────────────────────────────────────────────────────────────────

    fn handle_idle(&mut self, event: InputEvent) {
        // Key-down for a letter that has accent variants?  Enter Pending.
        if event.event_type() == EventType::KEY && event.value() == 1 {
            let key = Key::new(event.code());
            if let Some(ch) = keyboard::key_to_char(key, self.shift, self.caps) {
                if accents::has_variants(ch) {
                    debug!("Pending: {ch:?}");
                    self.state = State::Pending { key, base_char: ch };
                    return; // do NOT pass through yet
                }
            }
        }
        self.passthrough(event);
    }

    // ── Pending ───────────────────────────────────────────────────────────────

    /// Returns `Some(event)` if the event should be re-processed in Idle.
    fn handle_pending(
        &mut self,
        event: InputEvent,
        pending_key: Key,
        base_char: char,
    ) -> Option<InputEvent> {
        if event.event_type() == EventType::KEY && Key::new(event.code()) == pending_key {
            match event.value() {
                0 => {
                    // Released before autorepeat → was a normal tap, passthrough both
                    self.emit_key(pending_key, 1); // the key-down we held back
                    self.passthrough(event);       // key-up
                    self.state = State::Idle;
                }
                2 => {
                    // First autorepeat = held long enough → show popup
                    let variants = accents::variants(base_char).unwrap();
                    let (cx, cy) = cursor_pos();
                    info!("Showing popup for {base_char:?} at ({cx},{cy})");
                    self.popup = Some(Popup::new(base_char, variants, cx, cy));
                    self.state = State::PopupActive {
                        trigger_key: pending_key,
                        base_char,
                        accents: variants,
                        selected: 0,
                    };
                }
                _ => {
                    // Another event for the same key — keep pending
                    self.state = State::Pending { key: pending_key, base_char };
                }
            }
            return None;
        }

        // Non-KEY events (EV_SYN, EV_MSC, EV_LED, …) arrive between every key
        // event.  They must not break hold detection — pass them through and
        // stay in Pending.
        if event.event_type() != EventType::KEY {
            self.state = State::Pending { key: pending_key, base_char };
            self.passthrough(event);
            return None;
        }

        // A different *key* arrived while pending.
        // Flush the buffered key-down, then re-process the new event in Idle.
        self.emit_key(pending_key, 1);
        self.state = State::Idle;
        Some(event)
    }

    // ── PopupActive ──────────────────────────────────────────────────────────

    /// Returns `Some(event)` if the event should be re-processed in Idle.
    fn handle_popup(
        &mut self,
        event: InputEvent,
        trigger_key: Key,
        base_char: char,
        accents: &'static [char],
        mut selected: usize,
    ) -> Option<InputEvent> {
        // Ignore non-key events and autorepeat while popup is open.
        if event.event_type() != EventType::KEY || event.value() == 2 {
            self.state = State::PopupActive { trigger_key, base_char, accents, selected };
            return None;
        }

        let key   = Key::new(event.code());
        let value = event.value();

        // Release of the trigger key → keep popup open (macOS-style: release doesn't confirm).
        if key == trigger_key && value == 0 {
            self.state = State::PopupActive { trigger_key, base_char, accents, selected };
            return None;
        }

        // Only react on key-down from here on.
        if value != 1 {
            self.state = State::PopupActive { trigger_key, base_char, accents, selected };
            return None;
        }

        // Left / Right arrow
        if key == Key::KEY_LEFT {
            selected = selected.saturating_sub(1);
            self.update_selection(selected);
            self.state = State::PopupActive { trigger_key, base_char, accents, selected };
            return None;
        }
        if key == Key::KEY_RIGHT {
            selected = (selected + 1).min(accents.len()); // total items = 1 + accents.len()
            self.update_selection(selected);
            self.state = State::PopupActive { trigger_key, base_char, accents, selected };
            return None;
        }

        // Number keys 1–9
        let maybe_idx = match key {
            Key::KEY_1 => Some(0), Key::KEY_2 => Some(1), Key::KEY_3 => Some(2),
            Key::KEY_4 => Some(3), Key::KEY_5 => Some(4), Key::KEY_6 => Some(5),
            Key::KEY_7 => Some(6), Key::KEY_8 => Some(7), Key::KEY_9 => Some(8),
            _ => None,
        };
        if let Some(idx) = maybe_idx {
            let total = 1 + accents.len();
            if idx < total {
                let ch = if idx == 0 { base_char } else { accents[idx - 1] };
                self.update_selection(idx);
                self.confirm_flash(ch);
            }
            return None;
        }

        // Enter / Space / KP-Enter → confirm current
        if matches!(key, Key::KEY_ENTER | Key::KEY_SPACE | Key::KEY_KPENTER) {
            let ch = if selected == 0 { base_char } else { accents[selected - 1] };
            self.confirm_flash(ch);
            return None;
        }

        // Escape → type the original (base) character and close
        if key == Key::KEY_ESC {
            self.confirm(base_char);
            return None;
        }

        // Same letter again → cycle through accents
        let same_letter = keyboard::key_to_char(key, self.shift, self.caps)
            .map(|c| c.to_ascii_lowercase() == base_char.to_ascii_lowercase())
            .unwrap_or(false);
        if same_letter {
            selected = (selected + 1) % (1 + accents.len());
            self.update_selection(selected);
            self.state = State::PopupActive { trigger_key, base_char, accents, selected };
            return None;
        }

        // Any other key → confirm current selection, re-process this key in Idle
        let ch = if selected == 0 { base_char } else { accents[selected - 1] };
        self.confirm(ch);
        Some(event)
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Pass an event through to the virtual keyboard as-is.
    fn passthrough(&mut self, event: InputEvent) {
        if let Err(e) = self.vdev.emit(&[event]) {
            warn!("Passthrough error: {e}");
        }
    }

    /// Synthesise a key press or release and emit it (followed by SYN).
    fn emit_key(&mut self, key: Key, value: i32) {
        let _ = self.vdev.emit(&[
            InputEvent::new_now(EventType::KEY, key.code(), value),
            InputEvent::new_now(EventType::SYNCHRONIZATION, 0, 0),
        ]);
    }

    fn update_selection(&mut self, idx: usize) {
        if let Some(p) = &mut self.popup {
            p.set_selected(idx);
        }
    }

    /// Inject the chosen character and tear down the popup immediately.
    fn confirm(&mut self, ch: char) {
        if let Some(p) = self.popup.take() {
            p.close();
        }
        self.state = State::Idle;
        let session = self.session.clone();
        glib::timeout_add_local(Duration::from_millis(80), move || {
            injector::inject(ch, &session);
            ControlFlow::Break
        });
    }

    /// Like `confirm`, but keeps the popup visible for 100 ms so the user can
    /// see the selection move before it disappears.
    fn confirm_flash(&mut self, ch: char) {
        self.state = State::Idle;
        if let Some(popup) = self.popup.take() {
            let session = self.session.clone();
            let mut popup_slot = Some(popup);
            glib::timeout_add_local(Duration::from_millis(100), move || {
                if let Some(p) = popup_slot.take() {
                    p.close();
                }
                // Give the compositor ~80 ms to return focus before injecting.
                let session = session.clone();
                glib::timeout_add_local(Duration::from_millis(80), move || {
                    injector::inject(ch, &session);
                    ControlFlow::Break
                });
                ControlFlow::Break
            });
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Cursor position
// ─────────────────────────────────────────────────────────────────────────────

/// Query the current mouse cursor position in root (screen) coordinates.
fn cursor_pos() -> (i32, i32) {
    // Hyprland: use hyprctl cursorpos (most accurate on Wayland)
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
        if let Ok(out) = process::Command::new("hyprctl").arg("cursorpos").output() {
            if let Ok(s) = std::str::from_utf8(&out.stdout) {
                // Output: "X, Y"
                let mut parts = s.trim().splitn(2, ", ");
                if let (Some(xs), Some(ys)) = (parts.next(), parts.next()) {
                    if let (Ok(x), Ok(y)) = (xs.parse::<i32>(), ys.parse::<i32>()) {
                        return (x, y);
                    }
                }
            }
        }
    }

    // X11 / XWayland fallback
    if let Ok(out) = process::Command::new("xdotool").arg("getmouselocation").output() {
        if let Ok(s) = std::str::from_utf8(&out.stdout) {
            let parse = |prefix: &str| -> Option<i32> {
                s.split_whitespace()
                    .find(|w| w.starts_with(prefix))
                    .and_then(|w| w[prefix.len()..].parse().ok())
            };
            if let (Some(x), Some(y)) = (parse("x:"), parse("y:")) {
                return (x, y);
            }
        }
    }

    (960, 540)
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    let session = detector::session();
    let desktop = detector::desktop();
    info!("Session: {session:?}  Desktop: {desktop}");

    // Find physical keyboards and build the uinput passthrough device.
    let keyboards = keyboard::find_keyboards();
    if keyboards.is_empty() {
        anyhow::bail!(
            "No keyboard devices found. \
             Make sure your user is in the 'input' group:\n  \
             sudo usermod -aG input $USER"
        );
    }

    let vdev = keyboard::create_passthrough(&keyboards)
        .context("Failed to create uinput passthrough device")?;

    // Bounded mpsc channel: keyboard threads → GTK main thread.
    // SyncSender with a generous bound so bursts never block the reader threads.
    let (tx, rx) = std::sync::mpsc::sync_channel::<InputEvent>(4096);
    let rx = Arc::new(Mutex::new(rx));

    keyboard::spawn_readers(keyboards, tx);

    // ── GTK application ───────────────────────────────────────────────────────
    let app = Application::builder()
        .application_id("dev.presshold")
        .build();

    // We move the VirtualDevice into the activate callback.
    // Consumed once; subsequent activate calls (if any) are ignored.
    let vdev_cell     = RefCell::new(Some(vdev));
    let session_clone = session.clone();

    app.connect_activate(move |app| {
        // Take ownership; ignore subsequent activations.
        let Some(vdev) = vdev_cell.borrow_mut().take() else { return };

        let daemon = Rc::new(RefCell::new(Daemon::new(vdev, session_clone.clone())));

        // Poll the mpsc receiver every 1 ms from the GTK main loop.
        // 1 ms adds imperceptible latency while keeping CPU usage negligible.
        let rx = Arc::clone(&rx);
        glib::timeout_add_local(Duration::from_millis(1), move || {
            if let Ok(guard) = rx.lock() {
                while let Ok(event) = guard.try_recv() {
                    daemon.borrow_mut().handle_event(event);
                }
            }
            ControlFlow::Continue
        });

        // Prevent the application from quitting when there are no open windows.
        // We intentionally leak the guard — the process lifetime IS the hold.
        std::mem::forget(app.hold());

        info!("presshold is running. Hold a letter key to see accent options.");
    });

    // gtk4 0.10+ returns std::process::ExitCode directly
    let _status = app.run_with_args::<String>(&[]);
    Ok(())
}

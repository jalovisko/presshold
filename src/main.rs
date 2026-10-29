mod accents;
mod compose;
mod popup_keys;
mod detector;
mod injector;
mod keyboard;
mod popup;

use anyhow::{Context, Result};
use detector::Session;
use evdev::{EventType, InputEvent, Key};
use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::Application;
use log::{debug, info, warn};
use std::time::Instant;
use popup::Popup;
use popup_keys::PopupAction;
use std::cell::RefCell;
use std::collections::VecDeque;
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

/// Pause between closing the popup and typing, so the compositor has settled.
const INJECT_DELAY: Duration = Duration::from_millis(80);
/// How long a direct pick stays highlighted before the popup closes.
const FLASH: Duration = Duration::from_millis(100);

/// A character waiting to be typed once the popup has gone away.
struct PendingInject {
    ch: char,
    /// When to close the popup, if it is still showing the pick.
    close_popup_at: Option<Instant>,
    inject_at: Instant,
}

struct Daemon {
    state:       State,
    out:         Box<dyn keyboard::KeyOut>,
    popup:       Option<Popup>,
    session:     Session,
    /// Key configured as XKB Compose, if any. With it, accents are typed as
    /// Compose sequences on the passthrough device instead of via wtype.
    compose_key: Option<Key>,
    /// Order in which a vowel's accents are offered.
    order:       accents::AccentOrder,
    shift:       bool,
    /// Which Shift key the user is holding, so it can be lifted and restored.
    shift_key:   Key,
    caps:        bool,
    pending:     Option<PendingInject>,
    /// Events that arrived while a character was waiting to be typed; they
    /// are replayed, in order, right after it.
    deferred:    VecDeque<InputEvent>,
}

impl Daemon {
    fn new(out: Box<dyn keyboard::KeyOut>, session: Session, compose_key: Option<Key>, order: accents::AccentOrder) -> Self {
        Self {
            state: State::Idle,
            out,
            popup: None,
            session,
            compose_key,
            order,
            shift: false,
            shift_key: Key::KEY_LEFTSHIFT,
            caps: false,
            pending: None,
            deferred: VecDeque::new(),
        }
    }

    // ── Top-level event dispatcher ───────────────────────────────────────────

    fn handle_event(&mut self, event: InputEvent) {
        // While a chosen character waits to be typed, hold everything back so
        // it lands after the character, not before.
        if self.pending.is_some() {
            self.deferred.push_back(event);
            return;
        }

        let is_key = event.event_type() == EventType::KEY;
        let t0 = is_key.then(Instant::now);
        // Track modifier state so we know the effective character of a keypress.
        if is_key {
            match Key::new(event.code()) {
                k @ (Key::KEY_LEFTSHIFT | Key::KEY_RIGHTSHIFT) => {
                    self.shift = event.value() != 0;
                    if self.shift {
                        self.shift_key = k;
                    }
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
                    let variants = accents::variants_in(base_char, self.order).unwrap();
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
                    // Another event for the same key. Keep pending.
                    self.state = State::Pending { key: pending_key, base_char };
                }
            }
            return None;
        }

        // Non-KEY events (EV_SYN, EV_MSC, EV_LED, …) arrive between every key
        // event.  They must not break hold detection. Pass them through and
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

        let same_letter = keyboard::key_to_char(key, self.shift, self.caps)
            .map(|c| c.to_ascii_lowercase() == base_char.to_ascii_lowercase())
            .unwrap_or(false);
        let total = 1 + accents.len();
        let item = |idx: usize| if idx == 0 { base_char } else { accents[idx - 1] };

        match popup_keys::popup_action(key, same_letter) {
            PopupAction::MoveLeft => {
                selected = selected.saturating_sub(1);
                self.update_selection(selected);
                self.state = State::PopupActive { trigger_key, base_char, accents, selected };
            }
            PopupAction::MoveRight => {
                selected = (selected + 1).min(accents.len());
                self.update_selection(selected);
                self.state = State::PopupActive { trigger_key, base_char, accents, selected };
            }
            PopupAction::Cycle => {
                selected = (selected + 1) % total;
                self.update_selection(selected);
                self.state = State::PopupActive { trigger_key, base_char, accents, selected };
            }
            PopupAction::Pick(idx) if idx < total => {
                self.update_selection(idx);
                self.confirm_flash(item(idx));
            }
            PopupAction::Pick(_) => {
                self.state = State::PopupActive { trigger_key, base_char, accents, selected };
            }
            // Escape → type the original (base) character and close.
            PopupAction::Cancel => self.confirm(base_char, None),
            // Space, Enter, punctuation, another letter…: type the selected
            // character, then the key itself.
            PopupAction::ConfirmThenPassthrough => self.confirm(item(selected), Some(event)),
        }
        None
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Pass an event through to the virtual keyboard as-is.
    fn passthrough(&mut self, event: InputEvent) {
        if let Err(e) = self.out.emit(&[event]) {
            warn!("Passthrough error: {e}");
        }
    }

    /// Synthesise a key press or release and emit it (followed by SYN).
    fn emit_key(&mut self, key: Key, value: i32) {
        let _ = self.out.emit(&[
            InputEvent::new_now(EventType::KEY, key.code(), value),
            InputEvent::new_now(EventType::SYNCHRONIZATION, 0, 0),
        ]);
    }

    fn update_selection(&mut self, idx: usize) {
        if let Some(p) = &mut self.popup {
            p.set_selected(idx);
        }
    }

    /// Close the popup now and type `ch` once the compositor has settled.
    /// `then` (the key that triggered the confirmation) is replayed after it.
    fn confirm(&mut self, ch: char, then: Option<InputEvent>) {
        if let Some(p) = self.popup.take() {
            p.close();
        }
        self.state = State::Idle;
        self.pending = Some(PendingInject {
            ch,
            close_popup_at: None,
            inject_at: Instant::now() + INJECT_DELAY,
        });
        if let Some(ev) = then {
            self.deferred.push_back(ev);
        }
    }

    /// Like `confirm`, but keeps the popup visible for a moment so the user
    /// can see the selection move before it disappears.
    fn confirm_flash(&mut self, ch: char) {
        self.state = State::Idle;
        let now = Instant::now();
        self.pending = Some(PendingInject {
            ch,
            close_popup_at: Some(now + FLASH),
            inject_at: now + FLASH + INJECT_DELAY,
        });
    }

    /// Drive pending work: close a lingering popup, type the chosen character
    /// when its time has come, then replay the keys held back meanwhile.
    /// Called from the main loop every millisecond.
    fn tick(&mut self) {
        let Some(pending) = self.pending.as_ref() else { return };
        let now = Instant::now();
        if pending.close_popup_at.is_some_and(|t| now >= t) {
            if let Some(p) = self.popup.take() {
                p.close();
            }
            if let Some(p) = self.pending.as_mut() {
                p.close_popup_at = None;
            }
        }
        let Some(pending) = self.pending.as_ref() else { return };
        if now < pending.inject_at {
            return;
        }
        let ch = pending.ch;
        self.pending = None;
        self.type_char(ch);
        let replay: Vec<InputEvent> = self.deferred.drain(..).collect();
        for ev in replay {
            self.handle_event(ev);
        }
    }

    fn is_gnome_wayland(&self) -> bool {
        matches!(self.session, Session::Wayland)
            && std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .to_lowercase()
                .contains("gnome")
    }

    /// Type one character into the focused application.
    fn type_char(&mut self, ch: char) {
        if let Some(compose) = self.compose_key {
            if let Some(seq) = compose::sequence(ch, compose) {
                debug!("Typing {ch:?} as a Compose sequence");
                self.type_sequence(&seq);
                return;
            }
        }
        if self.is_gnome_wayland() {
            self.type_unicode_input(ch);
            return;
        }
        injector::inject(ch, &self.session);
    }

    /// Type a character on GNOME Wayland with the GTK Unicode input method:
    /// Ctrl+Shift+U, hex codepoint digits, Enter.
    ///
    /// This works in GTK apps, GNOME Terminal, Firefox, Chrome, LibreOffice and
    /// any app that supports the GTK Unicode input method. Our virtual device
    /// is trusted by GNOME because it replaced the grabbed physical keyboard.
    fn type_unicode_input(&mut self, ch: char) {
        let codepoint = format!("{:x}", ch as u32);

        let hex_key = |c: char| -> Key {
            match c {
                '0' => Key::KEY_0, '1' => Key::KEY_1, '2' => Key::KEY_2,
                '3' => Key::KEY_3, '4' => Key::KEY_4, '5' => Key::KEY_5,
                '6' => Key::KEY_6, '7' => Key::KEY_7, '8' => Key::KEY_8,
                '9' => Key::KEY_9, 'a' => Key::KEY_A, 'b' => Key::KEY_B,
                'c' => Key::KEY_C, 'd' => Key::KEY_D, 'e' => Key::KEY_E,
                _   => Key::KEY_F,
            }
        };

        let syn = || InputEvent::new_now(EventType::SYNCHRONIZATION, 0, 0);
        let dn  = |k: Key| InputEvent::new_now(EventType::KEY, k.code(), 1);
        let up  = |k: Key| InputEvent::new_now(EventType::KEY, k.code(), 0);

        let mut events: Vec<InputEvent> = vec![
            // Ctrl+Shift+U — enter Unicode input mode
            dn(Key::KEY_LEFTCTRL),  syn(),
            dn(Key::KEY_LEFTSHIFT), syn(),
            dn(Key::KEY_U),         syn(),
            up(Key::KEY_U),         syn(),
            up(Key::KEY_LEFTSHIFT), syn(),
            up(Key::KEY_LEFTCTRL),  syn(),
        ];

        // Hex digits
        for c in codepoint.chars() {
            let k = hex_key(c);
            events.push(dn(k)); events.push(syn());
            events.push(up(k)); events.push(syn());
        }

        // Enter to confirm
        events.push(dn(Key::KEY_ENTER)); events.push(syn());
        events.push(up(Key::KEY_ENTER)); events.push(syn());

        if let Err(e) = self.out.emit(&events) {
            warn!("Unicode input emit failed: {e}");
        }
    }

    /// Tap out `seq` on the passthrough device. A Shift the user is still
    /// holding is lifted first and put back afterwards, so it cannot leak
    /// into the sequence.
    fn type_sequence(&mut self, seq: &[compose::KeyStroke]) {
        let held_shift = self.shift.then_some(self.shift_key);
        if let Some(k) = held_shift {
            self.emit_key(k, 0);
        }
        for stroke in seq {
            if stroke.shift {
                self.emit_key(Key::KEY_LEFTSHIFT, 1);
            }
            self.emit_key(stroke.key, 1);
            self.emit_key(stroke.key, 0);
            if stroke.shift {
                self.emit_key(Key::KEY_LEFTSHIFT, 0);
            }
        }
        if let Some(k) = held_shift {
            self.emit_key(k, 1);
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

const USAGE: &str = "\
Usage: presshold [--grave-first]

  --grave-first   offer the grave accent before the acute one on vowels
                  (è before é), so Italian-style accents are one tap away.
                  Also enabled by PRESSHOLD_GRAVE_FIRST=1 in the environment.
  -h, --help      show this help";

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().skip(1).any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return Ok(());
    }
    if let Some(bad) = args.iter().skip(1).find(|a| *a != "--grave-first") {
        anyhow::bail!("unknown argument {bad:?}\n{USAGE}");
    }
    let order = accents::AccentOrder::from(&args, std::env::var("PRESSHOLD_GRAVE_FIRST").ok().as_deref());
    info!("Accent order: {order:?}");

    let session = detector::session();
    let desktop = detector::desktop();
    info!("Session: {session:?}  Desktop: {desktop}");

    // On GNOME Wayland, spawn ydotoold if it is installed but not yet running.
    // ydotoold is needed for character injection (wtype doesn't work on GNOME).
    if matches!(session, Session::Wayland) && desktop.to_lowercase().contains("gnome") {
        let running = process::Command::new("pgrep")
            .args(["-x", "ydotoold"])
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !running {
            match process::Command::new("ydotoold")
                .stdout(process::Stdio::null())
                .stderr(process::Stdio::null())
                .spawn()
            {
                Ok(_) => {
                    info!("Spawned ydotoold for GNOME Wayland injection");
                    // Give ydotoold a moment to create its socket.
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                Err(e) => warn!("ydotoold not found — character injection may not work on GNOME Wayland: {e}"),
            }
        }
    }

    // Find physical keyboards and build the uinput passthrough device.
    let keyboards = keyboard::find_keyboards();
    if keyboards.is_empty() {
        anyhow::bail!(
            "No keyboard devices found. \
             Make sure your user is in the 'input' group:\n  \
             sudo usermod -aG input $USER"
        );
    }

    let vdev = keyboard::create_passthrough()
        .context("Failed to create uinput passthrough device")?;

    let compose_key = compose::detect_compose_key();
    match compose_key {
        Some(k) => info!("Compose key is {k:?}: accents will be typed as Compose sequences"),
        None => info!("No Compose key configured: accents will be injected with wtype/ydotool/xdotool"),
    }

    // Bounded mpsc channel: keyboard threads → GTK main thread.
    // SyncSender with a generous bound so bursts never block the reader threads.
    let (tx, rx) = std::sync::mpsc::sync_channel::<InputEvent>(4096);
    let rx = Arc::new(Mutex::new(rx));

    let grabbed = keyboard::Grabbed::default();
    keyboard::spawn_readers(keyboards, &tx, &grabbed);
    // Keyboards plugged in later are grabbed as they appear.
    keyboard::spawn_hotplug_watcher(tx, grabbed);

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

        let daemon = Rc::new(RefCell::new(Daemon::new(Box::new(vdev), session_clone.clone(), compose_key, order)));

        // Poll the mpsc receiver every 1 ms from the GTK main loop.
        // 1 ms adds imperceptible latency while keeping CPU usage negligible.
        let rx = Arc::clone(&rx);
        glib::timeout_add_local(Duration::from_millis(1), move || {
            if let Ok(guard) = rx.lock() {
                while let Ok(event) = guard.try_recv() {
                    daemon.borrow_mut().handle_event(event);
                }
            }
            daemon.borrow_mut().tick();
            ControlFlow::Continue
        });

        // Prevent the application from quitting when there are no open windows.
        // We intentionally leak the guard. The process lifetime IS the hold.
        std::mem::forget(app.hold());

        info!("presshold is running. Hold a letter key to see accent options.");
    });

    // gtk4 0.10+ returns std::process::ExitCode directly
    let _status = app.run_with_args::<String>(&[]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Recorder(Rc<RefCell<Vec<InputEvent>>>);
    impl keyboard::KeyOut for Recorder {
        fn emit(&mut self, events: &[InputEvent]) -> std::io::Result<()> {
            self.0.borrow_mut().extend_from_slice(events);
            Ok(())
        }
    }

    fn daemon() -> (Daemon, Rc<RefCell<Vec<InputEvent>>>) {
        daemon_with(accents::AccentOrder::GraveFirst)
    }

    /// Most tests use the grave-first order so that index 1 is `è`.
    fn daemon_with(order: accents::AccentOrder) -> (Daemon, Rc<RefCell<Vec<InputEvent>>>) {
        let out = Rc::new(RefCell::new(Vec::new()));
        let d = Daemon::new(Box::new(Recorder(out.clone())), Session::Wayland, Some(Key::KEY_CAPSLOCK), order);
        (d, out)
    }

    fn key(k: Key, value: i32) -> InputEvent {
        InputEvent::new(EventType::KEY, k.code(), value)
    }

    fn typed(out: &Rc<RefCell<Vec<InputEvent>>>) -> Vec<(Key, i32)> {
        out.borrow()
            .iter()
            .filter(|e| e.event_type() == EventType::KEY)
            .map(|e| (Key::new(e.code()), e.value()))
            .collect()
    }

    fn popup_open(d: &mut Daemon, base: char, trigger: Key, selected: usize) {
        d.state = State::PopupActive {
            trigger_key: trigger,
            base_char: base,
            accents: accents::variants_in(base, d.order).unwrap(),
            selected,
        };
    }

    fn settle(d: &mut Daemon) {
        std::thread::sleep(Duration::from_millis(250));
        d.tick();
    }

    #[test]
    fn space_is_typed_after_the_composed_accent() {
        let (mut d, out) = daemon();
        popup_open(&mut d, 'e', Key::KEY_E, 1); // è
        d.handle_event(key(Key::KEY_SPACE, 1));
        d.handle_event(key(Key::KEY_SPACE, 0));
        assert!(typed(&out).is_empty(), "nothing may be typed before the accent is injected");
        settle(&mut d);
        assert_eq!(typed(&out), vec![
            (Key::KEY_CAPSLOCK, 1), (Key::KEY_CAPSLOCK, 0),
            (Key::KEY_GRAVE, 1), (Key::KEY_GRAVE, 0),
            (Key::KEY_E, 1), (Key::KEY_E, 0),
            (Key::KEY_SPACE, 1), (Key::KEY_SPACE, 0),
        ]);
        assert!(matches!(d.state, State::Idle));
    }

    #[test]
    fn default_macos_order_puts_the_acute_accent_first() {
        let (mut d, out) = daemon_with(accents::AccentOrder::MacOs);
        popup_open(&mut d, 'e', Key::KEY_E, 1); // é
        d.handle_event(key(Key::KEY_SPACE, 1));
        d.handle_event(key(Key::KEY_SPACE, 0));
        settle(&mut d);
        assert_eq!(typed(&out), vec![
            (Key::KEY_CAPSLOCK, 1), (Key::KEY_CAPSLOCK, 0),
            (Key::KEY_APOSTROPHE, 1), (Key::KEY_APOSTROPHE, 0),
            (Key::KEY_E, 1), (Key::KEY_E, 0),
            (Key::KEY_SPACE, 1), (Key::KEY_SPACE, 0),
        ]);
    }

    #[test]
    fn digit_pick_types_only_the_accent() {
        let (mut d, out) = daemon();
        popup_open(&mut d, 'a', Key::KEY_A, 0);
        d.handle_event(key(Key::KEY_2, 1)); // first accent: à
        settle(&mut d);
        assert_eq!(typed(&out), vec![
            (Key::KEY_CAPSLOCK, 1), (Key::KEY_CAPSLOCK, 0),
            (Key::KEY_GRAVE, 1), (Key::KEY_GRAVE, 0),
            (Key::KEY_A, 1), (Key::KEY_A, 0),
        ]);
    }

    #[test]
    fn shift_held_by_the_user_is_lifted_while_composing() {
        let (mut d, out) = daemon();
        d.handle_event(key(Key::KEY_LEFTSHIFT, 1)); // user holds Shift (typed È popup)
        popup_open(&mut d, 'E', Key::KEY_E, 1);     // È
        d.handle_event(key(Key::KEY_DOT, 1));       // Shift+. = '>'
        settle(&mut d);
        assert_eq!(typed(&out), vec![
            (Key::KEY_LEFTSHIFT, 1),                      // passthrough of the real press
            (Key::KEY_LEFTSHIFT, 0),                      // lifted for the sequence
            (Key::KEY_CAPSLOCK, 1), (Key::KEY_CAPSLOCK, 0),
            (Key::KEY_GRAVE, 1), (Key::KEY_GRAVE, 0),
            (Key::KEY_LEFTSHIFT, 1), (Key::KEY_E, 1), (Key::KEY_E, 0), (Key::KEY_LEFTSHIFT, 0),
            (Key::KEY_LEFTSHIFT, 1),                      // restored
            (Key::KEY_DOT, 1),
        ]);
    }

    #[test]
    fn keys_pressed_during_injection_are_replayed_in_order() {
        let (mut d, out) = daemon();
        popup_open(&mut d, 'o', Key::KEY_O, 1); // ò
        d.handle_event(key(Key::KEY_SPACE, 1));
        d.handle_event(key(Key::KEY_SPACE, 0));
        d.handle_event(key(Key::KEY_K, 1));
        d.handle_event(key(Key::KEY_K, 0));
        settle(&mut d);
        let tail: VecDeque<_> = typed(&out).into_iter().rev().take(4).collect();
        assert_eq!(tail.into_iter().rev().collect::<Vec<_>>(), vec![
            (Key::KEY_SPACE, 1), (Key::KEY_SPACE, 0), (Key::KEY_K, 1), (Key::KEY_K, 0),
        ]);
    }
}

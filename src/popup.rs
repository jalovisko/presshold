use gtk4::prelude::*;
use gtk4::{
    Align, Box as GBox, CssProvider, Label, Orientation,
    Window, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use log::debug;

// ─────────────────────────────────────────────────────────────────────────────
// Styling
// ─────────────────────────────────────────────────────────────────────────────

/// Popup sizing/typography scale relative to a 1920px-wide reference display.
/// Keeps the popup's *physical* footprint consistent across small laptop
/// panels, ultrawides, and 4K monitors, instead of a fixed pixel size that
/// looks oversized on lower-resolution screens.
fn popup_scale(mon_w: i32) -> f64 {
    (mon_w as f64 / 1920.0).clamp(0.72, 1.15)
}

fn build_css(scale: f64) -> String {
    let px = |base: f64| (base * scale).round() as i32;
    format!(
        "
.accent-popup {{
    background: linear-gradient(160deg, rgba(46,46,50,0.97), rgba(24,24,27,0.97));
    border-radius: {popup_radius}px;
    border: 1px solid rgba(255,255,255,0.12);
    box-shadow: 0 {shadow_y}px {shadow_blur}px rgba(0,0,0,0.5), inset 0 1px 0 rgba(255,255,255,0.06);
}}
.accent-item {{
    padding: {pad_v}px {pad_h}px;
    border-radius: {item_radius}px;
    min-width: {item_w}px;
    transition: background 120ms ease, box-shadow 120ms ease;
}}
.accent-item.selected {{
    background: linear-gradient(160deg, rgba(70,138,255,0.95), rgba(28,96,224,0.95));
    box-shadow: 0 {sel_shadow_y}px {sel_shadow_blur}px rgba(30,108,229,0.45), inset 0 1px 0 rgba(255,255,255,0.25);
}}
.accent-char {{
    color: white;
    font-size: {char_size}px;
    font-weight: 600;
    font-family: \"Inter\", \"Cantarell\", sans-serif;
}}
.accent-num {{
    color: rgba(220,220,220,0.55);
    font-size: {num_size}px;
    font-weight: 500;
}}
",
        popup_radius = px(14.0),
        shadow_y = px(10.0),
        shadow_blur = px(28.0),
        pad_v = px(6.0),
        pad_h = px(9.0),
        item_radius = px(10.0),
        item_w = px(46.0),
        sel_shadow_y = px(3.0),
        sel_shadow_blur = px(10.0),
        char_size = px(23.0),
        num_size = px(10.0).max(8),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Popup
// ─────────────────────────────────────────────────────────────────────────────

pub struct Popup {
    pub window: Window,
    items: Vec<GBox>,
    selected: usize,
}

impl Popup {
    /// Create and show the accent popup.
    ///
    /// The popup does **not** steal keyboard focus so that xdotool / wtype
    /// still delivers characters to the previously focused window.
    /// `base_char` is shown as item #1 (index 0); `accents` follow as #2, #3 …
    pub fn new(base_char: char, accents: &[char], cursor_x: i32, cursor_y: i32) -> Self {
        // ── Scale ────────────────────────────────────────────────────────────
        // Look up the monitor under the cursor *before* building the CSS so
        // typography/padding/margins can adapt to that monitor's resolution.
        let (_, _, mon_w, _) = monitor_geometry(cursor_x, cursor_y);
        let scale = popup_scale(mon_w);

        // ── CSS ──────────────────────────────────────────────────────────────
        let provider = CssProvider::new();
        provider.load_from_data(&build_css(scale));
        if let Some(display) = gtk4::gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        // ── Window ────────────────────────────────────────────────────────────
        let window = Window::builder()
            .decorated(false)
            .resizable(false)
            .deletable(false)
            .focusable(false)           // do not steal keyboard focus
            .css_classes(vec!["accent-popup"])
            .build();

        // ── Character row ─────────────────────────────────────────────────────
        let margin = ((8.0 * scale).round() as i32).max(4);
        let spacing = ((4.0 * scale).round() as i32).max(2);
        let row = GBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(spacing)
            .margin_top(margin)
            .margin_bottom(margin)
            .margin_start(margin)
            .margin_end(margin)
            .build();

        // base_char is item #1; accent variants follow as #2, #3, …
        let all_chars: Vec<char> = std::iter::once(base_char)
            .chain(accents.iter().copied())
            .collect();
        let mut items: Vec<GBox> = Vec::with_capacity(all_chars.len());

        for (i, ch) in all_chars.iter().enumerate() {
            let item = GBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(1)
                .halign(Align::Center)
                .css_classes(vec!["accent-item"])
                .build();

            item.append(
                &Label::builder()
                    .label(ch.to_string())
                    .css_classes(vec!["accent-char"])
                    .build(),
            );
            item.append(
                &Label::builder()
                    .label((i + 1).to_string())
                    .css_classes(vec!["accent-num"])
                    .build(),
            );

            row.append(&item);
            items.push(item);
        }

        window.set_child(Some(&row));

        if let Some(first) = items.first() {
            first.add_css_class("selected");
        }

        // ── Position ──────────────────────────────────────────────────────────
        #[cfg(feature = "layer-shell")]
        let used_layer_shell =
            layer_shell_position(&window, 1 + accents.len(), cursor_x, cursor_y, scale);
        #[cfg(not(feature = "layer-shell"))]
        let used_layer_shell = false;

        if !used_layer_shell {
            // X11 / non-layer-shell Wayland fallback: position after realize.
            let window_weak = window.downgrade();
            window.connect_realize(move |_| {
                if let Some(win) = window_weak.upgrade() {
                    position_window(&win, cursor_x, cursor_y);
                }
            });
        }

        window.set_visible(true); // show without focus request

        Self { window, items, selected: 0 }
    }

    pub fn set_selected(&mut self, idx: usize) {
        if let Some(old) = self.items.get(self.selected) {
            old.remove_css_class("selected");
        }
        self.selected = idx.min(self.items.len().saturating_sub(1));
        if let Some(new) = self.items.get(self.selected) {
            new.add_css_class("selected");
        }
    }

    #[allow(dead_code)]
    pub fn selected_index(&self) -> usize { self.selected }
    #[allow(dead_code)]
    pub fn len(&self) -> usize { self.items.len() }

    pub fn close(self) {
        self.window.close();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Positioning
// ─────────────────────────────────────────────────────────────────────────────

/// Geometry `(x, y, width, height)` of the monitor under the cursor, or a
/// 1920×1080 fallback if no display/monitor can be resolved.
fn monitor_geometry(cursor_x: i32, cursor_y: i32) -> (i32, i32, i32, i32) {
    gtk4::gdk::Display::default()
        .and_then(|display| {
            display
                .monitors()
                .into_iter()
                .filter_map(|obj| obj.ok().and_then(|o| o.downcast::<gtk4::gdk::Monitor>().ok()))
                .find(|mon| {
                    let g = mon.geometry();
                    cursor_x >= g.x()
                        && cursor_x < g.x() + g.width()
                        && cursor_y >= g.y()
                        && cursor_y < g.y() + g.height()
                })
        })
        .map(|mon| {
            let g = mon.geometry();
            (g.x(), g.y(), g.width(), g.height())
        })
        .unwrap_or((0, 0, 1920, 1080))
}

fn position_window(window: &Window, cursor_x: i32, cursor_y: i32) {
    // Popup appears just below the cursor, horizontally centred on it.
    // We don't know the window size before realization; use defaults for
    // clamping (over-/under-shoot doesn't hurt much).
    let w = window.allocated_width().max(80);
    let h = window.allocated_height().max(70);

    let (mx, my, mw, mh) = monitor_geometry(cursor_x, cursor_y);

    let x = (cursor_x - w / 2).clamp(mx, mx + mw - w);
    // Show below cursor; flip above if it would go off-screen.
    let y = if cursor_y + 20 + h <= my + mh {
        cursor_y + 20
    } else {
        cursor_y - h - 10
    }
    .clamp(my, my + mh - h);

    try_move_x11(window, x, y);
    debug!("Popup placed at ({x},{y})");
}


fn try_move_x11(window: &Window, x: i32, y: i32) {
    // On X11 we can call XMoveWindow through GDK's own exported symbols.
    // This avoids adding gdk4-x11 as a compile-time dependency.
    unsafe {
        // Obtain the Xlib Display* via GDK's exported C function.
        let gdk_display = match gtk4::gdk::Display::default() {
            Some(d) => d,
            None => return,
        };

        extern "C" {
            fn gdk_x11_display_get_xdisplay(display: *mut std::ffi::c_void)
                -> *mut std::ffi::c_void;
            fn gdk_x11_surface_get_xid(surface: *mut std::ffi::c_void) -> u64;
            fn XMoveWindow(
                display: *mut std::ffi::c_void,
                window: u64,
                x: std::ffi::c_int,
                y: std::ffi::c_int,
            );
        }

        let xdisplay =
            gdk_x11_display_get_xdisplay(gdk_display.as_ptr() as *mut _);
        if xdisplay.is_null() {
            return; // Wayland display, not X11
        }

        let surface = match window.surface() {
            Some(s) => s,
            None => return,
        };
        let xid = gdk_x11_surface_get_xid(surface.as_ptr() as *mut _);
        if xid != 0 {
            XMoveWindow(xdisplay, xid, x, y);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// gtk4-layer-shell positioning (Wayland overlay surface)
// ─────────────────────────────────────────────────────────────────────────────

/// Configure the window as a Wayland layer-shell overlay and position it
/// centred just below the cursor.  Returns `true` if layer-shell was used.
#[cfg(feature = "layer-shell")]
fn layer_shell_position(
    window: &Window,
    n_accents: usize,
    cursor_x: i32,
    cursor_y: i32,
    scale: f64,
) -> bool {
    use gtk4_layer_shell::{Edge, Layer, LayerShell};

    if !gtk4_layer_shell::is_supported() {
        debug!("gtk4-layer-shell not supported by compositor, falling back");
        return false;
    }

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    // Do NOT grab keyboard. The physical keyboard is already grabbed via evdev.
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);

    // Anchor the top-left corner to position (x, y) from the screen origin.
    window.set_anchor(Edge::Left,   true);
    window.set_anchor(Edge::Top,    true);
    window.set_anchor(Edge::Right,  false);
    window.set_anchor(Edge::Bottom, false);

    // Estimate popup size to position it, scaled to match `build_css`.
    // Each character box ≈ 62 px wide plus 16 px total padding; height ≈ 80 px
    // at the 1920px-wide reference resolution.
    let est_w = n_accents as i32 * (62.0 * scale).round() as i32 + (16.0 * scale).round() as i32;
    let est_h = (80.0 * scale).round() as i32;

    // `cursor_x` / `cursor_y` are global compositor coordinates, but layer-shell
    // margins are relative to the monitor the surface lives on. Find the monitor
    // under the cursor and convert, otherwise the popup is placed off-screen on
    // any output that is not positioned at (0, 0).
    let monitor = gtk4::gdk::Display::default().and_then(|d| {
        d.monitors()
            .into_iter()
            .filter_map(|o| o.ok().and_then(|o| o.downcast::<gtk4::gdk::Monitor>().ok()))
            .find(|m| {
                let g = m.geometry();
                cursor_x >= g.x()
                    && cursor_x < g.x() + g.width()
                    && cursor_y >= g.y()
                    && cursor_y < g.y() + g.height()
            })
    });

    let (mon_x, mon_y, mon_w, mon_h) = monitor
        .as_ref()
        .map(|m| {
            let g = m.geometry();
            (g.x(), g.y(), g.width(), g.height())
        })
        .unwrap_or((0, 0, 1920, 1080));

    // Pin the surface to the monitor holding the cursor; otherwise the
    // compositor is free to map it onto a different output.
    if let Some(m) = &monitor {
        window.set_monitor(Some(m));
    }

    let rel_x = cursor_x - mon_x;
    let rel_y = cursor_y - mon_y;

    let x = (rel_x - est_w / 2).clamp(0, (mon_w - est_w).max(0));
    // Show below cursor; flip above if it would go off the bottom of the monitor.
    let y = if rel_y + 20 + est_h <= mon_h {
        rel_y + 20
    } else {
        (rel_y - est_h - 10).max(0)
    };

    window.set_margin(Edge::Left, x);
    window.set_margin(Edge::Top,  y);

    // Opposite edges are not anchored, so the surface has no implicit size.
    // Without an explicit one the compositor never maps it.
    window.set_default_size(est_w, est_h);

    debug!("layer-shell popup at monitor-relative ({x},{y}) on monitor at ({mon_x},{mon_y}), size {est_w}x{est_h}");
    true
}

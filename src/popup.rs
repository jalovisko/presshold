use gtk4::prelude::*;
use gtk4::{
    Align, Box as GBox, CssProvider, Label, Orientation,
    Window, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use log::debug;

// ─────────────────────────────────────────────────────────────────────────────
// Styling
// ─────────────────────────────────────────────────────────────────────────────

const CSS: &str = "
.accent-popup {
    background-color: rgba(38, 38, 38, 0.97);
    border-radius: 12px;
    border: 1px solid rgba(255,255,255,0.14);
}
.accent-item {
    padding: 5px 7px;
    border-radius: 8px;
    min-width: 44px;
}
.accent-item.selected {
    background-color: rgba(30, 108, 229, 0.90);
}
.accent-char {
    color: white;
    font-size: 22px;
    font-weight: bold;
}
.accent-num {
    color: rgba(210,210,210,0.70);
    font-size: 10px;
}
";

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
    pub fn new(accents: &[char], cursor_x: i32, cursor_y: i32) -> Self {
        // ── CSS ──────────────────────────────────────────────────────────────
        let provider = CssProvider::new();
        provider.load_from_data(CSS);
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
        let row = GBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(8)
            .margin_end(8)
            .build();

        let mut items: Vec<GBox> = Vec::with_capacity(accents.len());

        for (i, &ch) in accents.iter().enumerate() {
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
        let used_layer_shell = layer_shell_position(&window, accents.len(), cursor_x, cursor_y);
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

fn position_window(window: &Window, cursor_x: i32, cursor_y: i32) {
    // Popup appears just below the cursor, horizontally centred on it.
    // We don't know the window size before realization; use defaults for
    // clamping (over-/under-shoot doesn't hurt much).
    let w = window.allocated_width().max(80);
    let h = window.allocated_height().max(70);

    let display = match gtk4::gdk::Display::default() {
        Some(d) => d,
        None => return,
    };

    // Find which monitor the cursor is on.
    let (mx, my, mw, mh) = display
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
        .map(|mon| {
            let g = mon.geometry();
            (g.x(), g.y(), g.width(), g.height())
        })
        .unwrap_or((0, 0, 1920, 1080));

    let x = (cursor_x - w / 2).clamp(mx, mx + mw - w);
    let y = (cursor_y + 20).clamp(my, my + mh - h);

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
) -> bool {
    use gtk4_layer_shell::{Edge, Layer, LayerShell};

    if !gtk4_layer_shell::is_supported() {
        debug!("gtk4-layer-shell not supported by compositor, falling back");
        return false;
    }

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    // Do NOT grab keyboard — the physical keyboard is already grabbed via evdev.
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);

    // Anchor the top-left corner to position (x, y) from the screen origin.
    window.set_anchor(Edge::Left,   true);
    window.set_anchor(Edge::Top,    true);
    window.set_anchor(Edge::Right,  false);
    window.set_anchor(Edge::Bottom, false);

    // Estimate popup width to horizontally centre over cursor.
    // Each character box ≈ 62 px wide plus 16 px total padding.
    let est_w = n_accents as i32 * 62 + 16;
    let x = (cursor_x - est_w / 2).max(0);
    let y = cursor_y + 20; // appear just below the text cursor

    window.set_margin(Edge::Left, x);
    window.set_margin(Edge::Top,  y);

    debug!("layer-shell popup at ({x},{y})");
    true
}

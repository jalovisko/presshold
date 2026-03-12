# presshold

A macOS-style accent character selector for Linux, implemented as a systemd user service.

Hold any letter key that has accent variants (e.g. `e`) and a popup appears with the available accented characters. Navigate with the keyboard and confirm your choice. The accent is typed into whatever application had focus.

```
┌──────────────────────────┐
│  é  è  ē  ĕ  ê  ě  ë  ę  │
│  1  2  3  4  5  6  7  8  │
└──────────────────────────┘
```

## 1. Interaction

| Input | Action |
|-------|--------|
| Hold letter key | Show popup |
| Same letter again | Cycle to next accent |
| `←` / `→` | Move selection |
| `1`–`9` | Pick by number |
| `Enter` / `Space` | Confirm selection |
| `Esc` | Cancel (type original character) |
| Release the held key | Confirm current selection |

## 2. Requirements

### 2.1. Rust toolchain

```bash
# Any distro, install via rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Or via your package manager:

| Distro | Command |
|--------|---------|
| Arch / Manjaro | `sudo pacman -S rustup && rustup default stable` |
| Debian / Ubuntu | `sudo apt install rustup && rustup default stable` |
| Fedora | `sudo dnf install rust cargo` |
| openSUSE | `sudo zypper install rust cargo` |

### 2.2. System libraries

| Package | Purpose | Arch | Debian/Ubuntu | Fedora |
|---------|---------|------|---------------|--------|
| GTK 4 | Popup window | `pacman -S gtk4` | `apt install libgtk-4-dev` | `dnf install gtk4-devel` |
| gtk4-layer-shell | Wayland popup positioning | `pacman -S gtk4-layer-shell` | `apt install libgtk4-layer-shell-dev` | `dnf install gtk-layer-shell-devel` |
| xdotool | Character injection on X11 | `pacman -S xdotool` | `apt install xdotool` | `dnf install xdotool` |
| wtype | Character injection on Wayland | `pacman -S wtype` | `apt install wtype` | (build from source) |

> You need one of xdotool (X11) or wtype (Wayland) for accent injection to work.
> `gtk4-layer-shell` is optional but strongly recommended on Wayland. Without it the popup
> position is compositor-controlled and may appear in an unexpected location.

### 2.3. User group

presshold reads raw keyboard devices via `/dev/input/event*`.
Your user must be in the **`input`** group:

```bash
sudo usermod -aG input $USER
# Log out and back in (or run: newgrp input) for the change to take effect.
```

## 3. Building

```bash
git clone https://github.com/jalovisko/presshold.git
cd presshold

# With Wayland layer-shell positioning (default, recommended):
cargo build --release

# Without layer-shell (X11 only, or if libgtk4-layer-shell is not installed):
cargo build --release --no-default-features
```

## 4. Installation

```bash
./install.sh
```

The script will:
1. Build a release binary and copy it to `~/.local/bin/presshold`
2. Write a `~/.config/presshold/env` file with your current display variables
3. Install and start the systemd user service

Check the service status:
```bash
systemctl --user status presshold.service
journalctl --user -u presshold.service -f
```

## 5. Supported characters

| Base | Variants |
|------|----------|
| a / A | á à ā ă â å ä ã æ |
| c / C | ç ć č |
| d / D | ð ď |
| e / E | é è ē ĕ ê ě ë ę |
| g / G | ğ |
| i / I | í ì ī ĭ î ï ĩ |
| l / L | ł ľ ĺ |
| n / N | ñ ń ň |
| o / O | ó ò ō ŏ ô ö õ ø œ |
| r / R | ř ŕ |
| s / S | ß š ś ş |
| t / T | þ ť |
| u / U | ú ù ū ŭ û ü ů |
| y / Y | ý ÿ |
| z / Z | ž ź ż |

## 6. How it works

1. presshold exclusively grabs every physical keyboard via the Linux `evdev` API.
2. A uinput virtual keyboard re-emits all events that should pass through to applications.
3. When a letter key is held long enough to trigger the kernel's autorepeat, presshold
   intercepts that event and shows a GTK4 popup.
4. Navigation keys are handled internally; the selected accent is injected via
   **`wtype`** (Wayland) or **`xdotool type`** (X11).
   Synthetic injection events bypass our evdev grab and reach the focused application directly.

## 7. Supported environments

- **X11**: i3, XFCE, KDE Plasma, GNOME, Openbox, awesome, bspwm, ...
- **Wayland**: Sway, Hyprland, KDE Plasma, GNOME, River, ...

Tested on Hyprland. Reports from other environments welcome.

## 8. Uninstalling

```bash
systemctl --user disable --now presshold.service
rm ~/.local/bin/presshold
rm ~/.config/systemd/user/presshold.service
rm -rf ~/.config/presshold
```

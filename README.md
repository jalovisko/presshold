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
| `Esc` | Cancel (type original character) |
| Any other key (`Space`, `Enter`, `.`, another letter, …) | Confirm selection, then type that key as usual |

So `e` held, `e` tapped, `Space` gives `é ` in one go.

### 1.1. Options

| Option | Effect |
|--------|--------|
| `--grave-first` | List the grave accent before the acute one on vowels (`è` before `é`), so languages that mostly use the grave accent, such as Italian, get it with a single tap. |
| `-h`, `--help` | Show usage. |

The same switch can be set from the environment with `PRESSHOLD_GRAVE_FIRST=1`.
The systemd service reads `~/.config/presshold/env`, so putting that line there
enables it without editing the unit.

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
| wtype | Character injection on Wayland (wlroots) | `pacman -S wtype` | `apt install wtype` | (build from source) |
| ydotool | Character injection on GNOME Wayland | `pacman -S ydotool` | `apt install ydotool` | `dnf install ydotool` |

> Which injection tool do you need?
> - X11 (any desktop): `xdotool`
> - Wayland on Sway / Hyprland / wlroots compositors: `wtype`
>
> presshold tries wtype, ydotool, and xdotool, in that order, and uses the first one that succeeds.
> `gtk4-layer-shell` is optional but strongly recommended on wlroots Wayland compositors.
> GNOME Wayland does not support the layer-shell protocol, so the popup will appear in the
> centre of the screen rather than near the cursor. This is a known GNOME limitation.



### 2.3. User group and uinput access

presshold reads raw keyboard devices via `/dev/input/event*` and creates a virtual keyboard via `/dev/uinput`.
Your user must be in the **`input`** group:

```bash
sudo usermod -aG input $USER
# Log out and back in (or run: newgrp input) for the change to take effect.
```

On some distros (e.g. Ubuntu/Debian), `/dev/uinput` is not group-writable by default.
Add a udev rule to fix this:

```bash
echo 'KERNEL=="uinput", GROUP="input", MODE="0660"' | sudo tee /etc/udev/rules.d/99-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then log out and back in.

## 3. Installation

### 3.1. Arch Linux (AUR)

```bash
yay -S presshold
```

Then enable the service:

```bash
systemctl --user enable --now presshold.service
```

### 3.2. Build from source

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
   Keyboards plugged in later are detected with inotify on `/dev/input` and grabbed
   as they appear, so no restart is needed after connecting an external keyboard.
2. A uinput virtual keyboard re-emits all events that should pass through to applications.
3. When a letter key is held long enough to trigger the kernel's autorepeat, presshold
   intercepts that event and shows a GTK4 popup.
4. Navigation keys are handled internally. The selected accent is typed on the
   virtual keyboard as an **XKB Compose sequence** (for example Compose, `` ` ``, `e`
   for `è`) when a Compose key is configured — presshold reads it from Hyprland's
   `input:kb_options` (`compose:caps`, `compose:ralt`, …). To an application this
   is indistinguishable from the physical keyboard, so it also works in Chromium
   and Electron apps, which ignore the throw-away keymaps `wtype` uses.
   Without a Compose key the accent is injected via **`wtype`** (Wayland),
   the GTK Unicode input method (GNOME Wayland) or **`xdotool type`** (X11).
5. Keys pressed while the accent is being typed are held back and replayed right
   after it, so `è.` never comes out as `.è`.

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

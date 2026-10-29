//! Typing characters through the XKB Compose mechanism.
//!
//! `wtype` injects characters that are missing from the keyboard layout by
//! pushing a throw-away keymap to the compositor. Some clients (Chromium,
//! Electron apps) ignore that keymap and drop the character. Typing the
//! Compose sequence on the passthrough uinput device instead is
//! indistinguishable from the physical keyboard, so it works everywhere the
//! user's own Compose key works.

use evdev::Key;
use log::debug;
use std::process::Command;

/// One key press on a US layout, optionally with Shift held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyStroke {
    pub key: Key,
    pub shift: bool,
}

/// Map an XKB options string such as `compose:caps,shift:both_capslock_cancel`
/// to the evdev key that acts as Compose, if any.
pub fn compose_key_from_kb_options(options: &str) -> Option<Key> {
    options.split(',').map(str::trim).find_map(|opt| {
        let name = opt.strip_prefix("compose:")?;
        // "-altgr" variants only add AltGr as an alternative; the main key is the same.
        let name = name.strip_suffix("-altgr").unwrap_or(name);
        match name {
            "caps"  => Some(Key::KEY_CAPSLOCK),
            "ralt"  => Some(Key::KEY_RIGHTALT),
            "lwin"  => Some(Key::KEY_LEFTMETA),
            "rwin"  => Some(Key::KEY_RIGHTMETA),
            "menu"  => Some(Key::KEY_COMPOSE),
            "lctrl" => Some(Key::KEY_LEFTCTRL),
            "rctrl" => Some(Key::KEY_RIGHTCTRL),
            "102"   => Some(Key::KEY_102ND),
            "paus"  => Some(Key::KEY_PAUSE),
            "prsc"  => Some(Key::KEY_SYSRQ),
            "sclk"  => Some(Key::KEY_SCROLLLOCK),
            _ => None,
        }
    })
}

/// Ask the running Hyprland instance which key is configured as Compose.
pub fn detect_compose_key() -> Option<Key> {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let out = Command::new("hyprctl")
        .args(["getoption", "input:kb_options", "-j"])
        .output()
        .ok()?;
    let json = String::from_utf8_lossy(&out.stdout);
    let value = json_string_field(&json, "str")?;
    debug!("Hyprland kb_options: {value:?}");
    compose_key_from_kb_options(&value)
}

/// Minimal extraction of `"field": "value"` from a flat JSON object.
fn json_string_field(json: &str, field: &str) -> Option<String> {
    let key = format!("\"{field}\"");
    let rest = &json[json.find(&key)? + key.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => { if let Some(n) = chars.next() { out.push(n); } }
            '"' => return Some(out),
            _ => out.push(c),
        }
    }
    None
}

/// The key presses that type `ch` on a US layout, using `compose` as the
/// Compose key for characters outside the layout. `None` if we do not know
/// a sequence for it.
pub fn sequence(ch: char, compose: Key) -> Option<Vec<KeyStroke>> {
    if let Some(ks) = ascii_keystroke(ch) {
        return Some(vec![ks]);
    }
    let keysyms = compose_keysyms(ch)?;
    let mut out = vec![KeyStroke { key: compose, shift: false }];
    for sym in keysyms {
        out.push(keysym_keystroke(sym)?);
    }
    Some(out)
}

/// Key for a printable ASCII character on a US layout.
fn ascii_keystroke(ch: char) -> Option<KeyStroke> {
    let plain = |key| Some(KeyStroke { key, shift: false });
    let shifted = |key| Some(KeyStroke { key, shift: true });
    match ch {
        'a'..='z' => plain(letter_key(ch)),
        'A'..='Z' => shifted(letter_key(ch.to_ascii_lowercase())),
        '1' => plain(Key::KEY_1), '2' => plain(Key::KEY_2), '3' => plain(Key::KEY_3),
        '4' => plain(Key::KEY_4), '5' => plain(Key::KEY_5), '6' => plain(Key::KEY_6),
        '7' => plain(Key::KEY_7), '8' => plain(Key::KEY_8), '9' => plain(Key::KEY_9),
        '0' => plain(Key::KEY_0),
        '!' => shifted(Key::KEY_1), '@' => shifted(Key::KEY_2), '#' => shifted(Key::KEY_3),
        '$' => shifted(Key::KEY_4), '%' => shifted(Key::KEY_5), '^' => shifted(Key::KEY_6),
        '&' => shifted(Key::KEY_7), '*' => shifted(Key::KEY_8), '(' => shifted(Key::KEY_9),
        ')' => shifted(Key::KEY_0),
        ' ' => plain(Key::KEY_SPACE),
        '`' => plain(Key::KEY_GRAVE),        '~' => shifted(Key::KEY_GRAVE),
        '-' => plain(Key::KEY_MINUS),        '_' => shifted(Key::KEY_MINUS),
        '=' => plain(Key::KEY_EQUAL),        '+' => shifted(Key::KEY_EQUAL),
        '[' => plain(Key::KEY_LEFTBRACE),    '{' => shifted(Key::KEY_LEFTBRACE),
        ']' => plain(Key::KEY_RIGHTBRACE),   '}' => shifted(Key::KEY_RIGHTBRACE),
        '\\' => plain(Key::KEY_BACKSLASH),   '|' => shifted(Key::KEY_BACKSLASH),
        ';' => plain(Key::KEY_SEMICOLON),    ':' => shifted(Key::KEY_SEMICOLON),
        '\'' => plain(Key::KEY_APOSTROPHE),  '"' => shifted(Key::KEY_APOSTROPHE),
        ',' => plain(Key::KEY_COMMA),        '<' => shifted(Key::KEY_COMMA),
        '.' => plain(Key::KEY_DOT),          '>' => shifted(Key::KEY_DOT),
        '/' => plain(Key::KEY_SLASH),        '?' => shifted(Key::KEY_SLASH),
        _ => None,
    }
}

fn letter_key(ch: char) -> Key {
    match ch {
        'a' => Key::KEY_A, 'b' => Key::KEY_B, 'c' => Key::KEY_C, 'd' => Key::KEY_D,
        'e' => Key::KEY_E, 'f' => Key::KEY_F, 'g' => Key::KEY_G, 'h' => Key::KEY_H,
        'i' => Key::KEY_I, 'j' => Key::KEY_J, 'k' => Key::KEY_K, 'l' => Key::KEY_L,
        'm' => Key::KEY_M, 'n' => Key::KEY_N, 'o' => Key::KEY_O, 'p' => Key::KEY_P,
        'q' => Key::KEY_Q, 'r' => Key::KEY_R, 's' => Key::KEY_S, 't' => Key::KEY_T,
        'u' => Key::KEY_U, 'v' => Key::KEY_V, 'w' => Key::KEY_W, 'x' => Key::KEY_X,
        'y' => Key::KEY_Y, 'z' => Key::KEY_Z,
        _ => unreachable!("letter_key called with {ch:?}"),
    }
}

/// Key for an X keysym name as used in the Compose file.
fn keysym_keystroke(sym: &str) -> Option<KeyStroke> {
    let ch = match sym {
        "grave" => '`', "asciitilde" => '~', "apostrophe" => '\'', "quotedbl" => '"',
        "comma" => ',', "less" => '<', "period" => '.', "greater" => '>',
        "slash" => '/', "question" => '?', "semicolon" => ';', "colon" => ':',
        "minus" => '-', "underscore" => '_', "equal" => '=', "plus" => '+',
        "bracketleft" => '[', "bracketright" => ']', "braceleft" => '{', "braceright" => '}',
        "backslash" => '\\', "bar" => '|', "asciicircum" => '^', "exclam" => '!',
        "at" => '@', "numbersign" => '#', "dollar" => '$', "percent" => '%',
        "ampersand" => '&', "asterisk" => '*', "parenleft" => '(', "parenright" => ')',
        "space" => ' ',
        single => {
            let mut it = single.chars();
            match (it.next(), it.next()) {
                (Some(c), None) => c,
                _ => return None,
            }
        }
    };
    ascii_keystroke(ch)
}

/// Compose sequences (keysyms after the Compose key) for every character the
/// popup offers, taken from the en_US.UTF-8 Compose file shipped with X11.
fn compose_keysyms(ch: char) -> Option<&'static [&'static str]> {
    Some(match ch {
        'á' => &["apostrophe", "a"],
        'à' => &["grave", "a"],
        'ā' => &["underscore", "a"],
        'ă' => &["U", "a"],
        'â' => &["asciicircum", "a"],
        'å' => &["o", "a"],
        'ä' => &["quotedbl", "a"],
        'ã' => &["asciitilde", "a"],
        'æ' => &["a", "e"],
        'ç' => &["comma", "c"],
        'ć' => &["apostrophe", "c"],
        'č' => &["c", "c"],
        'ð' => &["d", "h"],
        'ď' => &["c", "d"],
        'é' => &["apostrophe", "e"],
        'è' => &["grave", "e"],
        'ē' => &["underscore", "e"],
        'ĕ' => &["U", "e"],
        'ê' => &["asciicircum", "e"],
        'ě' => &["c", "e"],
        'ë' => &["quotedbl", "e"],
        'ę' => &["semicolon", "e"],
        'ğ' => &["U", "g"],
        'í' => &["apostrophe", "i"],
        'ì' => &["grave", "i"],
        'ī' => &["underscore", "i"],
        'ĭ' => &["U", "i"],
        'î' => &["asciicircum", "i"],
        'ï' => &["quotedbl", "i"],
        'ĩ' => &["asciitilde", "i"],
        'ł' => &["slash", "l"],
        'ľ' => &["c", "l"],
        'ĺ' => &["apostrophe", "l"],
        'ñ' => &["asciitilde", "n"],
        'ń' => &["apostrophe", "n"],
        'ň' => &["c", "n"],
        'ó' => &["apostrophe", "o"],
        'ò' => &["grave", "o"],
        'ō' => &["underscore", "o"],
        'ŏ' => &["U", "o"],
        'ô' => &["asciicircum", "o"],
        'ö' => &["quotedbl", "o"],
        'õ' => &["asciitilde", "o"],
        'ø' => &["slash", "o"],
        'œ' => &["o", "e"],
        'ř' => &["c", "r"],
        'ŕ' => &["apostrophe", "r"],
        'ß' => &["s", "s"],
        'š' => &["c", "s"],
        'ś' => &["apostrophe", "s"],
        'ş' => &["comma", "s"],
        'þ' => &["t", "h"],
        'ť' => &["c", "t"],
        'ú' => &["apostrophe", "u"],
        'ù' => &["grave", "u"],
        'ū' => &["underscore", "u"],
        'ŭ' => &["U", "u"],
        'û' => &["asciicircum", "u"],
        'ü' => &["quotedbl", "u"],
        'ů' => &["o", "u"],
        'ý' => &["apostrophe", "y"],
        'ÿ' => &["quotedbl", "y"],
        'ž' => &["c", "z"],
        'ź' => &["apostrophe", "z"],
        'ż' => &["period", "z"],
        'Á' => &["apostrophe", "A"],
        'À' => &["grave", "A"],
        'Ā' => &["underscore", "A"],
        'Ă' => &["U", "A"],
        'Â' => &["asciicircum", "A"],
        'Å' => &["o", "A"],
        'Ä' => &["quotedbl", "A"],
        'Ã' => &["asciitilde", "A"],
        'Æ' => &["A", "E"],
        'Ç' => &["comma", "C"],
        'Ć' => &["apostrophe", "C"],
        'Č' => &["c", "C"],
        'Ð' => &["D", "H"],
        'Ď' => &["c", "D"],
        'É' => &["apostrophe", "E"],
        'È' => &["grave", "E"],
        'Ē' => &["underscore", "E"],
        'Ĕ' => &["U", "E"],
        'Ê' => &["asciicircum", "E"],
        'Ě' => &["c", "E"],
        'Ë' => &["quotedbl", "E"],
        'Ę' => &["semicolon", "E"],
        'Ğ' => &["U", "G"],
        'Í' => &["apostrophe", "I"],
        'Ì' => &["grave", "I"],
        'Ī' => &["underscore", "I"],
        'Ĭ' => &["U", "I"],
        'Î' => &["asciicircum", "I"],
        'Ï' => &["quotedbl", "I"],
        'Ĩ' => &["asciitilde", "I"],
        'Ł' => &["slash", "L"],
        'Ľ' => &["c", "L"],
        'Ĺ' => &["apostrophe", "L"],
        'Ñ' => &["asciitilde", "N"],
        'Ń' => &["apostrophe", "N"],
        'Ň' => &["c", "N"],
        'Ó' => &["apostrophe", "O"],
        'Ò' => &["grave", "O"],
        'Ō' => &["underscore", "O"],
        'Ŏ' => &["U", "O"],
        'Ô' => &["asciicircum", "O"],
        'Ö' => &["quotedbl", "O"],
        'Õ' => &["asciitilde", "O"],
        'Ø' => &["slash", "O"],
        'Œ' => &["O", "E"],
        'Ř' => &["c", "R"],
        'Ŕ' => &["apostrophe", "R"],
        'Š' => &["c", "S"],
        'Ś' => &["apostrophe", "S"],
        'Ş' => &["comma", "S"],
        'Þ' => &["T", "H"],
        'Ť' => &["c", "T"],
        'Ú' => &["apostrophe", "U"],
        'Ù' => &["grave", "U"],
        'Ū' => &["underscore", "U"],
        'Ŭ' => &["U", "U"],
        'Û' => &["asciicircum", "U"],
        'Ü' => &["quotedbl", "U"],
        'Ů' => &["o", "U"],
        'Ý' => &["apostrophe", "Y"],
        'Ÿ' => &["quotedbl", "Y"],
        'Ž' => &["c", "Z"],
        'Ź' => &["apostrophe", "Z"],
        'Ż' => &["period", "Z"],
        '–' => &["minus", "minus", "period"],
        '—' => &["minus", "minus", "minus"],
        '−' => &["minus", "underscore"],
        _ => return None,
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use evdev::Key;

    fn k(key: Key) -> KeyStroke { KeyStroke { key, shift: false } }
    fn sk(key: Key) -> KeyStroke { KeyStroke { key, shift: true } }

    #[test]
    fn compose_key_is_read_from_hyprland_kb_options() {
        assert_eq!(compose_key_from_kb_options("compose:caps,shift:both_capslock_cancel"), Some(Key::KEY_CAPSLOCK));
        assert_eq!(compose_key_from_kb_options("grp:alt_shift_toggle, compose:ralt"), Some(Key::KEY_RIGHTALT));
        assert_eq!(compose_key_from_kb_options("compose:menu"), Some(Key::KEY_COMPOSE));
        assert_eq!(compose_key_from_kb_options("compose:rwin"), Some(Key::KEY_RIGHTMETA));
    }

    #[test]
    fn no_compose_option_means_no_compose_key() {
        assert_eq!(compose_key_from_kb_options(""), None);
        assert_eq!(compose_key_from_kb_options("grp:alt_shift_toggle"), None);
        assert_eq!(compose_key_from_kb_options("compose:nosuchkey"), None);
    }

    #[test]
    fn grave_accent_is_compose_grave_letter() {
        assert_eq!(sequence('è', Key::KEY_CAPSLOCK), Some(vec![k(Key::KEY_CAPSLOCK), k(Key::KEY_GRAVE), k(Key::KEY_E)]));
    }

    #[test]
    fn uppercase_acute_uses_shift_on_the_letter_only() {
        assert_eq!(sequence('É', Key::KEY_CAPSLOCK), Some(vec![k(Key::KEY_CAPSLOCK), k(Key::KEY_APOSTROPHE), sk(Key::KEY_E)]));
    }

    #[test]
    fn circumflex_needs_shift_on_the_accent_key() {
        assert_eq!(sequence('ê', Key::KEY_CAPSLOCK), Some(vec![k(Key::KEY_CAPSLOCK), sk(Key::KEY_6), k(Key::KEY_E)]));
    }

    #[test]
    fn ligatures_and_dashes_compose_from_letters() {
        assert_eq!(sequence('ß', Key::KEY_RIGHTALT), Some(vec![k(Key::KEY_RIGHTALT), k(Key::KEY_S), k(Key::KEY_S)]));
        assert_eq!(sequence('—', Key::KEY_RIGHTALT), Some(vec![k(Key::KEY_RIGHTALT), k(Key::KEY_MINUS), k(Key::KEY_MINUS), k(Key::KEY_MINUS)]));
    }

    #[test]
    fn plain_ascii_is_typed_directly_without_compose() {
        assert_eq!(sequence('e', Key::KEY_CAPSLOCK), Some(vec![k(Key::KEY_E)]));
        assert_eq!(sequence('E', Key::KEY_CAPSLOCK), Some(vec![sk(Key::KEY_E)]));
        assert_eq!(sequence('-', Key::KEY_CAPSLOCK), Some(vec![k(Key::KEY_MINUS)]));
    }

    #[test]
    fn every_offered_accent_has_a_compose_sequence() {
        for base in "abcdeghilnorstuyzABCDEGHILNORSTUYZ-".chars() {
            if let Some(vs) = crate::accents::variants(base) {
                for &v in vs {
                    assert!(sequence(v, Key::KEY_CAPSLOCK).is_some(), "no compose sequence for {v:?}");
                }
            }
        }
    }

    #[test]
    fn unknown_character_has_no_sequence() {
        assert_eq!(sequence('😀', Key::KEY_CAPSLOCK), None);
    }
}

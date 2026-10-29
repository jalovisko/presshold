/// Returns the accented variants for a base character, or `None` if none exist.
/// The variants are ordered the same way macOS presents them.
pub fn variants(ch: char) -> Option<&'static [char]> {
    match ch {
        // — lowercase —
        'a' => Some(&['á', 'à', 'ā', 'ă', 'â', 'å', 'ä', 'ã', 'æ']),
        'c' => Some(&['ç', 'ć', 'č']),
        'd' => Some(&['ð', 'ď']),
        'e' => Some(&['é', 'è', 'ē', 'ĕ', 'ê', 'ě', 'ë', 'ę']),
        'g' => Some(&['ğ']),
        'i' => Some(&['í', 'ì', 'ī', 'ĭ', 'î', 'ï', 'ĩ']),
        'l' => Some(&['ł', 'ľ', 'ĺ']),
        'n' => Some(&['ñ', 'ń', 'ň']),
        'o' => Some(&['ó', 'ò', 'ō', 'ŏ', 'ô', 'ö', 'õ', 'ø', 'œ']),
        'r' => Some(&['ř', 'ŕ']),
        's' => Some(&['ß', 'š', 'ś', 'ş']),
        't' => Some(&['þ', 'ť']),
        'u' => Some(&['ú', 'ù', 'ū', 'ŭ', 'û', 'ü', 'ů']),
        'y' => Some(&['ý', 'ÿ']),
        'z' => Some(&['ž', 'ź', 'ż']),
        // — uppercase —
        'A' => Some(&['Á', 'À', 'Ā', 'Ă', 'Â', 'Å', 'Ä', 'Ã', 'Æ']),
        'C' => Some(&['Ç', 'Ć', 'Č']),
        'D' => Some(&['Ð', 'Ď']),
        'E' => Some(&['É', 'È', 'Ē', 'Ĕ', 'Ê', 'Ě', 'Ë', 'Ę']),
        'G' => Some(&['Ğ']),
        'I' => Some(&['Í', 'Ì', 'Ī', 'Ĭ', 'Î', 'Ï', 'Ĩ']),
        'L' => Some(&['Ł', 'Ľ', 'Ĺ']),
        'N' => Some(&['Ñ', 'Ń', 'Ň']),
        'O' => Some(&['Ó', 'Ò', 'Ō', 'Ŏ', 'Ô', 'Ö', 'Õ', 'Ø', 'Œ']),
        'R' => Some(&['Ř', 'Ŕ']),
        'S' => Some(&['Š', 'Ś', 'Ş']),
        'T' => Some(&['Þ', 'Ť']),
        'U' => Some(&['Ú', 'Ù', 'Ū', 'Ŭ', 'Û', 'Ü', 'Ů']),
        'Y' => Some(&['Ý', 'Ÿ']),
        'Z' => Some(&['Ž', 'Ź', 'Ż']),
        // - punctuation -
        '-' => Some(&['\u{2013}', '\u{2014}', '\u{2212}']), // – — −
        _ => None,
    }
}

pub fn has_variants(ch: char) -> bool {
    variants(ch).is_some()
}

/// In which order the variants of a vowel are offered.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AccentOrder {
    /// As macOS: acute first (`é` before `è`).
    MacOs,
    /// Grave first (`è` before `é`), so languages that mostly use the grave
    /// accent, such as Italian, get it with a single tap.
    GraveFirst,
}

impl AccentOrder {
    /// `--grave-first` on the command line, or `PRESSHOLD_GRAVE_FIRST` set to
    /// anything but `0`, `false` or empty in the environment.
    pub fn from(args: &[String], env: Option<&str>) -> Self {
        let flag = args.iter().skip(1).any(|a| a == "--grave-first");
        let env = env.map_or(false, |v| !matches!(v.trim(), "" | "0" | "false" | "no"));
        if flag || env { AccentOrder::GraveFirst } else { AccentOrder::MacOs }
    }
}

/// Like [`variants`], but in the requested order.  The macOS table lists the
/// acute accent first and the grave one second for every vowel, so the
/// grave-first order is that table with the first two entries swapped.
pub fn variants_in(ch: char, order: AccentOrder) -> Option<&'static [char]> {
    if order == AccentOrder::MacOs {
        return variants(ch);
    }
    let swapped: &'static [char] = match ch {
        'a' => &['à', 'á', 'ā', 'ă', 'â', 'å', 'ä', 'ã', 'æ'],
        'e' => &['è', 'é', 'ē', 'ĕ', 'ê', 'ě', 'ë', 'ę'],
        'i' => &['ì', 'í', 'ī', 'ĭ', 'î', 'ï', 'ĩ'],
        'o' => &['ò', 'ó', 'ō', 'ŏ', 'ô', 'ö', 'õ', 'ø', 'œ'],
        'u' => &['ù', 'ú', 'ū', 'ŭ', 'û', 'ü', 'ů'],
        'A' => &['À', 'Á', 'Ā', 'Ă', 'Â', 'Å', 'Ä', 'Ã', 'Æ'],
        'E' => &['È', 'É', 'Ē', 'Ĕ', 'Ê', 'Ě', 'Ë', 'Ę'],
        'I' => &['Ì', 'Í', 'Ī', 'Ĭ', 'Î', 'Ï', 'Ĩ'],
        'O' => &['Ò', 'Ó', 'Ō', 'Ŏ', 'Ô', 'Ö', 'Õ', 'Ø', 'Œ'],
        'U' => &['Ù', 'Ú', 'Ū', 'Ŭ', 'Û', 'Ü', 'Ů'],
        _ => return variants(ch),
    };
    debug_assert_eq!(swapped[2..], variants(ch).unwrap()[2..], "grave-first table drifted from the macOS one");
    Some(swapped)
}

#[cfg(test)]
mod tests {
    use super::{variants, variants_in, AccentOrder};

    #[test]
    fn default_order_follows_macos_with_acute_first() {
        assert_eq!(variants('a').unwrap()[..2], ['á', 'à']);
        assert_eq!(variants('e').unwrap()[..2], ['é', 'è']);
        assert_eq!(variants_in('e', AccentOrder::MacOs).unwrap()[..2], ['é', 'è']);
        assert_eq!(variants_in('E', AccentOrder::MacOs).unwrap()[..2], ['É', 'È']);
    }

    #[test]
    fn grave_first_puts_the_grave_accent_first_on_vowels() {
        // One tap of the same letter lands on the grave accent (Italian: à è ì ò ù).
        for (ch, expect) in [('a', ['à', 'á']), ('e', ['è', 'é']), ('i', ['ì', 'í']), ('o', ['ò', 'ó']), ('u', ['ù', 'ú'])] {
            assert_eq!(variants_in(ch, AccentOrder::GraveFirst).unwrap()[..2], expect, "{ch}");
        }
        assert_eq!(variants_in('E', AccentOrder::GraveFirst).unwrap()[..2], ['È', 'É']);
        assert_eq!(variants_in('O', AccentOrder::GraveFirst).unwrap()[..2], ['Ò', 'Ó']);
    }

    #[test]
    fn grave_first_only_swaps_the_first_two_and_leaves_other_letters_alone() {
        assert_eq!(variants_in('e', AccentOrder::GraveFirst).unwrap()[2..], variants('e').unwrap()[2..]);
        assert_eq!(variants_in('n', AccentOrder::GraveFirst), variants('n'));
        assert_eq!(variants_in('y', AccentOrder::GraveFirst), variants('y'));
        assert_eq!(variants_in('-', AccentOrder::GraveFirst), variants('-'));
        assert_eq!(variants_in('x', AccentOrder::GraveFirst), None);
    }

    #[test]
    fn order_is_read_from_flag_or_environment() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(AccentOrder::from(&args(&["presshold"]), None), AccentOrder::MacOs);
        assert_eq!(AccentOrder::from(&args(&["presshold", "--grave-first"]), None), AccentOrder::GraveFirst);
        assert_eq!(AccentOrder::from(&args(&["presshold"]), Some("1")), AccentOrder::GraveFirst);
        assert_eq!(AccentOrder::from(&args(&["presshold"]), Some("true")), AccentOrder::GraveFirst);
        assert_eq!(AccentOrder::from(&args(&["presshold"]), Some("0")), AccentOrder::MacOs);
        assert_eq!(AccentOrder::from(&args(&["presshold"]), Some("")), AccentOrder::MacOs);
    }
}

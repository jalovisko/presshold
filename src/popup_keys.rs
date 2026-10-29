//! What a key press means while the accent popup is open.

use evdev::Key;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupAction {
    MoveLeft,
    MoveRight,
    /// Choose the item at this index (0 = base letter) and close.
    Pick(usize),
    /// The trigger letter again: advance the selection.
    Cycle,
    /// Escape: type the plain base letter.
    Cancel,
    /// Anything else: type the selected character, then let the key through.
    ConfirmThenPassthrough,
}

pub fn popup_action(key: Key, same_letter: bool) -> PopupAction {
    match key {
        Key::KEY_LEFT => PopupAction::MoveLeft,
        Key::KEY_RIGHT => PopupAction::MoveRight,
        Key::KEY_1 => PopupAction::Pick(0), Key::KEY_2 => PopupAction::Pick(1),
        Key::KEY_3 => PopupAction::Pick(2), Key::KEY_4 => PopupAction::Pick(3),
        Key::KEY_5 => PopupAction::Pick(4), Key::KEY_6 => PopupAction::Pick(5),
        Key::KEY_7 => PopupAction::Pick(6), Key::KEY_8 => PopupAction::Pick(7),
        Key::KEY_9 => PopupAction::Pick(8),
        Key::KEY_ESC => PopupAction::Cancel,
        _ if same_letter => PopupAction::Cycle,
        _ => PopupAction::ConfirmThenPassthrough,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::Key;

    #[test]
    fn space_and_enter_confirm_and_are_typed_afterwards() {
        assert_eq!(popup_action(Key::KEY_SPACE, false), PopupAction::ConfirmThenPassthrough);
        assert_eq!(popup_action(Key::KEY_ENTER, false), PopupAction::ConfirmThenPassthrough);
        assert_eq!(popup_action(Key::KEY_KPENTER, false), PopupAction::ConfirmThenPassthrough);
    }

    #[test]
    fn punctuation_and_other_letters_confirm_and_are_typed_afterwards() {
        assert_eq!(popup_action(Key::KEY_DOT, false), PopupAction::ConfirmThenPassthrough);
        assert_eq!(popup_action(Key::KEY_A, false), PopupAction::ConfirmThenPassthrough);
        assert_eq!(popup_action(Key::KEY_LEFTSHIFT, false), PopupAction::ConfirmThenPassthrough);
    }

    #[test]
    fn same_letter_cycles() {
        assert_eq!(popup_action(Key::KEY_E, true), PopupAction::Cycle);
    }

    #[test]
    fn digits_pick_directly_and_escape_cancels() {
        assert_eq!(popup_action(Key::KEY_1, false), PopupAction::Pick(0));
        assert_eq!(popup_action(Key::KEY_9, false), PopupAction::Pick(8));
        assert_eq!(popup_action(Key::KEY_ESC, false), PopupAction::Cancel);
    }

    #[test]
    fn arrows_move_the_selection() {
        assert_eq!(popup_action(Key::KEY_LEFT, false), PopupAction::MoveLeft);
        assert_eq!(popup_action(Key::KEY_RIGHT, false), PopupAction::MoveRight);
    }
}

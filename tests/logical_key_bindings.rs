use tuika::event::{Key, KeyCode};
use tuika::keymap::{Dispatch, Keymap, Layer};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Lower,
    Upper,
}

#[test]
fn public_keymap_matches_exact_layout_translated_characters() {
    let mut keymap = Keymap::new().layer(
        Layer::new("global")
            .bind("ctrl+r", Action::Lower)
            .bind("ctrl+R", Action::Upper),
    );

    let shifted = Key {
        code: KeyCode::Char('R'),
        ctrl: true,
        alt: false,
        // Some hosts retain this bit and some enhanced protocols fold it into
        // the character. Both normalize to the same logical-text chord.
        shift: true,
    };
    assert_eq!(keymap.dispatch(shifted), Dispatch::Command(Action::Upper));
    assert_eq!(
        keymap.dispatch(Key {
            code: KeyCode::Char('r'),
            ctrl: true,
            alt: false,
            shift: false,
        }),
        Dispatch::Command(Action::Lower)
    );

    let hints = keymap.hints();
    assert_eq!(hints[0].keys, "Ctrl+R");
    assert_eq!(hints[1].keys, "Ctrl+Shift+R");
}

#[test]
fn physical_shift_character_specs_fail_fallibly() {
    let error = Layer::new("global")
        .try_bind("shift+/", Action::Upper)
        .unwrap_err();
    assert!(error.to_string().contains("logical text"));

    // The logical result is portable and parses directly, including Unicode.
    assert!(Layer::new("global").try_bind("?", Action::Upper).is_ok());
    assert!(Layer::new("global").try_bind("ж", Action::Upper).is_ok());
    // Shift remains a real modifier for non-character keys.
    assert!(
        Layer::new("global")
            .try_bind("shift+enter", Action::Upper)
            .is_ok()
    );
}

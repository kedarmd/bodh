/// Return printable text from a key-down event without losing the platform's
/// produced character (for example, Shift+N yields `N` even when `key` is `n`).
pub fn typed_text<'a>(
    key: &'a str,
    key_char: Option<&'a str>,
    control: bool,
    platform: bool,
) -> Option<&'a str> {
    if control || platform {
        return None;
    }
    let text = key_char.unwrap_or(key);
    (text.chars().count() == 1).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::typed_text;

    #[test]
    fn shifted_repository_letters_keep_the_typed_case() {
        assert_eq!(typed_text("n", Some("N"), false, false), Some("N"));
    }

    #[test]
    fn modified_shortcuts_are_not_inserted_as_path_text() {
        assert_eq!(typed_text("n", Some("n"), true, false), None);
        assert_eq!(typed_text("n", Some("n"), false, true), None);
    }
}

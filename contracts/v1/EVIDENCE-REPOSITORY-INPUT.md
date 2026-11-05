# Repository path input and folder picker evidence

## Scope

The home screen accepts an existing Git checkout. This change preserves the user-approved D2 home-screen styling and adds a folder-picker action within the existing repository entry row.

## Changes

- Repository-path input now inserts GPUI's produced `key_char`, preserving Shift and keyboard-layout output such as `N` from physical key `n`. Control and platform shortcut chords remain excluded.
- `Choose Folder` opens GPUI's native directory prompt, puts the selected folder in the path field, and leaves repository validation/opening to the existing `Open Repository` action. Typing remains available as a fallback.
- If the platform folder dialog fails, the home screen shows the error. Cancelling leaves the current path unchanged.

## Automated evidence

- Regression: `repository_input::tests::shifted_repository_letters_keep_the_typed_case` asserts physical key `n` plus produced character `N` yields `N`.
- Shortcut coverage: `repository_input::tests::modified_shortcuts_are_not_inserted_as_path_text` rejects control/platform chords.
- Targeted command passed:

  ```text
  cargo test --locked --no-default-features --manifest-path spikes/native-sequence/Cargo.toml --lib repository_input::tests::shifted_repository_letters_keep_the_typed_case
  ```

- Full desktop validation passed with the native GPUI target. Raw runner output: [VALIDATE-REPOSITORY-INPUT.log](VALIDATE-REPOSITORY-INPUT.log).
- `cargo fmt --manifest-path spikes/native-sequence/Cargo.toml -- --check` and `git diff --check` passed.

## Native trial

The GUI runner is building the release app for an interactive trial. Native interaction acceptance is pending user feedback; process startup alone will not be recorded as input acceptance.

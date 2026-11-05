use bodh::CodeReplacementDraft;
use bodh::{DraftCounts, DraftProvenance, DraftSaveResult, ManualTextDraft, Revision, SourceRef};
use bodh_sequence_spike::{
    CodeReplacementEditor, DiffLineKind, MarkdownBlockKind, SyntaxTokenKind, TextDraftEditor,
    diff_rows, safe_markdown_blocks, split_diff_rows, syntax_spans,
};

#[test]
fn replacement_editor_restores_saved_text_and_keeps_preview_text_after_stale_save() {
    let draft = CodeReplacementDraft {
        id: "rep-1".into(),
        source: SourceRef {
            path: "src/lib.rs".into(),
            revision: Revision::Head,
            start_line: 4,
            end_line: 5,
        },
        original_text: "old();\n".into(),
        replacement: "new();\n".into(),
        revision: 2,
        provenance: DraftProvenance {
            preparation_id: "prep".into(),
            context_id: "ctx".into(),
            base_sha: "base".into(),
            head_sha: "head".into(),
            merge_base_sha: "merge".into(),
        },
        created_at: 1,
        updated_at: 2,
    };
    let mut editor = CodeReplacementEditor::resume(&draft, 7);
    assert!(!editor.is_dirty());
    editor.set_replacement("updated();\n".into());
    assert!(editor.is_dirty());
    editor.save_failed(
        "WORKSPACE_REVISION_STALE".into(),
        "reload before saving".into(),
    );
    assert_eq!(editor.replacement, "updated();\n");
    assert_eq!(editor.original_text, "old();\n");
    editor.move_cursor_to_edge(false);
    editor.insert_text("// ");
    assert_eq!(editor.replacement, "// updated();\n");
}

#[test]
fn unified_diff_rows_track_both_side_lines_and_distinguish_changes() {
    let rows = diff_rows(
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -4,3 +4,4 @@ fn render() {\n context();\n-old_call();\n+new_call();\n+extra_call();\n }\n\\ No newline at end of file\n",
    );
    let hunk = rows
        .iter()
        .find(|row| row.kind == DiffLineKind::Hunk)
        .unwrap();
    assert_eq!(hunk.content, "@@ -4,3 +4,4 @@ fn render() {");
    assert_eq!(rows[4].kind, DiffLineKind::Context);
    assert_eq!((rows[4].old_line, rows[4].new_line), (Some(4), Some(4)));
    assert_eq!(rows[5].kind, DiffLineKind::Deletion);
    assert_eq!((rows[5].old_line, rows[5].new_line), (Some(5), None));
    assert_eq!(rows[6].kind, DiffLineKind::Addition);
    assert_eq!((rows[6].old_line, rows[6].new_line), (None, Some(5)));
    assert_eq!(rows[7].kind, DiffLineKind::Addition);
    assert_eq!((rows[7].old_line, rows[7].new_line), (None, Some(6)));
    assert_eq!(rows[8].kind, DiffLineKind::Context);
    assert_eq!((rows[8].old_line, rows[8].new_line), (Some(6), Some(7)));
    assert_eq!(rows[9].kind, DiffLineKind::Meta);
}

#[test]
fn split_diff_pairs_replacements_and_keeps_side_specific_context_numbers() {
    let rows =
        diff_rows("@@ -4,3 +4,4 @@\n context\n-old_call();\n+new_call();\n+extra_call();\n end\n");
    let split = split_diff_rows(&rows);
    let replacement = split
        .iter()
        .find(|row| {
            row.left
                .as_ref()
                .is_some_and(|line| line.content == "old_call();")
                && row
                    .right
                    .as_ref()
                    .is_some_and(|line| line.content == "new_call();")
        })
        .unwrap();
    assert_eq!(replacement.left.as_ref().unwrap().old_line, Some(5));
    assert_eq!(replacement.right.as_ref().unwrap().new_line, Some(5));
    let addition = split
        .iter()
        .find(|row| {
            row.right
                .as_ref()
                .is_some_and(|line| line.content == "extra_call();")
        })
        .unwrap();
    assert!(addition.left.is_none());
    assert_eq!(addition.right.as_ref().unwrap().new_line, Some(6));
    let context = split
        .iter()
        .find(|row| {
            row.left
                .as_ref()
                .is_some_and(|line| line.content == "context")
        })
        .unwrap();
    assert_eq!(context.left.as_ref().unwrap().old_line, Some(4));
    assert_eq!(context.right.as_ref().unwrap().new_line, Some(4));
}

#[test]
fn full_markdown_becomes_safe_readable_blocks_with_inert_html_and_links() {
    let blocks = safe_markdown_blocks(
        "# Release\n\nRead [docs](javascript:alert(1)) and **keep** this.\n\n- first\n- second\n\n```rust\nfn main() {}\n```\n\n<script>alert('no execution')</script>\n",
    );
    assert_eq!(blocks[0].kind, MarkdownBlockKind::Heading(1));
    assert_eq!(blocks[0].text, "Release");
    let paragraph = blocks
        .iter()
        .find(|block| block.kind == MarkdownBlockKind::Paragraph)
        .unwrap();
    assert_eq!(
        paragraph.text,
        "Read docs (javascript:alert(1)) and keep this."
    );
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block.kind == MarkdownBlockKind::ListItem)
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>(),
        ["• first", "• second"]
    );
    let code = blocks
        .iter()
        .find(|block| block.kind == MarkdownBlockKind::Code("rust".into()))
        .unwrap();
    assert_eq!(code.text, "fn main() {}\n");
    let html = blocks
        .iter()
        .find(|block| block.kind == MarkdownBlockKind::PlainHtml)
        .unwrap();
    assert!(html.text.contains("<script>"));
    assert!(html.text.contains("alert('no execution')"));
    // The model contains only inert text; destinations never become actions.
    assert!(blocks.iter().all(|block| block.action_url.is_none()));
}

#[test]
fn syntax_highlights_common_code_tokens_and_leaves_unknown_languages_plain() {
    let line = "let result = \"safe\"; // visible comment";
    let spans = syntax_spans(line, "src/main.rs");
    assert!(spans.iter().any(|span| {
        span.kind == SyntaxTokenKind::Keyword && &line[span.start..span.end] == "let"
    }));
    assert!(spans.iter().any(|span| {
        span.kind == SyntaxTokenKind::String && &line[span.start..span.end] == "\"safe\""
    }));
    assert!(spans.iter().any(|span| {
        span.kind == SyntaxTokenKind::Comment && &line[span.start..span.end] == "// visible comment"
    }));
    assert!(
        syntax_spans("val value = 18", "unknown")
            .iter()
            .any(|span| {
                span.kind == SyntaxTokenKind::Number && span.start == 12 && span.end == 14
            })
    );
}

#[test]
fn failed_save_retains_typed_text_and_success_keeps_newer_keystrokes() {
    let source = SourceRef {
        path: "src/lib.rs".into(),
        revision: Revision::Head,
        start_line: 8,
        end_line: 8,
    };
    let mut editor = TextDraftEditor::new(source.clone(), 4);
    editor.set_body("Retain this typed review note.".into());
    editor.save_failed(
        "WORKSPACE_REVISION_STALE",
        "Workspace changed elsewhere; reload before saving this draft",
    );
    assert_eq!(editor.body, "Retain this typed review note.");
    assert!(editor.is_dirty());
    assert!(editor.can_save());
    assert!(
        editor
            .error
            .as_deref()
            .unwrap()
            .contains("Workspace changed")
    );
    assert_eq!(
        editor.error_code.as_deref(),
        Some("WORKSPACE_REVISION_STALE")
    );

    let submitted = editor.body.clone();
    editor.set_body("Retain this typed review note, with another thought.".into());
    let persisted = ManualTextDraft {
        id: "d-1".into(),
        body: submitted.clone(),
        source,
        revision: 1,
        provenance: DraftProvenance {
            preparation_id: "prep-1".into(),
            context_id: "context-1".into(),
            base_sha: "a".repeat(40),
            head_sha: "b".repeat(40),
            merge_base_sha: "a".repeat(40),
        },
        created_at: 1,
        updated_at: 2,
    };
    let result = DraftSaveResult {
        workspace_revision: 5,
        counts: DraftCounts {
            inline_comments: 1,
            replies: 0,
            thread_intents: 0,
            code_replacements: 0,
            review_comments: 0,
        },
        draft: persisted,
    };
    editor.save_succeeded(&result, &submitted);
    assert_eq!(editor.draft_id.as_deref(), Some("d-1"));
    assert_eq!(editor.draft_revision, 1);
    assert_eq!(editor.workspace_revision, 5);
    assert_eq!(
        editor.body,
        "Retain this typed review note, with another thought."
    );
    assert!(
        editor.is_dirty(),
        "keystrokes typed while saving remain unsaved"
    );
    assert!(editor.error.is_none());
}

#[test]
fn stale_draft_can_be_recovered_as_a_new_copy_without_overwriting_the_winner() {
    let source = SourceRef {
        path: "src/lib.rs".into(),
        revision: Revision::Head,
        start_line: 3,
        end_line: 3,
    };
    let persisted = ManualTextDraft {
        id: "d-3".into(),
        body: "Latest persisted body".into(),
        source: source.clone(),
        revision: 2,
        provenance: DraftProvenance {
            preparation_id: "prep-3".into(),
            context_id: "context-3".into(),
            base_sha: "e".repeat(40),
            head_sha: "f".repeat(40),
            merge_base_sha: "e".repeat(40),
        },
        created_at: 1,
        updated_at: 2,
    };
    let mut editor = TextDraftEditor::resume(&persisted, 10);
    editor.set_body("Stale local text I need to recover".into());
    editor.save_failed("DRAFT_REVISION_STALE", "A newer edit exists");
    editor.recover_as_new(11);
    assert_eq!(editor.draft_id, None);
    assert_eq!(editor.draft_revision, 0);
    assert_eq!(editor.workspace_revision, 11);
    assert_eq!(editor.body, "Stale local text I need to recover");
    assert!(editor.is_dirty());
    assert!(editor.can_save());
}

#[test]
fn restored_draft_is_clean_until_edited_and_new_draft_discard_is_local() {
    let source = SourceRef {
        path: "src/lib.rs".into(),
        revision: Revision::Base,
        start_line: 7,
        end_line: 8,
    };
    let draft = ManualTextDraft {
        id: "d-2".into(),
        body: "A saved comment".into(),
        source: source.clone(),
        revision: 3,
        provenance: DraftProvenance {
            preparation_id: "prep-2".into(),
            context_id: "context-2".into(),
            base_sha: "c".repeat(40),
            head_sha: "d".repeat(40),
            merge_base_sha: "c".repeat(40),
        },
        created_at: 1,
        updated_at: 2,
    };
    let mut resumed = TextDraftEditor::resume(&draft, 8);
    assert!(!resumed.is_dirty());
    resumed.set_body("Edited locally".into());
    assert!(resumed.is_dirty());

    let mut new_draft = TextDraftEditor::new(source, 8);
    new_draft.set_body("Unsaved note".into());
    new_draft.clear_new_draft();
    assert!(new_draft.body.is_empty());
    assert_eq!(new_draft.draft_id, None);
}

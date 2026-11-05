use bodh_sequence_spike::{
    DiffLineKind, MarkdownBlockKind, SyntaxTokenKind, diff_rows, safe_markdown_blocks,
    split_diff_rows, syntax_spans,
};

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

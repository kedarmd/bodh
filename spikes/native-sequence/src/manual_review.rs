use bodh::{DraftSaveResult, ManualTextDraft, SourceRef};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Header,
    Hunk,
    Context,
    Addition,
    Deletion,
    Meta,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitDiffRow {
    pub left: Option<DiffLine>,
    pub right: Option<DiffLine>,
}

/// Local editor state is retained when persistence reports a stale revision or I/O error.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDraftEditor {
    pub draft_id: Option<String>,
    pub source: SourceRef,
    pub body: String,
    pub cursor: usize,
    pub select_all: bool,
    pub workspace_revision: u64,
    pub draft_revision: u64,
    pub error_code: Option<String>,
    pub error: Option<String>,
    saved_body: Option<String>,
}

impl TextDraftEditor {
    pub fn new(source: SourceRef, workspace_revision: u64) -> Self {
        Self {
            draft_id: None,
            source,
            body: String::new(),
            cursor: 0,
            select_all: false,
            workspace_revision,
            draft_revision: 0,
            error_code: None,
            error: None,
            saved_body: None,
        }
    }

    pub fn resume(draft: &ManualTextDraft, workspace_revision: u64) -> Self {
        Self {
            draft_id: Some(draft.id.clone()),
            source: draft.source.clone(),
            body: draft.body.clone(),
            cursor: draft.body.len(),
            select_all: false,
            workspace_revision,
            draft_revision: draft.revision,
            error_code: None,
            error: None,
            saved_body: Some(draft.body.clone()),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_body.as_deref() != Some(self.body.as_str())
    }

    pub fn can_save(&self) -> bool {
        self.is_dirty() && !self.body.trim().is_empty()
    }

    pub fn set_body(&mut self, body: String) {
        self.body = body;
        self.cursor = self.body.len();
        self.select_all = false;
        self.error_code = None;
        self.error = None;
    }

    pub fn insert_text(&mut self, text: &str) {
        if self.select_all {
            self.body.clear();
            self.cursor = 0;
            self.select_all = false;
        }
        self.cursor = self.cursor.min(self.body.len());
        while !self.body.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
        self.body.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.error_code = None;
        self.error = None;
    }

    pub fn backspace(&mut self) {
        if self.select_all {
            self.body.clear();
            self.cursor = 0;
            self.select_all = false;
        } else if self.cursor > 0 {
            let mut previous = self.cursor - 1;
            while !self.body.is_char_boundary(previous) {
                previous -= 1;
            }
            self.body.replace_range(previous..self.cursor, "");
            self.cursor = previous;
        }
        self.error_code = None;
        self.error = None;
    }

    pub fn delete_forward(&mut self) {
        if self.select_all {
            self.body.clear();
            self.cursor = 0;
            self.select_all = false;
        } else if self.cursor < self.body.len() {
            let mut next = self.cursor + 1;
            while !self.body.is_char_boundary(next) {
                next += 1;
            }
            self.body.replace_range(self.cursor..next, "");
        }
        self.error_code = None;
        self.error = None;
    }

    pub fn move_cursor(&mut self, direction: i32) {
        self.select_all = false;
        if direction < 0 && self.cursor > 0 {
            let mut previous = self.cursor - 1;
            while !self.body.is_char_boundary(previous) {
                previous -= 1;
            }
            self.cursor = previous;
        } else if direction > 0 && self.cursor < self.body.len() {
            let mut next = self.cursor + 1;
            while !self.body.is_char_boundary(next) {
                next += 1;
            }
            self.cursor = next;
        }
    }

    pub fn move_cursor_to_edge(&mut self, end: bool) {
        self.cursor = if end { self.body.len() } else { 0 };
        self.select_all = false;
    }

    pub fn select_all(&mut self) {
        self.select_all = true;
        self.cursor = 0;
    }

    pub fn display_text_with_cursor(&self) -> String {
        if self.select_all {
            return format!("[{}]", self.body);
        }
        format!(
            "{}|{}",
            &self.body[..self.cursor],
            &self.body[self.cursor..]
        )
    }

    pub fn save_succeeded(&mut self, result: &DraftSaveResult, submitted_body: &str) {
        self.draft_id = Some(result.draft.id.clone());
        self.source = result.draft.source.clone();
        self.workspace_revision = result.workspace_revision;
        self.draft_revision = result.draft.revision;
        self.saved_body = Some(submitted_body.to_owned());
        self.error_code = None;
        self.error = None;
    }

    pub fn save_failed(&mut self, code: impl Into<String>, message: impl Into<String>) {
        self.error_code = Some(code.into());
        self.error = Some(message.into());
    }

    /// Keeps the conflicted editor body while giving it a new draft identity/revision base.
    pub fn recover_as_new(&mut self, current_workspace_revision: u64) {
        self.draft_id = None;
        self.draft_revision = 0;
        self.workspace_revision = current_workspace_revision;
        self.saved_body = None;
        self.error_code = None;
        self.error = None;
    }

    pub fn discard_succeeded(&mut self, workspace_revision: u64) {
        self.workspace_revision = workspace_revision;
        self.draft_id = None;
        self.draft_revision = 0;
        self.body.clear();
        self.cursor = 0;
        self.select_all = false;
        self.saved_body = None;
        self.error_code = None;
        self.error = None;
    }

    pub fn clear_new_draft(&mut self) {
        if self.draft_id.is_none() {
            self.body.clear();
            self.cursor = 0;
            self.select_all = false;
            self.saved_body = None;
            self.error_code = None;
            self.error = None;
        }
    }
}

/// Converts a bounded Git unified patch into rows with exact base/head line numbers.
pub fn diff_rows(patch: &str) -> Vec<DiffLine> {
    let mut rows = Vec::new();
    let (mut old, mut new) = (None, None);
    for line in patch.lines() {
        if line.starts_with("@@ ") {
            let (old_start, new_start) = hunk_starts(line).unwrap_or((None, None));
            old = old_start;
            new = new_start;
            rows.push(DiffLine {
                kind: DiffLineKind::Hunk,
                old_line: None,
                new_line: None,
                content: line.to_owned(),
            });
            continue;
        }
        if line.starts_with("\\ No newline at end of file") {
            rows.push(DiffLine {
                kind: DiffLineKind::Meta,
                old_line: None,
                new_line: None,
                content: line.to_owned(),
            });
            continue;
        }
        if old.is_none() || new.is_none() {
            rows.push(DiffLine {
                kind: DiffLineKind::Header,
                old_line: None,
                new_line: None,
                content: line.to_owned(),
            });
            continue;
        }
        let (kind, content, old_line, new_line) = match line.as_bytes().first().copied() {
            Some(b' ') => {
                let pair = (old, new);
                old = old.and_then(|v| v.checked_add(1));
                new = new.and_then(|v| v.checked_add(1));
                (DiffLineKind::Context, &line[1..], pair.0, pair.1)
            }
            Some(b'-') => {
                let old_line = old;
                old = old.and_then(|v| v.checked_add(1));
                (DiffLineKind::Deletion, &line[1..], old_line, None)
            }
            Some(b'+') => {
                let new_line = new;
                new = new.and_then(|v| v.checked_add(1));
                (DiffLineKind::Addition, &line[1..], None, new_line)
            }
            _ => (DiffLineKind::Meta, line, None, None),
        };
        rows.push(DiffLine {
            kind,
            old_line,
            new_line,
            content: content.to_owned(),
        });
    }
    rows
}

/// Pairs adjacent deletions and additions into split-view rows while preserving
/// context and each side's original line numbers.
pub fn split_diff_rows(rows: &[DiffLine]) -> Vec<SplitDiffRow> {
    let mut output = Vec::new();
    let mut index = 0;
    while index < rows.len() {
        match rows[index].kind {
            DiffLineKind::Deletion => {
                let start = index;
                while index < rows.len() && rows[index].kind == DiffLineKind::Deletion {
                    index += 1;
                }
                let deletion_end = index;
                while index < rows.len() && rows[index].kind == DiffLineKind::Addition {
                    index += 1;
                }
                let deletion_count = deletion_end - start;
                let addition_count = index - deletion_end;
                for offset in 0..deletion_count.max(addition_count) {
                    output.push(SplitDiffRow {
                        left: rows
                            .get(start + offset)
                            .filter(|_| offset < deletion_count)
                            .cloned(),
                        right: rows
                            .get(deletion_end + offset)
                            .filter(|_| offset < addition_count)
                            .cloned(),
                    });
                }
            }
            DiffLineKind::Addition => {
                output.push(SplitDiffRow {
                    left: None,
                    right: Some(rows[index].clone()),
                });
                index += 1;
            }
            DiffLineKind::Context => {
                output.push(SplitDiffRow {
                    left: Some(rows[index].clone()),
                    right: Some(rows[index].clone()),
                });
                index += 1;
            }
            DiffLineKind::Header | DiffLineKind::Hunk | DiffLineKind::Meta => {
                output.push(SplitDiffRow {
                    left: Some(rows[index].clone()),
                    right: Some(rows[index].clone()),
                });
                index += 1;
            }
        }
    }
    output
}

fn hunk_starts(line: &str) -> Option<(Option<u32>, Option<u32>)> {
    let spec = line.strip_prefix("@@ ")?.split(" @@").next()?;
    let mut sides = spec.split_whitespace();
    let old = sides.next()?.strip_prefix('-')?;
    let new = sides.next()?.strip_prefix('+')?;
    Some((Some(hunk_start(old)?), Some(hunk_start(new)?)))
}

fn hunk_start(spec: &str) -> Option<u32> {
    spec.split(',').next()?.parse().ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownBlockKind {
    Heading(u8),
    Paragraph,
    Code(String),
    Quote,
    ListItem,
    Table,
    PlainHtml,
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownBlock {
    pub kind: MarkdownBlockKind,
    pub text: String,
    /// Links remain visible as text; this reader intentionally exposes no URL action.
    pub action_url: Option<String>,
}

/// Parses CommonMark into text-only display blocks. HTML is emitted as inert text,
/// and link destinations are visible but never turned into navigation actions.
pub fn safe_markdown_blocks(markdown: &str) -> Vec<MarkdownBlock> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES;
    let mut blocks = Vec::new();
    let mut current: Option<MarkdownBlock> = None;
    let mut list_counters: Vec<Option<u64>> = Vec::new();
    let mut list_item_depth = 0usize;
    let mut quote_depth = 0usize;
    let mut link_destinations = Vec::<String>::new();

    fn flush(current: &mut Option<MarkdownBlock>, blocks: &mut Vec<MarkdownBlock>) {
        if let Some(block) = current.take()
            && (!block.text.is_empty() || block.kind == MarkdownBlockKind::Rule)
        {
            blocks.push(block);
        }
    }
    fn ensure(
        current: &mut Option<MarkdownBlock>,
        list_item_depth: usize,
        quote_depth: usize,
        default: MarkdownBlockKind,
    ) -> &mut MarkdownBlock {
        current.get_or_insert_with(|| MarkdownBlock {
            kind: if list_item_depth > 0 {
                MarkdownBlockKind::ListItem
            } else if quote_depth > 0 {
                MarkdownBlockKind::Quote
            } else {
                default
            },
            text: String::new(),
            action_url: None,
        })
    }

    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut current, &mut blocks);
                let level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                current = Some(MarkdownBlock {
                    kind: MarkdownBlockKind::Heading(level),
                    text: String::new(),
                    action_url: None,
                });
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut current, &mut blocks);
                let language = match kind {
                    CodeBlockKind::Fenced(language) => language.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                current = Some(MarkdownBlock {
                    kind: MarkdownBlockKind::Code(language),
                    text: String::new(),
                    action_url: None,
                });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut current, &mut blocks);
                quote_depth += 1;
                current = Some(MarkdownBlock {
                    kind: MarkdownBlockKind::Quote,
                    text: String::new(),
                    action_url: None,
                });
                current.as_mut().unwrap().text.push_str("> ");
            }
            Event::Start(Tag::List(start)) => list_counters.push(start),
            Event::Start(Tag::Item) => {
                flush(&mut current, &mut blocks);
                list_item_depth += 1;
                let block = MarkdownBlock {
                    kind: MarkdownBlockKind::ListItem,
                    text: String::new(),
                    action_url: None,
                };
                let marker = match list_counters.last_mut() {
                    Some(Some(next)) => {
                        let number = *next;
                        *next = next.saturating_add(1);
                        format!("{number}. ")
                    }
                    _ => "• ".into(),
                };
                current = Some(block);
                current.as_mut().unwrap().text.push_str(&marker);
            }
            Event::Start(Tag::Table(_)) => {
                flush(&mut current, &mut blocks);
                current = Some(MarkdownBlock {
                    kind: MarkdownBlockKind::Table,
                    text: String::new(),
                    action_url: None,
                });
            }
            Event::Start(Tag::HtmlBlock) => {
                flush(&mut current, &mut blocks);
                current = Some(MarkdownBlock {
                    kind: MarkdownBlockKind::PlainHtml,
                    text: String::new(),
                    action_url: None,
                });
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                link_destinations.push(dest_url.to_string());
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                link_destinations.push(dest_url.to_string());
                ensure(
                    &mut current,
                    list_item_depth,
                    quote_depth,
                    MarkdownBlockKind::Paragraph,
                )
                .text
                .push_str("Image: ");
            }
            Event::Start(Tag::Paragraph) => {
                if current.is_none() {
                    current = Some(MarkdownBlock {
                        kind: if list_item_depth > 0 {
                            MarkdownBlockKind::ListItem
                        } else if quote_depth > 0 {
                            MarkdownBlockKind::Quote
                        } else {
                            MarkdownBlockKind::Paragraph
                        },
                        text: String::new(),
                        action_url: None,
                    });
                }
            }
            Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::CodeBlock)
            | Event::End(TagEnd::Paragraph) => {
                if list_item_depth == 0 && quote_depth == 0 {
                    flush(&mut current, &mut blocks);
                }
            }
            Event::End(TagEnd::Item) => {
                list_item_depth = list_item_depth.saturating_sub(1);
                flush(&mut current, &mut blocks);
            }
            Event::End(TagEnd::List(_)) => {
                list_counters.pop();
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                quote_depth = quote_depth.saturating_sub(1);
                flush(&mut current, &mut blocks);
            }
            Event::End(TagEnd::HtmlBlock) => flush(&mut current, &mut blocks),
            Event::End(TagEnd::Table) => flush(&mut current, &mut blocks),
            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => {
                ensure(
                    &mut current,
                    list_item_depth,
                    quote_depth,
                    MarkdownBlockKind::Table,
                )
                .text
                .push('\n');
            }
            Event::End(TagEnd::TableCell) => {
                ensure(
                    &mut current,
                    list_item_depth,
                    quote_depth,
                    MarkdownBlockKind::Table,
                )
                .text
                .push_str("  |  ");
            }
            Event::End(TagEnd::Link) | Event::End(TagEnd::Image) => {
                if let Some(destination) = link_destinations.pop() {
                    let block = ensure(
                        &mut current,
                        list_item_depth,
                        quote_depth,
                        MarkdownBlockKind::Paragraph,
                    );
                    block.text.push_str(" (");
                    block.text.push_str(&destination);
                    block.text.push(')');
                }
            }
            Event::Text(value) | Event::Code(value) => {
                ensure(
                    &mut current,
                    list_item_depth,
                    quote_depth,
                    MarkdownBlockKind::Paragraph,
                )
                .text
                .push_str(&value);
            }
            Event::Html(value) | Event::InlineHtml(value) => ensure(
                &mut current,
                list_item_depth,
                quote_depth,
                MarkdownBlockKind::PlainHtml,
            )
            .text
            .push_str(&value),
            Event::FootnoteReference(label) => ensure(
                &mut current,
                list_item_depth,
                quote_depth,
                MarkdownBlockKind::Paragraph,
            )
            .text
            .push_str(&format!("[^{label}]")),
            Event::TaskListMarker(checked) => ensure(
                &mut current,
                list_item_depth,
                quote_depth,
                MarkdownBlockKind::Paragraph,
            )
            .text
            .push_str(if checked { "☑ " } else { "☐ " }),
            Event::SoftBreak | Event::HardBreak => ensure(
                &mut current,
                list_item_depth,
                quote_depth,
                MarkdownBlockKind::Paragraph,
            )
            .text
            .push('\n'),
            Event::Rule => {
                flush(&mut current, &mut blocks);
                blocks.push(MarkdownBlock {
                    kind: MarkdownBlockKind::Rule,
                    text: "────────────────".into(),
                    action_url: None,
                });
            }
            Event::InlineMath(value) | Event::DisplayMath(value) => ensure(
                &mut current,
                list_item_depth,
                quote_depth,
                MarkdownBlockKind::Paragraph,
            )
            .text
            .push_str(&value),
            Event::Start(_) | Event::End(_) => {}
        }
    }
    flush(&mut current, &mut blocks);
    blocks
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxTokenKind {
    Keyword,
    String,
    Comment,
    Number,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxSpan {
    pub start: usize,
    pub end: usize,
    pub kind: SyntaxTokenKind,
}

/// Small deterministic lexer for common review languages, used for readable syntax cues.
/// Unknown languages still receive diff semantics and retain their plain text.
pub fn syntax_spans(line: &str, language: &str) -> Vec<SyntaxSpan> {
    let extension = language
        .rsplit('.')
        .next()
        .unwrap_or(language)
        .to_ascii_lowercase();
    let keywords: &[&str] = match extension.as_str() {
        "rs" | "rust" => &[
            "as", "async", "await", "break", "const", "continue", "crate", "else", "enum", "false",
            "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
            "ref", "return", "self", "static", "struct", "trait", "true", "type", "use", "where",
            "while",
        ],
        "js" | "jsx" | "ts" | "tsx" | "javascript" | "typescript" => &[
            "async", "await", "break", "case", "catch", "class", "const", "continue", "else",
            "false", "for", "function", "if", "import", "in", "let", "new", "null", "return",
            "static", "this", "throw", "true", "try", "type", "typeof", "var", "while",
        ],
        "py" | "python" => &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "elif",
            "else", "except", "false", "for", "from", "if", "import", "in", "is", "lambda", "not",
            "or", "pass", "raise", "return", "true", "try", "while", "with",
        ],
        _ => &[],
    };
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let token = if (bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/'))
            || ((extension == "py" || extension == "python") && bytes[i] == b'#')
        {
            i = bytes.len();
            Some(SyntaxTokenKind::Comment)
        } else if matches!(bytes[i], b'\'' | b'"' | b'`') {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            Some(SyntaxTokenKind::String)
        } else if bytes[i].is_ascii_digit() {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            Some(SyntaxTokenKind::Number)
        } else if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];
            if keywords.contains(&word) {
                Some(SyntaxTokenKind::Keyword)
            } else {
                None
            }
        } else {
            i += line[i..].chars().next().unwrap().len_utf8();
            None
        };
        if let Some(kind) = token {
            spans.push(SyntaxSpan {
                start,
                end: i,
                kind,
            });
        }
    }
    spans
}

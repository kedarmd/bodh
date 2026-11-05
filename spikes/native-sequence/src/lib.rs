//! Disposable native rendering experiment, not the Bodh contract.
pub mod gh_prerequisite;
pub mod manual_review;
pub mod preferences;
pub mod repository_input;

pub use manual_review::{
    DiffLine, DiffLineKind, MarkdownBlock, MarkdownBlockKind, SplitDiffRow, SyntaxSpan,
    SyntaxTokenKind, TextDraftEditor, diff_rows, safe_markdown_blocks, split_diff_rows,
    syntax_spans,
};

pub const SOURCE: &str = include_str!("../fixtures/review.rs");
pub const BASE_SOURCE: &str = include_str!("../fixtures/review.base.rs");

#[derive(Clone, Debug)]
pub struct Source {
    pub start: usize,
    pub end: usize,
}
impl Source {
    pub fn text(&self) -> String {
        SOURCE
            .lines()
            .skip(self.start - 1)
            .take(self.end - self.start + 1)
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn changed(&self) -> bool {
        // Equal-length handcrafted fixture pair; this is not a general diff mapper.
        SOURCE
            .lines()
            .zip(BASE_SOURCE.lines())
            .enumerate()
            .any(|(index, (head, base))| {
                (self.start..=self.end).contains(&(index + 1)) && head != base
            })
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Message,
    Return,
    SelfCall,
    Note,
}
#[derive(Clone, Debug)]
pub struct Interaction {
    pub from: usize,
    pub to: usize,
    pub kind: Kind,
    pub label: String,
    pub explanation: String,
    pub source: Source,
}
// Conditional branches cannot contain conditionals by construction.
#[derive(Clone, Debug)]
pub enum Step {
    Interaction(Interaction),
    Alt(Vec<(String, Vec<Interaction>)>),
    Opt(String, Vec<Interaction>),
}
#[derive(Clone, Debug)]
pub struct Flow {
    pub title: String,
    pub participants: Vec<String>,
    pub steps: Vec<Step>,
}
impl Flow {
    pub fn interactions(&self) -> Vec<&Interaction> {
        self.steps
            .iter()
            .flat_map(|step| match step {
                Step::Interaction(i) => vec![i],
                Step::Alt(branches) => branches
                    .iter()
                    .flat_map(|(_, items)| items.iter())
                    .collect(),
                Step::Opt(_, items) => items.iter().collect(),
            })
            .collect()
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub fn intersects(&self, other: Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}
#[derive(Clone, Debug)]
pub struct Row {
    pub interaction: usize,
    pub rect: Rect,
    pub arrow_y: f32,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub rect: Rect,
    pub label: String,
    pub branch: bool,
}
#[derive(Clone, Debug)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub columns: Vec<f32>,
    pub rows: Vec<Row>,
    pub frames: Vec<Frame>,
}
impl Scene {
    pub fn layout(flow: &Flow) -> Self {
        // Fixed, deterministic renderer rules; overflow scrolls instead of changing order.
        let columns: Vec<_> = (0..flow.participants.len())
            .map(|i| 140.0 + i as f32 * 240.0)
            .collect();
        let width = (flow.participants.len() as f32 * 240.0 + 120.0).max(520.0);
        let mut scene = Self {
            width,
            height: 0.,
            columns,
            rows: vec![],
            frames: vec![],
        };
        let mut y = 90.;
        let mut index = 0;
        for step in &flow.steps {
            match step {
                Step::Interaction(i) => scene.add(i, &mut index, &mut y),
                Step::Alt(branches) => {
                    let group_start = y;
                    y += 32.;
                    for (branch, items) in branches {
                        let start = y;
                        y += 32.;
                        for item in items {
                            scene.add(item, &mut index, &mut y);
                        }
                        scene.frames.push(Frame {
                            rect: Rect {
                                x: 20.,
                                y: start,
                                w: width - 40.,
                                h: y - start,
                            },
                            label: format!("[{branch}]"),
                            branch: true,
                        });
                    }
                    scene.frames.push(Frame {
                        rect: Rect {
                            x: 20.,
                            y: group_start,
                            w: width - 40.,
                            h: y - group_start,
                        },
                        label: "alt".into(),
                        branch: false,
                    });
                }
                Step::Opt(condition, items) => {
                    let start = y;
                    y += 32.;
                    for item in items {
                        scene.add(item, &mut index, &mut y);
                    }
                    scene.frames.push(Frame {
                        rect: Rect {
                            x: 20.,
                            y: start,
                            w: width - 40.,
                            h: y - start,
                        },
                        label: format!("opt [{condition}]"),
                        branch: false,
                    });
                }
            }
        }
        scene.height = y + 32.;
        scene
    }
    fn add(&mut self, i: &Interaction, index: &mut usize, y: &mut f32) {
        let a = self.columns[i.from];
        let b = self.columns[i.to];
        let w = if i.from == i.to { 200. } else { (a - b).abs() };
        // Conservative monospace budget, including cue and ordinal. Native text wraps.
        let lines = ((i.label.chars().count() + 24) as f32 / ((w - 16.) / 8.).floor()).ceil();
        let h = (lines * 22. + 52.).max(90.);
        self.rows.push(Row {
            interaction: *index,
            rect: Rect {
                x: a.min(b),
                y: *y,
                w,
                h,
            },
            arrow_y: *y + h - 24.,
        });
        *index += 1;
        *y += h + 12.;
    }
    pub fn visible_rows(&self, viewport: Rect) -> impl Iterator<Item = &Row> {
        let start = self
            .rows
            .partition_point(|r| r.rect.y + r.rect.h <= viewport.y);
        self.rows[start..]
            .iter()
            .take_while(move |r| r.rect.y < viewport.y + viewport.h)
            .filter(move |r| r.rect.intersects(viewport))
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.rows
            .iter()
            .find(|row| row.rect.contains(x, y))
            .map(|r| r.interaction)
    }
}
fn interaction(
    from: usize,
    to: usize,
    kind: Kind,
    label: &str,
    start: usize,
    end: usize,
) -> Interaction {
    Interaction {
        from,
        to,
        kind,
        label: label.into(),
        source: Source { start, end },
        explanation: format!(
            "Handcrafted causal step: {label}. Reads only embedded fixture lines {start}–{end}; never working-tree source."
        ),
    }
}
pub fn demo() -> Flow {
    use Kind::*;
    Flow {
        title: "Prepare pinned review".into(),
        participants: vec![
            "API".into(),
            "Auth service / session validation".into(),
            "Snapshot store".into(),
        ],
        steps: vec![
            Step::Interaction(interaction(0, 1, Message, "Extract request token", 3, 3)),
            Step::Interaction(interaction(
                1,
                1,
                SelfCall,
                "Check expiry before loading the session",
                14,
                14,
            )),
            Step::Interaction(interaction(
                1,
                2,
                Message,
                "Load the immutable snapshot using the authenticated session cache key",
                5,
                6,
            )),
            Step::Alt(vec![
                (
                    "snapshot current".into(),
                    vec![interaction(2, 0, Return, "Return pinned review", 7, 8)],
                ),
                (
                    "snapshot stale".into(),
                    vec![interaction(2, 0, Return, "Reject stale context", 9, 10)],
                ),
            ]),
            Step::Opt(
                "audit enabled".into(),
                vec![interaction(
                    0,
                    0,
                    SelfCall,
                    "Record review identity",
                    17,
                    18,
                )],
            ),
            Step::Interaction(interaction(
                0,
                0,
                Note,
                "Source stays pinned when the flow changes",
                1,
                2,
            )),
        ],
    }
}
pub fn stress(participants: usize, steps: usize) -> Flow {
    Flow { title: format!("Stress · {participants} participants / {steps} steps"), participants: (0..participants).map(|i|format!("Participant {i:02}")).collect(), steps: (0..steps).map(|i| Step::Interaction(interaction(i%participants,(i+1)%participants,Kind::Message,"Long label: inspect immutable source without selecting an unrelated interaction, even after horizontal and vertical scrolling or resizing the adjacent source inspector",3+i%15,3+i%15))).collect() }
}

/// Shared local search predicate for the native pull-request browser.
pub fn matches_pull_request(
    number: u64,
    title: &str,
    author: &str,
    state: &str,
    query: &str,
    lifecycle: &str,
) -> bool {
    let query = query.trim().to_lowercase();
    let lifecycle_matches = match lifecycle {
        "All" => true,
        "Open" => state.eq_ignore_ascii_case("Open"),
        "Closed" => state.eq_ignore_ascii_case("Closed") || state.eq_ignore_ascii_case("Merged"),
        _ => false,
    };
    lifecycle_matches
        && (query.is_empty()
            || title.to_lowercase().contains(&query)
            || number.to_string().contains(&query)
            || author.to_lowercase().contains(&query))
}

#[cfg(test)]
mod product_search_tests {
    use super::matches_pull_request;

    #[test]
    fn browser_search_matches_title_number_author_and_lifecycle() {
        assert!(matches_pull_request(
            24,
            "Improve cache",
            "ana",
            "OPEN",
            "cache",
            "All"
        ));
        assert!(matches_pull_request(
            31,
            "Fix source",
            "devon",
            "CLOSED",
            "31",
            "All"
        ));
        assert!(matches_pull_request(
            31,
            "Fix source",
            "devon",
            "CLOSED",
            "DEVON",
            "All"
        ));
        assert!(matches_pull_request(
            24,
            "Improve cache",
            "ana",
            "OPEN",
            "",
            "Open"
        ));
        assert!(!matches_pull_request(
            24,
            "Improve cache",
            "ana",
            "OPEN",
            "",
            "Closed"
        ));
        assert!(matches_pull_request(
            32,
            "Merged change",
            "ana",
            "MERGED",
            "",
            "Closed"
        ));
    }
}

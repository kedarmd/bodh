use bodh::{Diagram, Finding, Revision, SimpleStep, SourceRef, Step};
use bodh_sequence_spike::{Flow, Interaction, Kind, Scene, Source as RenderSource};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct InteractionLink {
    pub id: String,
    pub source: Option<SourceRef>,
    pub explanation: String,
    pub findings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct AdaptedDiagram {
    pub id: String,
    pub description: String,
    pub flow: Flow,
    pub scene: Scene,
    pub links: Vec<InteractionLink>,
    pub changed_interactions: HashSet<usize>,
}

pub type ChangedLines = HashMap<(String, bool), HashSet<u32>>;

pub fn adapt_diagram(
    diagram: &Diagram,
    findings: &[Finding],
    changed_lines: &ChangedLines,
) -> AdaptedDiagram {
    let participant_indices = diagram
        .participants
        .iter()
        .enumerate()
        .map(|(index, participant)| (participant.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut links = Vec::new();
    let steps = diagram
        .steps
        .iter()
        .map(|step| {
            adapt_step(
                step,
                &participant_indices,
                findings,
                changed_lines,
                &mut links,
            )
        })
        .collect();
    let flow = Flow {
        title: diagram.title.clone(),
        participants: diagram
            .participants
            .iter()
            .map(|participant| participant.label.clone())
            .collect(),
        steps,
    };
    let changed_interactions = links
        .iter()
        .enumerate()
        .filter_map(|(index, link)| {
            link.source
                .as_ref()
                .filter(|source| source_intersects_changed(source, changed_lines))
                .map(|_| index)
        })
        .collect();
    let scene = Scene::layout(&flow);
    AdaptedDiagram {
        id: diagram.id.clone(),
        description: diagram.description.clone(),
        flow,
        scene,
        links,
        changed_interactions,
    }
}

fn adapt_step(
    step: &Step,
    participants: &HashMap<&str, usize>,
    findings: &[Finding],
    changed_lines: &ChangedLines,
    links: &mut Vec<InteractionLink>,
) -> bodh_sequence_spike::Step {
    match step {
        Step::Message {
            id,
            from,
            to,
            label,
            source,
        } => bodh_sequence_spike::Step::Interaction(adapt_interaction(
            id,
            participants[from.as_str()],
            participants[to.as_str()],
            Kind::Message,
            label,
            source.as_ref(),
            findings,
            changed_lines,
            links,
        )),
        Step::Return {
            id,
            from,
            to,
            label,
            in_reply_to,
            source,
        } => {
            let interaction = adapt_interaction(
                id,
                participants[from.as_str()],
                participants[to.as_str()],
                Kind::Return,
                label,
                source.as_ref(),
                findings,
                changed_lines,
                links,
            );
            bodh_sequence_spike::Step::Interaction(annotate_return(interaction, links, in_reply_to))
        }
        Step::SelfCall {
            id,
            participant,
            label,
            source,
        } => {
            let at = participants[participant.as_str()];
            bodh_sequence_spike::Step::Interaction(adapt_interaction(
                id,
                at,
                at,
                Kind::SelfCall,
                label,
                source.as_ref(),
                findings,
                changed_lines,
                links,
            ))
        }
        Step::Note {
            id,
            participants: note_participants,
            text,
            source,
        } => {
            let positions = note_participants
                .iter()
                .filter_map(|participant| participants.get(participant.as_str()).copied())
                .collect::<Vec<_>>();
            let start = positions.iter().copied().min().unwrap_or(0);
            let end = positions.iter().copied().max().unwrap_or(start);
            bodh_sequence_spike::Step::Interaction(adapt_interaction(
                id,
                start,
                end,
                Kind::Note,
                text,
                source.as_ref(),
                findings,
                changed_lines,
                links,
            ))
        }
        Step::Alt {
            label, branches, ..
        } => bodh_sequence_spike::Step::Alt(
            label.clone(),
            branches
                .iter()
                .map(|branch| {
                    (
                        branch.label.clone(),
                        branch
                            .steps
                            .iter()
                            .map(|step| {
                                adapt_simple_step(
                                    step,
                                    participants,
                                    findings,
                                    changed_lines,
                                    links,
                                )
                            })
                            .collect(),
                    )
                })
                .collect(),
        ),
        Step::Opt { label, steps, .. } => bodh_sequence_spike::Step::Opt(
            label.clone(),
            steps
                .iter()
                .map(|step| adapt_simple_step(step, participants, findings, changed_lines, links))
                .collect(),
        ),
    }
}

fn adapt_simple_step(
    step: &SimpleStep,
    participants: &HashMap<&str, usize>,
    findings: &[Finding],
    changed_lines: &ChangedLines,
    links: &mut Vec<InteractionLink>,
) -> Interaction {
    match step {
        SimpleStep::Message {
            id,
            from,
            to,
            label,
            source,
        } => adapt_interaction(
            id,
            participants[from.as_str()],
            participants[to.as_str()],
            Kind::Message,
            label,
            source.as_ref(),
            findings,
            changed_lines,
            links,
        ),
        SimpleStep::Return {
            id,
            from,
            to,
            label,
            in_reply_to,
            source,
        } => {
            let interaction = adapt_interaction(
                id,
                participants[from.as_str()],
                participants[to.as_str()],
                Kind::Return,
                label,
                source.as_ref(),
                findings,
                changed_lines,
                links,
            );
            annotate_return(interaction, links, in_reply_to)
        }
        SimpleStep::SelfCall {
            id,
            participant,
            label,
            source,
        } => {
            let at = participants[participant.as_str()];
            adapt_interaction(
                id,
                at,
                at,
                Kind::SelfCall,
                label,
                source.as_ref(),
                findings,
                changed_lines,
                links,
            )
        }
        SimpleStep::Note {
            id,
            participants: note_participants,
            text,
            source,
        } => {
            let positions = note_participants
                .iter()
                .filter_map(|participant| participants.get(participant.as_str()).copied())
                .collect::<Vec<_>>();
            adapt_interaction(
                id,
                positions.iter().copied().min().unwrap_or(0),
                positions.iter().copied().max().unwrap_or(0),
                Kind::Note,
                text,
                source.as_ref(),
                findings,
                changed_lines,
                links,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn adapt_interaction(
    id: &str,
    from: usize,
    to: usize,
    kind: Kind,
    label: &str,
    source: Option<&SourceRef>,
    findings: &[Finding],
    changed_lines: &ChangedLines,
    links: &mut Vec<InteractionLink>,
) -> Interaction {
    let source = source.cloned();
    let finding_ids = source.as_ref().map_or_else(Vec::new, |source| {
        findings
            .iter()
            .filter(|finding| finding_sources(finding).any(|candidate| overlaps(candidate, source)))
            .map(|finding| finding.id.clone())
            .collect()
    });
    let explanation = source.as_ref().map_or_else(
        || format!("{label}. No pinned source range was supplied for this interaction."),
        |source| {
            format!(
                "{label} · {}:{}–{} ({:?})",
                source.path, source.start_line, source.end_line, source.revision
            )
        },
    );
    links.push(InteractionLink {
        id: id.to_owned(),
        source: source.clone(),
        explanation: explanation.clone(),
        findings: finding_ids,
    });
    let source_lines = source
        .as_ref()
        .map_or((1, 1), |source| (source.start_line, source.end_line));
    let _ = changed_lines;
    Interaction {
        from,
        to,
        kind,
        label: label.to_owned(),
        source: RenderSource {
            start: source_lines.0 as usize,
            end: source_lines.1 as usize,
        },
        explanation,
    }
}

fn annotate_return(
    mut interaction: Interaction,
    links: &mut [InteractionLink],
    in_reply_to: &str,
) -> Interaction {
    interaction.explanation = format!(
        "{} · returns to interaction {in_reply_to}",
        interaction.explanation
    );
    if let Some(link) = links.last_mut() {
        link.explanation = interaction.explanation.clone();
    }
    interaction
}

fn finding_sources(finding: &Finding) -> impl Iterator<Item = &SourceRef> {
    finding
        .evidence
        .iter()
        .map(|evidence| &evidence.source)
        .chain(finding.locations.iter().map(|location| &location.source))
}

fn overlaps(left: &SourceRef, right: &SourceRef) -> bool {
    left.path == right.path
        && left.revision == right.revision
        && left.start_line <= right.end_line
        && right.start_line <= left.end_line
}

fn source_intersects_changed(source: &SourceRef, changed_lines: &ChangedLines) -> bool {
    let side = source.revision == Revision::Head;
    changed_lines
        .get(&(source.path.clone(), side))
        .is_some_and(|lines| {
            lines
                .iter()
                .any(|line| (source.start_line..=source.end_line).contains(line))
        })
}

#[cfg(test)]
mod tests {
    use super::{ChangedLines, Kind, adapt_diagram};
    use bodh::{Diagram, Finding};
    use std::collections::HashSet;

    fn diagram() -> Diagram {
        serde_json::from_value(serde_json::json!({
            "id":"flow-1", "title":"Request flow", "description":"An explanation",
            "participants":[{"id":"ui","label":"User interface","category":null},{"id":"api","label":"API","category":null}],
            "steps":[
                {"kind":"message","id":"call","from":"ui","to":"api","label":"Send request","source":{"path":"src/api.rs","revision":"head","start_line":10,"end_line":10}},
                {"kind":"alt","id":"choice","label":"Result","branches":[
                    {"label":"Success","steps":[{"kind":"return","id":"ok","from":"api","to":"ui","label":"Return value","in_reply_to":"call","source":{"path":"src/api.rs","revision":"head","start_line":11,"end_line":11}}]},
                    {"label":"Failure","steps":[{"kind":"note","id":"failure","participants":["ui","api"],"text":"Show error","source":{"path":"src/api.rs","revision":"head","start_line":12,"end_line":12}}]}
                ]},
                {"kind":"opt","id":"audit","label":"Audit enabled","steps":[{"kind":"self_call","id":"record","participant":"api","label":"Record request","source":{"path":"src/api.rs","revision":"head","start_line":13,"end_line":13}}]}
            ]
        })).unwrap()
    }

    fn finding() -> Finding {
        serde_json::from_value(serde_json::json!({
            "id":"F1","title":"Keep request scoped","severity":"medium","confidence":"high",
            "summary":"summary","reason":"reason","impact":"impact",
            "evidence":[{"source":{"path":"src/api.rs","revision":"head","start_line":9,"end_line":11},"explanation":"Evidence"}],
            "locations":[],"suggestions":[],"related_thread_ids":[]
        })).unwrap()
    }

    #[test]
    fn adapts_every_step_variant_in_declared_order_and_keeps_semantic_links() {
        let mut changed = ChangedLines::new();
        changed.insert(("src/api.rs".into(), true), HashSet::from([10, 12]));
        let adapted = adapt_diagram(&diagram(), &[finding()], &changed);

        assert_eq!(adapted.flow.participants, ["User interface", "API"]);
        assert!(matches!(
            adapted.flow.steps[1],
            bodh_sequence_spike::Step::Alt(_, _)
        ));
        assert!(matches!(
            adapted.flow.steps[2],
            bodh_sequence_spike::Step::Opt(_, _)
        ));
        assert_eq!(
            adapted
                .links
                .iter()
                .map(|link| link.id.as_str())
                .collect::<Vec<_>>(),
            ["call", "ok", "failure", "record"]
        );
        let interactions = adapted.flow.interactions();
        assert_eq!(
            interactions
                .iter()
                .map(|item| item.kind)
                .collect::<Vec<_>>(),
            [Kind::Message, Kind::Return, Kind::Note, Kind::SelfCall]
        );
        assert!(
            adapted.links[1]
                .explanation
                .contains("returns to interaction call")
        );
        assert_eq!(adapted.links[0].findings, ["F1"]);
        assert_eq!(adapted.changed_interactions, HashSet::from([0, 2]));
        assert_eq!(
            adapted
                .scene
                .rows
                .iter()
                .map(|row| row.interaction)
                .collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn changed_cues_require_an_exact_revision_path_and_line_intersection() {
        let mut changed = ChangedLines::new();
        changed.insert(("src/api.rs".into(), false), HashSet::from([10]));
        let adapted = adapt_diagram(&diagram(), &[], &changed);
        assert!(adapted.changed_interactions.is_empty());
    }
}

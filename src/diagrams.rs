//! Validate sequence semantics before a native layout or hit target is produced.
use crate::{
    ContractError, OperationControl, ReviewContext, ReviewResult, SimpleStep, Step, read_source,
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

type MessageEndpoints = (String, String);

pub(crate) fn validate_diagrams(
    review: &ReviewResult,
    context: &ReviewContext,
    repository: &Path,
    control: &OperationControl,
) -> Result<(), ContractError> {
    let mut diagram_ids = HashSet::new();
    for (index, diagram) in review.diagrams.iter().enumerate() {
        let pointer = format!("/diagrams/{index}");
        require_id(&diagram.id, &format!("{pointer}/id"))?;
        if !diagram_ids.insert(diagram.id.as_str()) {
            return Err(invalid(
                "ID_DUPLICATE",
                "Diagram IDs must be unique",
                &format!("{pointer}/id"),
            ));
        }
        require_text(&diagram.title, &format!("{pointer}/title"))?;
        require_text(&diagram.description, &format!("{pointer}/description"))?;
        if diagram.participants.is_empty() || diagram.steps.is_empty() {
            return Err(invalid(
                "DIAGRAM_EMPTY",
                "A diagram needs participants and at least one interaction",
                &pointer,
            ));
        }

        let mut participants = HashSet::new();
        for (participant_index, participant) in diagram.participants.iter().enumerate() {
            let item = format!("{pointer}/participants/{participant_index}");
            require_id(&participant.id, &format!("{item}/id"))?;
            require_text(&participant.label, &format!("{item}/label"))?;
            if !participants.insert(participant.id.as_str()) {
                return Err(invalid(
                    "ID_DUPLICATE",
                    "Participant IDs must be unique within a diagram",
                    &format!("{item}/id"),
                ));
            }
        }

        // Participant and step references share a scoped ID namespace per diagram.
        let mut step_ids: HashSet<String> =
            participants.iter().map(|id| (*id).to_owned()).collect();
        let mut known_messages = HashMap::new();
        validate_top_steps(
            &diagram.steps,
            &participants,
            &mut step_ids,
            &mut known_messages,
            context,
            repository,
            control,
            &format!("{pointer}/steps"),
        )?;
    }
    Ok(())
}

enum CommonStep<'a> {
    Message {
        id: &'a str,
        from: &'a str,
        to: &'a str,
        label: &'a str,
        source: Option<&'a crate::SourceRef>,
    },
    Return {
        id: &'a str,
        from: &'a str,
        to: &'a str,
        label: &'a str,
        in_reply_to: &'a str,
        source: Option<&'a crate::SourceRef>,
    },
    SelfCall {
        id: &'a str,
        participant: &'a str,
        label: &'a str,
        source: Option<&'a crate::SourceRef>,
    },
    Note {
        id: &'a str,
        participants: &'a [String],
        text: &'a str,
        source: Option<&'a crate::SourceRef>,
    },
}

impl<'a> CommonStep<'a> {
    fn from_top(step: &'a Step) -> Option<Self> {
        match step {
            Step::Message {
                id,
                from,
                to,
                label,
                source,
            } => Some(Self::Message {
                id,
                from,
                to,
                label,
                source: source.as_ref(),
            }),
            Step::Return {
                id,
                from,
                to,
                label,
                in_reply_to,
                source,
            } => Some(Self::Return {
                id,
                from,
                to,
                label,
                in_reply_to,
                source: source.as_ref(),
            }),
            Step::SelfCall {
                id,
                participant,
                label,
                source,
            } => Some(Self::SelfCall {
                id,
                participant,
                label,
                source: source.as_ref(),
            }),
            Step::Note {
                id,
                participants,
                text,
                source,
            } => Some(Self::Note {
                id,
                participants,
                text,
                source: source.as_ref(),
            }),
            Step::Alt { .. } | Step::Opt { .. } => None,
        }
    }

    fn from_simple(step: &'a SimpleStep) -> Self {
        match step {
            SimpleStep::Message {
                id,
                from,
                to,
                label,
                source,
            } => Self::Message {
                id,
                from,
                to,
                label,
                source: source.as_ref(),
            },
            SimpleStep::Return {
                id,
                from,
                to,
                label,
                in_reply_to,
                source,
            } => Self::Return {
                id,
                from,
                to,
                label,
                in_reply_to,
                source: source.as_ref(),
            },
            SimpleStep::SelfCall {
                id,
                participant,
                label,
                source,
            } => Self::SelfCall {
                id,
                participant,
                label,
                source: source.as_ref(),
            },
            SimpleStep::Note {
                id,
                participants,
                text,
                source,
            } => Self::Note {
                id,
                participants,
                text,
                source: source.as_ref(),
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_top_steps(
    steps: &[Step],
    participants: &HashSet<&str>,
    step_ids: &mut HashSet<String>,
    messages: &mut HashMap<String, MessageEndpoints>,
    context: &ReviewContext,
    repository: &Path,
    control: &OperationControl,
    pointer: &str,
) -> Result<(), ContractError> {
    for (index, step) in steps.iter().enumerate() {
        let item = format!("{pointer}/{index}");
        if let Some(common) = CommonStep::from_top(step) {
            validate_common_step(
                common,
                participants,
                step_ids,
                messages,
                context,
                repository,
                control,
                &item,
            )?;
            continue;
        }

        match step {
            Step::Alt {
                id,
                label,
                branches,
                ..
            } => {
                insert_step_id(id, step_ids, &format!("{item}/id"))?;
                require_text(label, &format!("{item}/label"))?;
                if branches.len() < 2 {
                    return Err(invalid(
                        "ALT_BRANCHES_INVALID",
                        "An alt step needs at least two nonempty branches",
                        &format!("{item}/branches"),
                    ));
                }
                let mut labels = HashSet::new();
                for (branch_index, branch) in branches.iter().enumerate() {
                    let branch_pointer = format!("{item}/branches/{branch_index}");
                    require_text(&branch.label, &format!("{branch_pointer}/label"))?;
                    if !labels.insert(branch.label.as_str()) {
                        return Err(invalid(
                            "ALT_BRANCHES_INVALID",
                            "Branch labels must be distinct",
                            &format!("{branch_pointer}/label"),
                        ));
                    }
                    if branch.steps.is_empty() {
                        return Err(invalid(
                            "ALT_BRANCHES_INVALID",
                            "Every alt branch needs at least one step",
                            &format!("{branch_pointer}/steps"),
                        ));
                    }
                    // A branch can use earlier messages, but calls inside one alternative
                    // are not guaranteed to have occurred after the alternative.
                    let mut branch_messages = messages.clone();
                    validate_simple_steps(
                        &branch.steps,
                        participants,
                        step_ids,
                        &mut branch_messages,
                        context,
                        repository,
                        control,
                        &format!("{branch_pointer}/steps"),
                    )?;
                }
            }
            Step::Opt {
                id,
                label,
                steps: body,
                ..
            } => {
                insert_step_id(id, step_ids, &format!("{item}/id"))?;
                require_text(label, &format!("{item}/label"))?;
                if body.is_empty() {
                    return Err(invalid(
                        "OPT_STEPS_INVALID",
                        "An opt step needs at least one interaction",
                        &format!("{item}/steps"),
                    ));
                }
                // An optional call cannot be assumed to have run in following steps.
                let mut optional_messages = messages.clone();
                validate_simple_steps(
                    body,
                    participants,
                    step_ids,
                    &mut optional_messages,
                    context,
                    repository,
                    control,
                    &format!("{item}/steps"),
                )?;
            }
            Step::Message { .. }
            | Step::Return { .. }
            | Step::SelfCall { .. }
            | Step::Note { .. } => unreachable!("common steps handled above"),
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_simple_steps(
    steps: &[SimpleStep],
    participants: &HashSet<&str>,
    step_ids: &mut HashSet<String>,
    messages: &mut HashMap<String, MessageEndpoints>,
    context: &ReviewContext,
    repository: &Path,
    control: &OperationControl,
    pointer: &str,
) -> Result<(), ContractError> {
    for (index, step) in steps.iter().enumerate() {
        let item = format!("{pointer}/{index}");
        validate_common_step(
            CommonStep::from_simple(step),
            participants,
            step_ids,
            messages,
            context,
            repository,
            control,
            &item,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_common_step(
    step: CommonStep<'_>,
    participants: &HashSet<&str>,
    step_ids: &mut HashSet<String>,
    messages: &mut HashMap<String, MessageEndpoints>,
    context: &ReviewContext,
    repository: &Path,
    control: &OperationControl,
    item: &str,
) -> Result<(), ContractError> {
    match step {
        CommonStep::Message {
            id,
            from,
            to,
            label,
            source,
        } => validate_message(
            id,
            from,
            to,
            label,
            source,
            participants,
            step_ids,
            messages,
            context,
            repository,
            control,
            item,
        ),
        CommonStep::Return {
            id,
            from,
            to,
            label,
            in_reply_to,
            source,
        } => validate_return(
            id,
            from,
            to,
            label,
            in_reply_to,
            source,
            participants,
            step_ids,
            messages,
            context,
            repository,
            control,
            item,
        ),
        CommonStep::SelfCall {
            id,
            participant,
            label,
            source,
        } => {
            insert_step_id(id, step_ids, &format!("{item}/id"))?;
            require_text(label, &format!("{item}/label"))?;
            if !participants.contains(participant) {
                return Err(invalid(
                    "PARTICIPANT_REFERENCE_INVALID",
                    "Self-call must reference a declared participant",
                    &format!("{item}/participant"),
                ));
            }
            validate_code_source(
                source,
                context,
                repository,
                control,
                &format!("{item}/source"),
            )?;
            messages.insert(
                id.to_owned(),
                (participant.to_owned(), participant.to_owned()),
            );
            Ok(())
        }
        CommonStep::Note {
            id,
            participants: note_participants,
            text,
            source,
        } => validate_note(
            id,
            note_participants,
            text,
            source,
            participants,
            step_ids,
            context,
            repository,
            control,
            item,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_message(
    id: &str,
    from: &str,
    to: &str,
    label: &str,
    source: Option<&crate::SourceRef>,
    participants: &HashSet<&str>,
    step_ids: &mut HashSet<String>,
    messages: &mut HashMap<String, MessageEndpoints>,
    context: &ReviewContext,
    repository: &Path,
    control: &OperationControl,
    pointer: &str,
) -> Result<(), ContractError> {
    insert_step_id(id, step_ids, &format!("{pointer}/id"))?;
    require_text(label, &format!("{pointer}/label"))?;
    validate_endpoints(from, to, participants, pointer)?;
    if from == to {
        return Err(invalid(
            "MESSAGE_ENDPOINTS_INVALID",
            "Use self_call for an interaction within one participant",
            &format!("{pointer}/to"),
        ));
    }
    validate_code_source(
        source,
        context,
        repository,
        control,
        &format!("{pointer}/source"),
    )?;
    messages.insert(id.to_owned(), (from.to_owned(), to.to_owned()));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_return(
    id: &str,
    from: &str,
    to: &str,
    label: &str,
    in_reply_to: &str,
    source: Option<&crate::SourceRef>,
    participants: &HashSet<&str>,
    step_ids: &mut HashSet<String>,
    messages: &HashMap<String, MessageEndpoints>,
    context: &ReviewContext,
    repository: &Path,
    control: &OperationControl,
    pointer: &str,
) -> Result<(), ContractError> {
    insert_step_id(id, step_ids, &format!("{pointer}/id"))?;
    require_text(label, &format!("{pointer}/label"))?;
    validate_endpoints(from, to, participants, pointer)?;
    let Some((request_from, request_to)) = messages.get(in_reply_to) else {
        return Err(invalid(
            "STEP_REFERENCE_INVALID",
            "A return must refer to an earlier message on the same flow path",
            &format!("{pointer}/in_reply_to"),
        ));
    };
    if from != request_to || to != request_from {
        return Err(invalid(
            "RETURN_ENDPOINTS_INVALID",
            "Return endpoints must reverse the referenced message",
            &format!("{pointer}/from"),
        ));
    }
    validate_code_source(
        source,
        context,
        repository,
        control,
        &format!("{pointer}/source"),
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_note(
    id: &str,
    note_participants: &[String],
    text: &str,
    source: Option<&crate::SourceRef>,
    participants: &HashSet<&str>,
    step_ids: &mut HashSet<String>,
    context: &ReviewContext,
    repository: &Path,
    control: &OperationControl,
    pointer: &str,
) -> Result<(), ContractError> {
    insert_step_id(id, step_ids, &format!("{pointer}/id"))?;
    require_text(text, &format!("{pointer}/text"))?;
    let mut referenced = HashSet::new();
    for (index, participant) in note_participants.iter().enumerate() {
        if !participants.contains(participant.as_str()) || !referenced.insert(participant.as_str())
        {
            return Err(invalid(
                "PARTICIPANT_REFERENCE_INVALID",
                "Note participants must be declared and distinct",
                &format!("{pointer}/participants/{index}"),
            ));
        }
    }
    validate_optional_source(
        source,
        context,
        repository,
        control,
        &format!("{pointer}/source"),
    )
}

fn validate_endpoints(
    from: &str,
    to: &str,
    participants: &HashSet<&str>,
    pointer: &str,
) -> Result<(), ContractError> {
    if !participants.contains(from) {
        return Err(invalid(
            "PARTICIPANT_REFERENCE_INVALID",
            "Step source must be a declared participant",
            &format!("{pointer}/from"),
        ));
    }
    if !participants.contains(to) {
        return Err(invalid(
            "PARTICIPANT_REFERENCE_INVALID",
            "Step destination must be a declared participant",
            &format!("{pointer}/to"),
        ));
    }
    Ok(())
}

fn insert_step_id(
    id: &str,
    step_ids: &mut HashSet<String>,
    pointer: &str,
) -> Result<(), ContractError> {
    require_id(id, pointer)?;
    if !step_ids.insert(id.to_owned()) {
        return Err(invalid(
            "ID_DUPLICATE",
            "Step IDs must be unique within a diagram, including conditional branches",
            pointer,
        ));
    }
    Ok(())
}

fn validate_optional_source(
    source: Option<&crate::SourceRef>,
    context: &ReviewContext,
    repository: &Path,
    control: &OperationControl,
    pointer: &str,
) -> Result<(), ContractError> {
    if let Some(source) = source {
        read_source(repository, context, source, control).map_err(|mut error| {
            error.pointer = format!("{pointer}{}", error.pointer);
            error
        })?;
    }
    Ok(())
}

fn validate_code_source(
    source: Option<&crate::SourceRef>,
    context: &ReviewContext,
    repository: &Path,
    control: &OperationControl,
    pointer: &str,
) -> Result<(), ContractError> {
    let source = source.ok_or_else(|| {
        invalid(
            "STEP_SOURCE_REQUIRED",
            "Message, return and self-call steps need a pinned source reference",
            pointer,
        )
    })?;
    read_source(repository, context, source, control).map_err(|mut error| {
        error.pointer = format!("{pointer}{}", error.pointer);
        error
    })?;
    Ok(())
}

fn require_id(id: &str, pointer: &str) -> Result<(), ContractError> {
    crate::findings::require_id(id, pointer)
}

fn require_text(text: &str, pointer: &str) -> Result<(), ContractError> {
    if text.trim().is_empty() {
        return Err(invalid(
            "DIAGRAM_CONTENT_INVALID",
            "Expected nonempty diagram text",
            pointer,
        ));
    }
    Ok(())
}

fn invalid(code: &str, message: &str, pointer: &str) -> ContractError {
    ContractError::new(code, message, pointer)
}

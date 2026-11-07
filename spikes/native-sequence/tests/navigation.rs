use bodh_sequence_spike::{Scene, demo};

#[test]
fn clicking_an_interaction_resolves_its_exact_immutable_source() {
    let flow = demo();
    let scene = Scene::layout(&flow);
    let hit = scene.hit(250.0, 115.0).expect("first message label");
    assert_eq!(hit, 0);
    assert_eq!(
        flow.interactions()[hit].source.text(),
        "    let token = request.token();"
    );
    assert_eq!(
        scene.hit(5.0, 115.0),
        None,
        "blank space cannot select code"
    );
}

#[test]
fn every_visible_hit_resolves_in_order_across_conditionals_and_stress_layouts() {
    for flow in [
        demo(),
        bodh_sequence_spike::stress(12, 120),
        bodh_sequence_spike::stress(24, 1000),
    ] {
        let scene = Scene::layout(&flow);
        let interactions = flow.interactions();
        assert_eq!(scene.rows.len(), interactions.len());
        for (index, row) in scene.rows.iter().enumerate() {
            assert_eq!(
                scene.hit(row.rect.x + row.rect.w / 2., row.rect.y + 10.),
                Some(index)
            );
            assert_eq!(
                scene.hit(row.rect.x + 1., row.rect.y + row.rect.h + 1.),
                None
            );
            let source = &interactions[index].source;
            assert!(source.start > 0 && source.end <= bodh_sequence_spike::SOURCE.lines().count());
            assert!(!source.text().is_empty());
        }
    }
    let flow = demo();
    assert_eq!(
        flow.interactions()[1].source.text(),
        "    token.check_expiry()?;"
    );
    assert!(flow.interactions()[1].source.changed());
    assert!(!flow.interactions()[0].source.changed());
    assert_eq!(
        flow.interactions()[4].source.text(),
        "    } else {\n        return Err(Error::Stale);"
    );
}

#[test]
fn alternatives_share_one_conditional_frame() {
    let scene = Scene::layout(&demo());
    let alt = scene
        .frames
        .iter()
        .find(|f| f.label == "alt · snapshot result")
        .expect("one encompassing alt frame");
    for index in [3, 4] {
        let row = &scene.rows[index];
        assert!(alt.rect.contains(row.rect.x + 1., row.rect.y + 1.));
    }
}

#[test]
fn self_call_on_the_last_participant_stays_inside_the_scrollable_scene() {
    let mut flow = demo();
    let mut last = flow.interactions()[1].clone();
    last.from = 2;
    last.to = 2;
    flow.steps
        .push(bodh_sequence_spike::Step::Interaction(last));
    let scene = Scene::layout(&flow);
    let row = scene.rows.last().unwrap();
    assert!(row.rect.x + row.rect.w <= scene.width);
}

#[test]
fn long_participant_and_interaction_labels_expand_their_scene_rows() {
    let mut flow = demo();
    flow.participants[0] = "A participant label long enough to wrap onto several lines".into();
    let mut interaction = flow.interactions()[0].clone();
    interaction.label =
        "A long interaction description that must remain inside its hit row when it wraps".into();
    flow.steps[0] = bodh_sequence_spike::Step::Interaction(interaction);

    let scene = Scene::layout(&flow);
    assert!(scene.participant_height > 65.);
    assert!(scene.rows[0].rect.y >= 16. + scene.participant_height + 9.);
    assert!(scene.rows[0].rect.h > 90.);
    assert!(scene.rows[0].rect.x + scene.rows[0].rect.w <= scene.width);
}

#[test]
fn viewport_culling_keeps_partial_rows_and_original_source_identity() {
    use bodh_sequence_spike::{Rect, stress};
    let flow = stress(12, 120);
    let scene = Scene::layout(&flow);
    let visible: Vec<_> = scene
        .visible_rows(Rect {
            x: 0.,
            y: 90.,
            w: 840.,
            h: 400.,
        })
        .map(|r| r.interaction)
        .collect();
    assert_eq!(visible, [0, 1]);
    let last = scene.rows.last().unwrap();
    let visible: Vec<_> = scene
        .visible_rows(last.rect)
        .map(|r| r.interaction)
        .collect();
    assert_eq!(visible, [119]);
    assert_eq!(flow.interactions()[119].source.start, 17);
}

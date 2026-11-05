use super::*;

/// Builds native labels and hit regions only after the actual clipped viewport is known.
/// Scrolling runs prepaint again, so it cannot leave stale virtualized hit targets.
pub struct SequenceDiagram<T> {
    pub flow: std::rc::Rc<Flow>,
    pub scene: std::rc::Rc<Scene>,
    pub selected: usize,
    pub palette: Palette,
    pub owner: WeakEntity<T>,
    pub select: fn(&mut T, usize, &mut Window, &mut Context<T>),
    pub interaction_ids: std::rc::Rc<Vec<String>>,
    pub changed_interactions: Option<std::rc::Rc<std::collections::HashSet<usize>>>,
}
impl<T: 'static> IntoElement for SequenceDiagram<T> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl<T: 'static> Element for SequenceDiagram<T> {
    type RequestLayoutState = ();
    type PrepaintState = AnyElement;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = px(self.scene.width).into();
        style.size.height = px(self.scene.height).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let mask = window.content_mask().bounds;
        let viewport = Rect {
            x: f32::from(mask.left() - bounds.left()),
            y: f32::from(mask.top() - bounds.top()),
            w: f32::from(mask.size.width),
            h: f32::from(mask.size.height),
        };
        let flow = &self.flow;
        let scene = &self.scene;
        let selected = self.selected;
        let palette = self.palette;
        let changed_interactions = self.changed_interactions.clone();
        let interactions = flow.interactions();
        let mut diagram = div()
            .relative()
            .flex_shrink_0()
            .w(px(scene.width))
            .h(px(scene.height));
        let lines_scene = scene.clone();
        let lines_flow = flow.clone();
        let canvas_changed_interactions = changed_interactions.clone();
        diagram = diagram.child(
            canvas(
                move |_, _, _| (),
                move |bounds, _, window, _| {
                    let mask = window.content_mask().bounds;
                    let viewport = Rect {
                        x: f32::from(mask.left() - bounds.left()),
                        y: f32::from(mask.top() - bounds.top()),
                        w: f32::from(mask.size.width),
                        h: f32::from(mask.size.height),
                    };
                    // Clip before tessellation: a paint mask alone does not avoid building enormous dashed paths.
                    let lifeline_start = 16. + lines_scene.participant_height + 9.;
                    let top = (lifeline_start
                        + ((viewport.y - lifeline_start).max(0.) / 11.).floor() * 11.)
                        .max(lifeline_start);
                    let bottom = lines_scene.height.min(viewport.y + viewport.h);
                    for &x in &lines_scene.columns {
                        if x < viewport.x || x > viewport.x + viewport.w || top >= bottom {
                            continue;
                        }
                        stroke(
                            window,
                            &[(x, top), (x, bottom)],
                            bounds.origin,
                            palette.muted,
                            1.,
                            true,
                        );
                    }
                    let interactions = lines_flow.interactions();
                    for row in lines_scene.visible_rows(viewport) {
                        let i = interactions[row.interaction];
                        if i.kind == Kind::Note {
                            continue;
                        }
                        let a = lines_scene.columns[i.from];
                        let b = lines_scene.columns[i.to];
                        let y = row.arrow_y;
                        let changed = canvas_changed_interactions.as_ref().map_or_else(
                            || i.source.changed(),
                            |set| set.contains(&row.interaction),
                        );
                        let color = if changed {
                            palette.accent
                        } else {
                            palette.text
                        };
                        let width = if changed { 3. } else { 1. };
                        let (end, direction) = if i.kind == Kind::SelfCall {
                            stroke(
                                window,
                                &[(a, y - 14.), (a + 100., y - 14.), (a + 100., y), (a, y)],
                                bounds.origin,
                                color,
                                width,
                                false,
                            );
                            (a, 1.)
                        } else {
                            stroke(
                                window,
                                &[(a, y), (b, y)],
                                bounds.origin,
                                color,
                                width,
                                i.kind == Kind::Return,
                            );
                            (b, if b > a { -1. } else { 1. })
                        };
                        stroke(
                            window,
                            &[
                                (end + direction * 8., y - 5.),
                                (end, y),
                                (end + direction * 8., y + 5.),
                            ],
                            bounds.origin,
                            color,
                            width,
                            false,
                        );
                    }
                },
            )
            .absolute()
            .size_full(),
        );
        for (index, label) in flow.participants.iter().enumerate() {
            if !(Rect {
                x: scene.columns[index] - 100.,
                y: 16.,
                w: 200.,
                h: scene.participant_height,
            })
            .intersects(viewport)
            {
                continue;
            }
            diagram = diagram.child(
                div()
                    .absolute()
                    .left(px(scene.columns[index] - 100.))
                    .top(px(16.))
                    .w(px(200.))
                    .h(px(scene.participant_height))
                    .p_2()
                    .bg(rgb(palette.panel))
                    .border_1()
                    .border_color(rgb(palette.muted))
                    .child(label.clone()),
            );
        }
        for frame in &scene.frames {
            if !frame.rect.intersects(viewport) {
                continue;
            }
            diagram = diagram.child(
                div()
                    .absolute()
                    .left(px(frame.rect.x))
                    .top(px(frame.rect.y))
                    .w(px(frame.rect.w))
                    .h(px(frame.rect.h))
                    .when(frame.branch, |d| d.border_t_1())
                    .when(!frame.branch, |d| d.border_1())
                    .border_color(rgb(palette.muted))
                    .child(
                        div()
                            .px_2()
                            .text_color(rgb(palette.muted))
                            .child(frame.label.clone()),
                    ),
            );
        }
        for row in scene.visible_rows(viewport) {
            let index = row.interaction;
            let i = interactions[index];
            let changed = changed_interactions
                .as_ref()
                .map_or_else(|| i.source.changed(), |set| set.contains(&index));
            let stable_id = self
                .interaction_ids
                .get(index)
                .cloned()
                .unwrap_or_else(|| index.to_string());
            let owner = self.owner.clone();
            let select = self.select;
            diagram = diagram.child(
                div()
                    .id(ElementId::Name(stable_id.into()))
                    .absolute()
                    .left(px(row.rect.x))
                    .top(px(row.rect.y))
                    .w(px(row.rect.w))
                    .h(px(row.rect.h))
                    .border_2()
                    .border_color(if index == selected {
                        rgb(palette.text).into()
                    } else {
                        transparent_black()
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .m_1()
                            .px_1()
                            .bg(rgb(if i.kind == Kind::Note {
                                palette.panel
                            } else {
                                palette.bg
                            }))
                            .text_color(rgb(if changed {
                                palette.accent
                            } else {
                                palette.text
                            }))
                            .child(format!(
                                "{}{} · {}",
                                index + 1,
                                if changed { " [changed]" } else { "" },
                                i.label
                            )),
                    )
                    .on_click(move |_, window, app| {
                        let _ = owner.update(app, |this, cx| {
                            select(this, index, window, cx);
                        });
                    }),
            );
        }
        let mut element = diagram.into_any_element();
        element.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::Definite(bounds.size.height),
            ),
            window,
            cx,
        );
        element.prepaint_at(bounds.origin, window, cx);
        element
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        element: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

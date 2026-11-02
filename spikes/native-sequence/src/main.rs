use bodh_sequence_spike::{Flow, Kind, SOURCE, Scene, demo, stress};
use gpui::{prelude::*, *};

struct Demo {
    flows: Vec<Flow>,
    active: usize,
    selected: Vec<usize>,
    scroll: Vec<ScrollHandle>,
    inspector_scroll: Vec<ScrollHandle>,
    dark: bool,
    inspector_width: f32,
    dragging: bool,
    focus: FocusHandle,
}

#[derive(Clone, Copy)]
struct Palette {
    bg: u32,
    panel: u32,
    text: u32,
    muted: u32,
    accent: u32,
}
impl Demo {
    fn reveal(&self) {
        let scene = Scene::layout(&self.flows[self.active]);
        let row = &scene.rows[self.selected[self.active]];
        self.scroll[self.active].set_offset(point(
            px(-(row.rect.x - 40.).max(0.)),
            px(-(row.rect.y - 90.).max(0.)),
        ));
    }
    fn palette(&self) -> Palette {
        if self.dark {
            Palette {
                bg: 0x181a1d,
                panel: 0x22252a,
                text: 0xe5e7eb,
                muted: 0x949ba5,
                accent: 0x8bb8ff,
            }
        } else {
            Palette {
                bg: 0xffffff,
                panel: 0xf0f2f5,
                text: 0x20242a,
                muted: 0x626a75,
                accent: 0x175db5,
            }
        }
    }
    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected[self.active] = index;
        self.inspector_scroll[self.active].set_offset(point(px(0.), px(0.)));
        cx.notify();
    }
}
fn stroke(
    window: &mut Window,
    points: &[(f32, f32)],
    origin: Point<Pixels>,
    color: u32,
    width: f32,
    dashed: bool,
) {
    let mut path = PathBuilder::stroke(px(width));
    if dashed {
        path = path.dash_array(&[px(6.), px(5.)]);
    }
    for (i, &(x, y)) in points.iter().enumerate() {
        let p = origin + point(px(x), px(y));
        if i == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, rgb(color));
    }
}
impl Render for Demo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette();
        let flow = &self.flows[self.active];
        let scene = Scene::layout(flow);
        let selected = self.selected[self.active];
        let interactions = flow.interactions();
        let source = interactions[selected].source.clone();
        let explanation = interactions[selected].explanation.clone();
        let mut diagram = div()
            .relative()
            .flex_shrink_0()
            .w(px(scene.width))
            .h(px(scene.height));
        let lines_scene = scene.clone();
        let lines_flow = flow.clone();
        diagram = diagram.child(
            canvas(
                move |_, _, _| (),
                move |bounds, _, window, _| {
                    for &x in &lines_scene.columns {
                        stroke(
                            window,
                            &[(x, 65.), (x, lines_scene.height)],
                            bounds.origin,
                            palette.muted,
                            1.,
                            true,
                        );
                    }
                    let interactions = lines_flow.interactions();
                    for row in &lines_scene.rows {
                        let i = interactions[row.interaction];
                        if i.kind == Kind::Note {
                            continue;
                        }
                        let a = lines_scene.columns[i.from];
                        let b = lines_scene.columns[i.to];
                        let y = row.arrow_y;
                        let color = if i.source.changed() {
                            palette.accent
                        } else {
                            palette.text
                        };
                        let width = if i.source.changed() { 3. } else { 1. };
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
            diagram = diagram.child(
                div()
                    .absolute()
                    .left(px(scene.columns[index] - 100.))
                    .top(px(16.))
                    .w(px(200.))
                    .p_2()
                    .bg(rgb(palette.panel))
                    .border_1()
                    .border_color(rgb(palette.muted))
                    .child(label.clone()),
            );
        }
        for frame in &scene.frames {
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
        for row in &scene.rows {
            let index = row.interaction;
            let i = interactions[index];
            let changed = i.source.changed();
            diagram = diagram.child(
                div()
                    .id(("interaction", index))
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus);
                        this.select(index, cx);
                    })),
            );
        }
        let mut toolbar = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .p_2()
            .bg(rgb(palette.panel));
        for (index, flow) in self.flows.iter().enumerate() {
            toolbar = toolbar.child(
                div()
                    .id(("flow", index))
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(rgb(if index == self.active {
                        palette.text
                    } else {
                        palette.muted
                    }))
                    .cursor_pointer()
                    .child(flow.title.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.active = index;
                        window.focus(&this.focus);
                        cx.notify();
                    })),
            );
        }
        toolbar = toolbar.child(
            div()
                .id("theme")
                .px_2()
                .py_1()
                .border_1()
                .cursor_pointer()
                .child("Theme [T]")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.dark = !this.dark;
                    cx.notify();
                })),
        );
        div().id("spike").track_focus(&self.focus).size_full().flex().flex_col().bg(rgb(palette.bg)).text_color(rgb(palette.text)).font_family("DejaVu Sans Mono").text_size(px(14.))
            .on_key_down(cx.listener(|this,event: &KeyDownEvent,window,cx| {
                let count = this.flows[this.active].interactions().len();
                let current = this.selected[this.active];
                match event.keystroke.key.as_str() {
                    "down" | "right" => {this.select((current+1)%count,cx);this.reveal();},
                    "up" | "left" => {this.select((current+count-1)%count,cx);this.reveal();},
                    "t" => {this.dark=!this.dark;cx.notify();},
                    "tab" => {this.active=(this.active+1)%this.flows.len();cx.notify();},
                    "[" => {this.inspector_width=(this.inspector_width-40.).max(260.);cx.notify();},
                    "]" => {this.inspector_width=(this.inspector_width+40.).min(f32::from(window.viewport_size().width)*0.7);cx.notify();},
                    _ => {}
                }
            }))
            .on_mouse_move(cx.listener(|this,event: &MouseMoveEvent,window,cx| {
                if this.dragging && event.pressed_button == Some(MouseButton::Left) {
                    let width = f32::from(window.viewport_size().width);
                    this.inspector_width=(width-f32::from(event.position.x)).clamp(260.,(width*0.7).max(260.));
                    cx.notify();
                } else {this.dragging=false;}
            }))
            .on_mouse_up(MouseButton::Left,cx.listener(|this,_,_,_|this.dragging=false))
            .child(toolbar)
            .child(div().p_2().text_color(rgb(palette.muted)).child("DISPOSABLE SPIKE · Click / arrows: source · Tab: flow · T: theme · Drag divider / [ ]: pane width"))
            .child(div().flex().flex_1().min_h_0().overflow_hidden()
                .child(div().id(("diagram-scroll",self.active)).track_scroll(&self.scroll[self.active]).overflow_scroll().flex_1().min_w_0().h_full().child(diagram))
                .child(div().id("divider").w(px(8.)).h_full().flex_shrink_0().bg(rgb(palette.muted)).cursor_col_resize().on_mouse_down(MouseButton::Left,cx.listener(|this,_,_,_|this.dragging=true)))
                .child(div().id(("inspector",self.active)).track_scroll(&self.inspector_scroll[self.active]).w(px(self.inspector_width)).h_full().flex_shrink_0().overflow_scroll().bg(rgb(palette.panel)).p_3()
                    .child(div().mb_2().child(format!("Pinned fixture · review.rs:{}–{}",source.start,source.end)))
                    .child(div().mb_2().child(explanation))
                    .child(div().mb_2().text_color(rgb(palette.muted)).child("Embedded at compile time · head fixture · line 14 changed"))
                    .children(SOURCE.lines().enumerate().map(|(index,line)| {
                        let relevant = (source.start..=source.end).contains(&(index+1));
                        div().flex().min_w(px(640.)).border_l_2().border_color(if relevant {rgb(palette.text).into()} else {transparent_black()})
                            .bg(rgb(if relevant {palette.bg} else {palette.panel}))
                            .child(div().w(px(40.)).flex_shrink_0().text_color(rgb(palette.muted)).child(format!("{:>3}",index+1)))
                            .child(div().whitespace_nowrap().child(line.to_string()))
                    }))))
            .child(div().p_2().border_t_1().child(format!("Selected {} · {} · {} × {} logical px · [changed] + heavy stroke = direct diff intersection",selected+1,if self.focus.is_focused(_window) {"keyboard focus"} else {"pointer focus"},scene.width,scene.height)))
    }
}
fn main() {
    Application::new().run(|cx| {
        cx.open_window(
            WindowOptions {
                app_id: Some("bodh-sequence-spike".into()),
                titlebar: Some(TitlebarOptions {
                    title: Some("Bodh · native sequence spike #2".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(640.), px(480.))),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus);
                    Demo {
                        flows: vec![demo(), stress(12, 120), stress(24, 1000)],
                        active: 0,
                        selected: vec![0; 3],
                        scroll: (0..3).map(|_| ScrollHandle::new()).collect(),
                        inspector_scroll: (0..3).map(|_| ScrollHandle::new()).collect(),
                        dark: true,
                        inspector_width: 420.,
                        dragging: false,
                        focus,
                    }
                })
            },
        )
        .expect("open native spike window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}

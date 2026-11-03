use bodh_sequence_spike::{Flow, Kind, Rect, SOURCE, Scene, demo, stress};
use gpui::{prelude::*, *};

mod diagram;
mod profile;

struct FlowState {
    flow: std::rc::Rc<Flow>,
    scene: std::rc::Rc<Scene>,
    selected: usize,
    scroll: ScrollHandle,
    inspector_scroll: ScrollHandle,
}

impl FlowState {
    fn new(flow: Flow) -> Self {
        let scene = std::rc::Rc::new(Scene::layout(&flow));
        Self {
            flow: std::rc::Rc::new(flow),
            scene,
            selected: 0,
            scroll: ScrollHandle::new(),
            inspector_scroll: ScrollHandle::new(),
        }
    }
}

struct Demo {
    flows: Vec<FlowState>,
    active: usize,
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
        let scene = &self.flows[self.active].scene;
        let row = &scene.rows[self.flows[self.active].selected];
        self.flows[self.active].scroll.set_offset(point(
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
        if self.flows[self.active].selected == index {
            return;
        }
        self.flows[self.active].selected = index;
        self.flows[self.active]
            .inspector_scroll
            .set_offset(point(px(0.), px(0.)));
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
        let inspector_width = self.inspector_width.clamp(
            260.,
            (f32::from(_window.viewport_size().width) * 0.7).max(260.),
        );
        let flow = &self.flows[self.active].flow;
        let scene = &self.flows[self.active].scene;
        let selected = self.flows[self.active].selected;
        let interactions = flow.interactions();
        let source = interactions[selected].source.clone();
        let explanation = interactions[selected].explanation.clone();
        let diagram = diagram::SequenceDiagram {
            flow: self.flows[self.active].flow.clone(),
            scene: self.flows[self.active].scene.clone(),
            selected,
            palette,
            owner: cx.entity().downgrade(),
        };
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
                    .child(flow.flow.title.clone())
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
                let count = this.flows[this.active].flow.interactions().len();
                let current = this.flows[this.active].selected;
                match event.keystroke.key.as_str() {
                    "down" | "right" => {this.select((current+1)%count,cx);this.reveal();},
                    "up" | "left" => {this.select((current+count-1)%count,cx);this.reveal();},
                    "t" => {this.dark = !this.dark;cx.notify();},
                    "tab" => {this.active=(this.active+1)%this.flows.len();cx.notify();},
                    "[" => {this.inspector_width=(this.inspector_width-40.).max(260.);cx.notify();},
                    "]" => {this.inspector_width=(this.inspector_width+40.).min(f32::from(window.viewport_size().width)*0.7);cx.notify();},
                    _ => {}
                }
            }))
            .on_mouse_move(cx.listener(|this,event: &MouseMoveEvent,window,cx| {
                if this.dragging && event.pressed_button == Some(MouseButton::Left) {
                    let width = f32::from(window.viewport_size().width);
                    let next=(width-f32::from(event.position.x)).clamp(260.,(width*0.7).max(260.));
                    if (next-this.inspector_width).abs() >= 1. { this.inspector_width=next; cx.notify(); }
                } else {this.dragging=false;}
            }))
            .on_mouse_up(MouseButton::Left,cx.listener(|this,_,_,_|this.dragging=false))
            .child(toolbar)
            .child(div().p_2().text_color(rgb(palette.muted)).child("DISPOSABLE SPIKE · Click / arrows: source · Tab: flow · T: theme · Drag divider / [ ]: pane width"))
            .child(div().flex().flex_1().min_h_0().overflow_hidden()
                .child(div().id(("diagram-scroll",self.active)).track_scroll(&self.flows[self.active].scroll).overflow_scroll().flex_1().min_w_0().h_full().child(diagram))
                .child(div().id("divider").w(px(8.)).h_full().flex_shrink_0().bg(rgb(palette.muted)).cursor_col_resize().on_mouse_down(MouseButton::Left,cx.listener(|this,_,_,_|this.dragging=true)))
                .child(div().id(("inspector",self.active)).track_scroll(&self.flows[self.active].inspector_scroll).w(px(inspector_width)).h_full().flex_shrink_0().overflow_scroll().bg(rgb(palette.panel)).p_3()
                    .child(div().mb_2().child(format!("Pinned fixture · review.rs:{}–{}",source.start,source.end)))
                    .child(div().mb_2().text_color(rgb(palette.muted)).child("Embedded at compile time · head fixture · line 14 changed"))
                    .children(SOURCE.lines().enumerate().filter(|(index,_)| (source.start.saturating_sub(3)..source.end+2).contains(index)).map(|(index,line)| {
                        let relevant = (source.start..=source.end).contains(&(index+1));
                        div().flex().min_w(px(640.)).border_l_2().border_color(if relevant {rgb(palette.text).into()} else {transparent_black()})
                            .bg(rgb(if relevant {palette.bg} else {palette.panel}))
                            .child(div().w(px(40.)).flex_shrink_0().text_color(rgb(palette.muted)).child(format!("{:>3}",index+1)))
                            .child(div().whitespace_nowrap().child(line.to_string()))
                    }))
                    .child(div().mt_3().child(explanation))))
            .child(div().p_2().border_t_1().child(format!("Selected {} · {} · {} × {} logical px · [changed] + heavy stroke = direct diff intersection",selected+1,if self.focus.is_focused(_window) {"keyboard focus"} else {"pointer focus"},scene.width,scene.height)))
    }
}
fn main() {
    let diagnostic = std::env::args().any(|arg| arg == "--profile");
    Application::new().run(move |cx| {
        let handle = cx
            .open_window(
                WindowOptions {
                    app_id: Some(
                        if diagnostic {
                            "bodh-sequence-diagnostic"
                        } else {
                            "bodh-sequence-spike"
                        }
                        .into(),
                    ),
                    focus: !diagnostic,
                    show: !diagnostic,
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
                            flows: vec![demo(), stress(12, 120), stress(24, 1000)]
                                .into_iter()
                                .map(FlowState::new)
                                .collect(),
                            active: 0,
                            dark: true,
                            inspector_width: 420.,
                            dragging: false,
                            focus,
                        }
                    })
                },
            )
            .expect("open native spike window");
        if diagnostic {
            profile::run(handle, cx);
        }
        cx.on_window_closed(|cx| cx.quit()).detach();
        if !diagnostic {
            cx.activate(true);
        }
    });
}

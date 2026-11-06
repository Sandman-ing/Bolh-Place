//! Холст в окне: бесконечное поле с сеткой и карточками (выпуск 0.2).
//!
//! Вся логика правок — в `bolh_core::canvas`; здесь мышь, клавиши и
//! рисование. Изменения уходят наружу событием [`CanvasEvent::Changed`]:
//! вкладка кладёт их в файл и сохраняет через ядро.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use bolh_core::canvas::{self, Editor, Guide, Point, Rect, View};
use bolh_core::format::Canvas;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Selectable as _, Sizable as _, h_flex};
use gpui_kit::{
    App, AppContext as _, Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, KeyUpEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PinchEvent, Pixels, Render,
    ScrollDelta, ScrollEvent, ScrollSource, SharedString, Styled, Subscription,
    TestSupportExt as _, Window, canvas as paint_canvas, div, fill, point,
    prelude::FluentBuilder as _, px, size,
};

use crate::i18n::tr;

pub const CONTEXT: &str = "Canvas";

/// Насколько близко к соседу объект прилипает, в точках экрана.
const SNAP_SCREEN_PX: f64 = 6.0;
/// Размер уголка для изменения размера, в точках экрана.
const HANDLE_PX: f64 = 10.0;
/// Масштаб за один щелчок колеса.
const WHEEL_ZOOM: f64 = 1.1;
/// Мельче этого масштаба текст карточек не читается (мельче 6 точек), и
/// холст рисуется «обзором»: все карточки одним проходом, без раскладки.
/// Так 5000 объектов, видимые разом, укладываются в кадр.
const OVERVIEW_ZOOM: f64 = 0.35;
/// Сетка не гуще этого числа точек на экране: каждая точка — отдельная
/// фигура, и тысячи их съедали кадр.
const GRID_MAX_DOTS: f64 = 1500.0;
/// Рамка карточки в точках экрана — как у `.border_1()`. Изнутри карточки
/// её не видно, а мерить нужно всю высоту: иначе мерка всегда на две точки
/// меньше настоящей, и окно перерисовывалось бы без конца.
const CARD_BORDER_PX: f64 = 1.0;
/// Мельче этого масштаба полоски текста в обзоре тоньше полутора точек —
/// не видны, а фигур втрое больше.
const OVERVIEW_TEXT_ZOOM: f64 = 0.15;

gpui_kit::actions!(
    canvas,
    [
        Undo,
        Redo,
        Delete,
        Copy,
        Cut,
        Paste,
        Duplicate,
        SelectAll,
        Deselect,
        BringToFront,
        SendToBack,
        BringForward,
        SendBackward,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ZoomFit,
        EditText,
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        MoveLeftGrid,
        MoveRightGrid,
        MoveUpGrid,
        MoveDownGrid,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("ctrl-z", Undo, c),
        KeyBinding::new("ctrl-y", Redo, c),
        KeyBinding::new("ctrl-shift-z", Redo, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("backspace", Delete, c),
        KeyBinding::new("ctrl-c", Copy, c),
        KeyBinding::new("ctrl-x", Cut, c),
        KeyBinding::new("ctrl-v", Paste, c),
        KeyBinding::new("ctrl-d", Duplicate, c),
        KeyBinding::new("ctrl-a", SelectAll, c),
        KeyBinding::new("escape", Deselect, c),
        KeyBinding::new("ctrl-shift-]", BringToFront, c),
        KeyBinding::new("ctrl-shift-[", SendToBack, c),
        KeyBinding::new("ctrl-]", BringForward, c),
        KeyBinding::new("ctrl-[", SendBackward, c),
        KeyBinding::new("ctrl-=", ZoomIn, c),
        KeyBinding::new("ctrl-+", ZoomIn, c),
        KeyBinding::new("ctrl--", ZoomOut, c),
        KeyBinding::new("ctrl-0", ZoomReset, c),
        KeyBinding::new("shift-1", ZoomFit, c),
        KeyBinding::new("enter", EditText, c),
        KeyBinding::new("left", MoveLeft, c),
        KeyBinding::new("right", MoveRight, c),
        KeyBinding::new("up", MoveUp, c),
        KeyBinding::new("down", MoveDown, c),
        // С Shift — на шаг сетки.
        KeyBinding::new("shift-left", MoveLeftGrid, c),
        KeyBinding::new("shift-right", MoveRightGrid, c),
        KeyBinding::new("shift-up", MoveUpGrid, c),
        KeyBinding::new("shift-down", MoveDownGrid, c),
    ]);
}

pub enum CanvasEvent {
    /// Содержимое холста поменялось — пора сохранять.
    Changed,
}

/// Что сейчас делает мышь.
#[derive(Debug, Clone, PartialEq)]
enum Gesture {
    None,
    /// Тащим выделенное. `applied` — сколько уже сдвинули с начала.
    Move {
        /// Карточка под мышью и была ли нажата клавиша добавления.
        pressed: String,
        add: bool,
        start: Point,
        start_bounds: Rect,
        applied: (f64, f64),
    },
    /// Тянем уголок объекта. `corner`: 0 — левый верхний, далее по часовой.
    Resize {
        id: String,
        start: Point,
        original: Rect,
        corner: u8,
    },
    /// Рамка выделения от точки `start`.
    Marquee {
        start: Point,
        now: Point,
        add: bool,
    },
    /// Сдвиг поля средней кнопкой или с пробелом.
    Pan {
        last: Point,
    },
}

pub struct CanvasView {
    editor: Editor,
    view: View,
    view_initialized: bool,
    grid: bool,
    gesture: Gesture,
    guides: Vec<Guide>,
    space_held: bool,
    /// Карточка, текст которой сейчас правится, и поле для правки.
    editing: Option<(String, gpui_kit::Entity<TextareaState>, Subscription)>,
    read_only: bool,
    /// Поставить фокус в поле текста при следующей отрисовке. Сразу нельзя:
    /// щелчок по холсту после нашего обработчика забирает фокус себе.
    focus_editing: bool,
    /// Высоты карточек после раскладки, в точках холста: длинный текст
    /// делает карточку выше заданной. Собираются при раскладке кадра и
    /// передаются ядру в начале следующего — только для показа и мыши.
    measured: Rc<RefCell<Vec<(String, f64)>>>,
    /// Где на экране поле: нужно, чтобы перевести точку окна в точку холста.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    focus_handle: FocusHandle,
}

impl EventEmitter<CanvasEvent> for CanvasView {}

impl Focusable for CanvasView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn f(p: Pixels) -> f64 {
    f64::from(f32::from(p))
}

impl CanvasView {
    pub fn new(canvas: Canvas, read_only: bool, cx: &mut Context<Self>) -> Self {
        CanvasView {
            editor: Editor::new(canvas),
            view: View::default(),
            view_initialized: false,
            grid: true,
            gesture: Gesture::None,
            guides: Vec::new(),
            space_held: false,
            editing: None,
            read_only,
            focus_editing: false,
            measured: Rc::new(RefCell::new(Vec::new())),
            bounds: Rc::new(Cell::new(Bounds::default())),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    pub fn view(&self) -> View {
        self.view
    }

    /// Холст со всеми правками — для сохранения.
    pub fn canvas(&self) -> Canvas {
        self.editor.to_canvas()
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    /// После расхождения холст живёт копией со своим id.
    pub fn set_canvas_id(&mut self, id: bolh_core::Uuid) {
        self.editor.set_id(id);
    }

    /// Пришла версия из облака: показать её, сбросив отмену.
    pub fn replace_canvas(&mut self, canvas: Canvas, cx: &mut Context<Self>) {
        self.editing = None;
        self.editor.replace_canvas(canvas);
        cx.notify();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(CanvasEvent::Changed);
        cx.notify();
    }

    fn origin(&self) -> (f64, f64) {
        let b = self.bounds.get();
        (f(b.origin.x), f(b.origin.y))
    }

    /// Точка окна → точка на поле (относительно его левого верхнего угла).
    fn local(&self, p: gpui_kit::Point<Pixels>) -> Point {
        let (ox, oy) = self.origin();
        Point {
            x: f(p.x) - ox,
            y: f(p.y) - oy,
        }
    }

    fn grid_step(&self) -> Option<f64> {
        self.grid.then_some(canvas::GRID)
    }

    fn size(&self) -> (f64, f64) {
        let b = self.bounds.get();
        (f(b.size.width), f(b.size.height))
    }

    /// Уголок выделенного объекта под точкой экрана — для изменения размера.
    fn handle_at(&self, local: Point) -> Option<(String, u8, Rect)> {
        if self.editor.selection().len() != 1 {
            return None;
        }
        let id = &self.editor.selection()[0];
        let r = self.editor.rect_of(id)?;
        let a = self.view.to_screen(Point { x: r.x, y: r.y });
        let b = self.view.to_screen(Point {
            x: r.right(),
            y: r.bottom(),
        });
        let corners = [(a.x, a.y), (b.x, a.y), (b.x, b.y), (a.x, b.y)];
        corners.iter().enumerate().find_map(|(i, (x, y))| {
            ((local.x - x).abs() <= HANDLE_PX && (local.y - y).abs() <= HANDLE_PX)
                .then(|| (id.clone(), i as u8, r))
        })
    }

    // ---- Мышь -----------------------------------------------------------

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let local = self.local(ev.position);
        let world = self.view.to_world(local);
        // Щелчок внутри карточки, текст которой правится, — это поле ввода:
        // курсор в нужное место, выделение текста. Правку не прерываем.
        if let Some((id, ..)) = &self.editing
            && self.editor.hit(world) == Some(id.as_str())
        {
            // Щелчок ниже текста холст забрал бы фокус себе — вернём его полю.
            self.focus_editing = true;
            cx.notify();
            return;
        }
        self.focus_handle.focus(window, cx);
        if ev.button == MouseButton::Middle || (ev.button == MouseButton::Left && self.space_held) {
            self.gesture = Gesture::Pan { last: local };
            return;
        }
        if ev.button != MouseButton::Left {
            return;
        }
        self.finish_editing(window, cx);
        if self.read_only {
            return;
        }
        if let Some((id, corner, original)) = self.handle_at(local) {
            self.editor.checkpoint();
            self.gesture = Gesture::Resize {
                id,
                start: world,
                original,
                corner,
            };
            return;
        }
        let add = ev.modifiers.control || ev.modifiers.shift;
        match self.editor.hit(world).map(str::to_string) {
            Some(id) => {
                if ev.click_count >= 2 {
                    self.editor.select_only(&id);
                    self.start_editing(&id, window, cx);
                    return;
                }
                if add {
                    self.editor.toggle_selected(&id);
                } else if !self.editor.is_selected(&id) {
                    self.editor.select_only(&id);
                }
                if self.editor.is_selected(&id)
                    && let Some(b) = self.editor.selection_bounds()
                {
                    self.editor.checkpoint();
                    self.gesture = Gesture::Move {
                        pressed: id.clone(),
                        add,
                        start: world,
                        start_bounds: b,
                        applied: (0.0, 0.0),
                    };
                }
            }
            None => {
                if ev.click_count >= 2 {
                    let id = self.editor.add_card(world, self.grid_step());
                    self.changed(cx);
                    self.start_editing(&id, window, cx);
                    return;
                }
                if !add {
                    self.editor.clear_selection();
                }
                self.gesture = Gesture::Marquee {
                    start: world,
                    now: world,
                    add,
                };
            }
        }
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let local = self.local(ev.position);
        let world = self.view.to_world(local);
        // К чему прилипать — только видимые соседи; Alt — без прилипания.
        let snapping = !ev.modifiers.alt
            && matches!(self.gesture, Gesture::Move { .. } | Gesture::Resize { .. });
        let threshold = SNAP_SCREEN_PX / self.view.zoom;
        let grid = self.grid_step();
        let others = if snapping {
            let (w, h) = self.size();
            self.editor.unselected_rects_in(self.view.visible(w, h))
        } else {
            Vec::new()
        };
        match &mut self.gesture {
            Gesture::None => return,
            Gesture::Pan { last } => {
                let (dx, dy) = (local.x - last.x, local.y - last.y);
                *last = local;
                self.view.pan(dx, dy);
            }
            Gesture::Marquee { now, .. } => *now = world,
            Gesture::Move {
                start,
                start_bounds,
                applied,
                ..
            } => {
                let raw = (world.x - start.x, world.y - start.y);
                let moved = Rect {
                    x: start_bounds.x + raw.0,
                    y: start_bounds.y + raw.1,
                    ..*start_bounds
                };
                let s = if snapping {
                    canvas::snap(moved, &others, grid, threshold)
                } else {
                    canvas::Snap::default()
                };
                let total = (raw.0 + s.dx, raw.1 + s.dy);
                let step = (total.0 - applied.0, total.1 - applied.1);
                *applied = total;
                self.guides = s.guides;
                self.editor.move_selection(step.0, step.1);
            }
            Gesture::Resize {
                id,
                start,
                original,
                corner,
            } => {
                let (dx, dy) = (world.x - start.x, world.y - start.y);
                let o = *original;
                let (mut l, mut t, mut r, mut b) = (o.x, o.y, o.right(), o.bottom());
                match corner {
                    0 => (l, t) = (l + dx, t + dy),
                    1 => (r, t) = (r + dx, t + dy),
                    2 => (r, b) = (r + dx, b + dy),
                    _ => (l, b) = (l + dx, b + dy),
                }
                // Тянущиеся края мягко прилипают к соседям и линиям сетки,
                // вдали от них — идут точно за мышью. Alt — без прилипания.
                let left_moves = matches!(corner, 0 | 3);
                let top_moves = matches!(corner, 0 | 1);
                let edge_x = if left_moves { l } else { r };
                let edge_y = if top_moves { t } else { b };
                let id = id.clone();
                self.guides.clear();
                if snapping {
                    let s =
                        canvas::snap_edges(Some(edge_x), Some(edge_y), &others, grid, threshold);
                    if left_moves {
                        l += s.dx
                    } else {
                        r += s.dx
                    }
                    if top_moves {
                        t += s.dy
                    } else {
                        b += s.dy
                    }
                    self.guides = s.guides;
                }
                // Уголок перетащили за противоположный — держим минимум,
                // прижимая к неподвижной стороне.
                let (min_w, min_h) = canvas::MIN_SIZE;
                if r - l < min_w {
                    if left_moves {
                        l = r - min_w
                    } else {
                        r = l + min_w
                    }
                }
                if b - t < min_h {
                    if top_moves {
                        t = b - min_h
                    } else {
                        b = t + min_h
                    }
                }
                self.editor.set_rect(
                    &id,
                    Rect {
                        x: l,
                        y: t,
                        w: r - l,
                        h: b - t,
                    },
                );
            }
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        self.guides.clear();
        match gesture {
            Gesture::Move { pressed, add, .. } => {
                if self.editor.drop_empty_checkpoint() {
                    // Щёлкнули, не сдвинув, по одной из выделенных карточек —
                    // выделенной остаётся только она, как в проводнике.
                    if !add {
                        self.editor.select_only(&pressed);
                    }
                } else {
                    self.changed(cx);
                }
            }
            Gesture::Resize { .. } => {
                if !self.editor.drop_empty_checkpoint() {
                    self.changed(cx);
                }
            }
            Gesture::Marquee { start, now, add } => {
                if start != now {
                    self.editor.select_in(Rect::from_corners(start, now), add);
                }
            }
            Gesture::Pan { .. } | Gesture::None => {}
        }
        cx.notify();
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.local(ev.position);
        match ev.delta {
            // Колесо мыши — масштаб (раздел 5: «масштаб колёсиком»).
            ScrollDelta::Lines(d) if ev.source == ScrollSource::Pointer => {
                let factor = WHEEL_ZOOM.powf(f64::from(d.y));
                self.view.zoom_at(at, factor);
            }
            // Тачпад с Ctrl — так Windows присылает щипок — тоже масштаб.
            ScrollDelta::Pixels(d) if ev.modifiers.control => {
                let factor = (f(d.y) / 200.0).exp();
                self.view.zoom_at(at, factor);
            }
            // Тачпад двумя пальцами и палец на экране — сдвиг поля.
            delta => {
                let d = delta.pixel_delta(px(20.));
                self.view.pan(f(d.x), f(d.y));
            }
        }
        cx.notify();
    }

    fn on_pinch(&mut self, ev: &PinchEvent, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.local(ev.position);
        self.view.zoom_at(at, 1.0 + f64::from(ev.delta));
        cx.notify();
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "space" && self.editing.is_none() {
            self.space_held = true;
            cx.stop_propagation();
        }
    }

    fn on_key_up(&mut self, ev: &KeyUpEvent, _: &mut Window, _: &mut Context<Self>) {
        if ev.keystroke.key == "space" {
            self.space_held = false;
        }
    }

    // ---- Текст карточки --------------------------------------------------

    fn start_editing(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(node) = self.editor.node(id) else {
            return;
        };
        let text = node.text.clone();
        let state = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).auto_grow(1, 400);
            s.set_value(text, window, cx);
            s
        });
        self.editor.checkpoint();
        let card = id.to_string();
        let sub = cx.subscribe_in(&state, window, move |this, state, ev, _, cx| {
            if matches!(ev, InputEvent::Change) {
                let value = state.read(cx).value().to_string();
                this.editor.set_text(&card, &value);
                this.changed(cx);
            }
        });
        self.focus_editing = true;
        self.editing = Some((id.to_string(), state, sub));
        cx.notify();
    }

    /// Заканчивает правку текста. Фокус возвращается холсту: иначе он
    /// остался бы на исчезнувшем поле, и клавиши холста перестали бы работать.
    fn finish_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.take().is_some() {
            self.focus_handle.focus(window, cx);
            // Ничего не набрали — точка отмены не нужна.
            self.editor.drop_empty_checkpoint();
            cx.notify();
        }
    }

    // ---- Действия с клавиатуры -------------------------------------------

    fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Editor) -> bool) {
        if self.read_only || self.editing.is_some() {
            return;
        }
        if f(&mut self.editor) {
            self.changed(cx);
        } else {
            cx.notify();
        }
    }

    fn nudge(&mut self, dx: f64, dy: f64, shift: bool, cx: &mut Context<Self>) {
        let step = if shift { canvas::GRID } else { 1.0 };
        self.edit(cx, |e| {
            if e.selection().is_empty() {
                return false;
            }
            e.checkpoint();
            e.move_selection(dx * step, dy * step);
            true
        });
    }

    pub fn add_card_at_center(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let (w, h) = self.size();
        let at = self.view.to_world(Point {
            x: w / 2.0,
            y: h / 2.0,
        });
        let id = self.editor.add_card(at, self.grid_step());
        self.changed(cx);
        self.start_editing(&id, window, cx);
    }

    fn zoom_center(&mut self, factor: f64, cx: &mut Context<Self>) {
        let (w, h) = self.size();
        self.view.zoom_at(
            Point {
                x: w / 2.0,
                y: h / 2.0,
            },
            factor,
        );
        cx.notify();
    }

    pub fn zoom_fit(&mut self, cx: &mut Context<Self>) {
        let (w, h) = self.size();
        let all = self
            .editor
            .nodes()
            .iter()
            .map(|n| self.editor.rect(n))
            .reduce(|a, b| a.union(&b));
        match all {
            Some(r) if w > 0.0 => self.view.fit(r, w, h),
            _ => self.view = View::default(),
        }
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) -> bool {
        match self.editor.copy_selection() {
            Some(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                true
            }
            None => false,
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            return;
        };
        self.edit(cx, |e| e.paste(&text, canvas::PASTE_OFFSET));
    }

    // ---- Отрисовка ------------------------------------------------------

    fn render_toolbar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let zoom = SharedString::from(format!("{:.0}%", self.view.zoom * 100.0));
        h_flex()
            .id("canvas-toolbar")
            .absolute()
            .bottom(px(16.))
            .left(px(16.))
            .gap_1()
            .p_1()
            .rounded(px(14.))
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().input)
            .when(!self.read_only, |d| {
                d.child(
                    Button::new("canvas-add-card")
                        .ghost()
                        .small()
                        .icon(Icon::new(IconName::SquarePlus))
                        .tooltip(tr(cx, "canvas.add_card"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.add_card_at_center(window, cx)),
                        ),
                )
            })
            .child(
                Button::new("canvas-grid")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Grid3x3))
                    .selected(self.grid)
                    .tooltip(tr(cx, "canvas.grid"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.grid = !this.grid;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("canvas-zoom-out")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Minus))
                    .tooltip(tr(cx, "canvas.zoom_out"))
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_center(1.0 / 1.25, cx))),
            )
            .child(
                Button::new("canvas-zoom-reset")
                    .ghost()
                    .small()
                    .label(zoom)
                    .tooltip(tr(cx, "canvas.zoom_reset"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let z = this.view.zoom;
                        this.zoom_center(1.0 / z, cx)
                    })),
            )
            .child(
                Button::new("canvas-zoom-in")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Plus))
                    .tooltip(tr(cx, "canvas.zoom_in"))
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_center(1.25, cx))),
            )
            .child(
                Button::new("canvas-zoom-fit")
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::Maximize))
                    .tooltip(tr(cx, "canvas.zoom_fit"))
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_fit(cx))),
            )
            .into_any_element()
    }
}

impl Render for CanvasView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (w, h) = self.size();
        // В первый показ — начало координат по центру поля.
        if !self.view_initialized && w > 0.0 {
            self.view_initialized = true;
            if self.editor.nodes().is_empty() {
                self.view.offset_x = w / 2.0;
                self.view.offset_y = h / 2.0;
            } else {
                self.zoom_fit(cx);
            }
        }
        // Высоты, которые карточки заняли в прошлом кадре, — ядру: по ним
        // попадание мышью, уголки и прилипание.
        for (id, h) in self.measured.borrow_mut().drain(..) {
            self.editor.set_shown_height(&id, h);
        }
        if self.focus_editing {
            self.focus_editing = false;
            if let Some((_, state, _)) = &self.editing {
                state.focus_handle(cx).focus(window, cx);
            }
        }
        let view = self.view;
        let zoom = view.zoom;
        let visible = view.visible(w.max(1.0), h.max(1.0));

        // Сетка: точки в узлах. При мелком масштабе и на большом экране —
        // через узел, чтобы точек было не больше GRID_MAX_DOTS.
        let bounds_cell = self.bounds.clone();
        let grid_color = theme.border;
        let show_grid = self.grid;
        let grid = paint_canvas(
            move |bounds, _, _| bounds_cell.set(bounds),
            move |bounds, _, window, _| {
                if !show_grid {
                    return;
                }
                let (bw, bh) = (f(bounds.size.width), f(bounds.size.height));
                let mut step = canvas::GRID * zoom;
                while step < 16.0 || (bw / step) * (bh / step) > GRID_MAX_DOTS {
                    step *= 2.0;
                }
                let ox = f(bounds.origin.x);
                let oy = f(bounds.origin.y);
                let start_x = view.offset_x.rem_euclid(step);
                let start_y = view.offset_y.rem_euclid(step);
                let dot = (1.5 * zoom.clamp(0.75, 1.5)) as f32;
                // Одним слоем: иначе каждая точка отдельно ищет своё место
                // в порядке рисования, а точек сотни.
                window.paint_layer(bounds, |window| {
                    let mut y = start_y;
                    while y < bh {
                        let mut x = start_x;
                        while x < bw {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(px((ox + x) as f32), px((oy + y) as f32)),
                                    size(px(dot), px(dot)),
                                ),
                                grid_color,
                            ));
                            x += step;
                        }
                        y += step;
                    }
                });
            },
        )
        .absolute()
        .size_full();

        let editing_id = self.editing.as_ref().map(|(id, ..)| id.clone());
        let overview = zoom < OVERVIEW_ZOOM && editing_id.is_none();
        let text_px = (16.0 * zoom) as f32;
        let radius = px((16.0 * zoom) as f32);

        // Обзор: все видимые карточки — прямоугольники одним проходом, текст
        // — полосками. Ни одного элемента раскладки на карточку.
        let overview_layer = overview.then(|| {
            let with_text = zoom >= OVERVIEW_TEXT_ZOOM;
            let shapes: Vec<(Rect, bool, usize)> = self
                .editor
                .nodes()
                .iter()
                .map(|n| (n, self.editor.rect(n)))
                .filter(|(_, r)| r.intersects(&visible))
                .map(|(n, r)| {
                    let lines = if with_text {
                        n.text.chars().count().div_ceil(24).min(3)
                    } else {
                        0
                    };
                    (r, self.editor.is_selected(&n.id), lines)
                })
                .collect();
            let (card_bg, card_border, bar, ring) =
                (theme.popover, theme.input, theme.muted, theme.primary);
            paint_canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let (ox, oy) = (f(bounds.origin.x), f(bounds.origin.y));
                    // Все карточки обзора — одним слоем: порядок рисования
                    // ищется один раз, а не для каждой из тысяч фигур.
                    window.paint_layer(bounds, |window| {
                        for (r, selected, lines) in &shapes {
                            let a = view.to_screen(Point { x: r.x, y: r.y });
                            let rect = Bounds::new(
                                point(px((ox + a.x) as f32), px((oy + a.y) as f32)),
                                size(px((r.w * zoom) as f32), px((r.h * zoom) as f32)),
                            );
                            window.paint_quad(
                                fill(rect, card_bg)
                                    .corner_radii(radius)
                                    .border_widths(px(if *selected { 2. } else { 1. }))
                                    .border_color(if *selected { ring } else { card_border }),
                            );
                            let pad = 12.0 * zoom;
                            for k in 0..*lines {
                                let width =
                                    (r.w - 24.0) * zoom * if k + 1 == *lines { 0.6 } else { 0.9 };
                                let line = Bounds::new(
                                    point(
                                        px((ox + a.x + pad) as f32),
                                        px((oy + a.y + pad + k as f64 * 22.0 * zoom) as f32),
                                    ),
                                    size(px(width.max(0.0) as f32), px((10.0 * zoom) as f32)),
                                );
                                window.paint_quad(
                                    fill(line, bar).corner_radii(px((3.0 * zoom) as f32)),
                                );
                            }
                        }
                    });
                },
            )
            .absolute()
            .size_full()
        });

        let cards = if overview {
            Vec::new()
        } else {
            self.editor
                .nodes()
                .iter()
                .filter(|n| self.editor.rect(n).intersects(&visible))
                .map(|n| {
                    let s = view.to_screen(Point { x: n.x, y: n.y });
                    let selected = self.editor.is_selected(&n.id);
                    let editing = editing_id.as_deref() == Some(n.id.as_str());
                    let mut card = div()
                        .id(SharedString::from(format!("node:{}", n.id)))
                        .test_support()
                        .absolute()
                        .left(px(s.x as f32))
                        .top(px(s.y as f32))
                        .w(px((n.width * zoom) as f32))
                        // Высота — не меньше заданной; длинный текст растит
                        // карточку на экране, а в файле остаётся заданная.
                        .min_h(px((n.height * zoom) as f32))
                        .rounded(radius)
                        .bg(theme.popover)
                        // Рамка одной толщины всегда: выделение рисуется
                        // отдельным контуром снаружи, и текст не сдвигается.
                        .border_1()
                        .border_color(theme.input)
                        .p(px((12.0 * zoom) as f32))
                        .text_size(px(text_px))
                        .line_height(px(text_px * 1.375))
                        .text_color(theme.foreground);
                    if editing && let Some((_, state, _)) = &self.editing {
                        // Тот же шрифт, что и без правки, — текст не прыгает.
                        card = card.child(
                            Textarea::new(state)
                                .bordered(false)
                                .appearance(false)
                                // Без своего фона поле не рисуется вовсе.
                                .bg(theme.popover)
                                .text_color(theme.foreground)
                                // Самый маленький размер поля — с минимальными
                                // внутренними отступами (4 слева и справа), их
                                // и гасим, чтобы текст стоял там же, где без
                                // правки.
                                .xsmall()
                                .mx(px(-4.))
                                .p_0()
                                .text_size(px(text_px))
                                .line_height(px(text_px * 1.375)),
                        );
                    } else if !n.text.is_empty() {
                        card = card.child(n.text.clone());
                    }
                    // Мерить нужно только карточки с текстом: без текста
                    // высота на экране и в файле совпадают.
                    if editing || !n.text.is_empty() {
                        let measured = self.measured.clone();
                        let id = n.id.clone();
                        let known = self.editor.rect(n).h;
                        card = card.child(
                            paint_canvas(
                                move |b, window, _| {
                                    let h = (f(b.size.height) + 2.0 * CARD_BORDER_PX) / zoom;
                                    // Высота поменялась — нужен ещё кадр, чтобы
                                    // уголки и выделение встали по ней. Просто
                                    // `refresh` во время отрисовки не действует.
                                    if (h - known).abs() > 0.5 {
                                        window.request_animation_frame();
                                    }
                                    measured.borrow_mut().push((id, h));
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .size_full(),
                        );
                    }
                    if selected {
                        card = card.child(
                            div()
                                .absolute()
                                .top(px(-3.))
                                .left(px(-3.))
                                .right(px(-3.))
                                .bottom(px(-3.))
                                .rounded(radius + px(2.))
                                .border_2()
                                .border_color(theme.primary),
                        );
                    }
                    card
                })
                .collect::<Vec<_>>()
        };

        // Уголки для изменения размера у одного выделенного объекта.
        let handles = (self.editor.selection().len() == 1 && editing_id.is_none())
            .then(|| self.editor.rect_of(&self.editor.selection()[0]))
            .flatten()
            .map(|r| {
                let a = view.to_screen(Point { x: r.x, y: r.y });
                let b = view.to_screen(Point {
                    x: r.right(),
                    y: r.bottom(),
                });
                [(a.x, a.y), (b.x, a.y), (b.x, b.y), (a.x, b.y)]
                    .into_iter()
                    .enumerate()
                    .map(|(i, (x, y))| {
                        div()
                            .id(("handle", i))
                            // Курсор подсказывает, что за уголок можно тянуть.
                            .when(i % 2 == 0, |d| d.cursor_nwse_resize())
                            .when(i % 2 == 1, |d| d.cursor_nesw_resize())
                            .absolute()
                            .left(px((x - 5.0) as f32))
                            .top(px((y - 5.0) as f32))
                            .size(px(10.))
                            .rounded(px(3.))
                            .bg(theme.background)
                            .border_2()
                            .border_color(theme.primary)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let marquee = match self.gesture {
            Gesture::Marquee { start, now, .. } if start != now => {
                let r = Rect::from_corners(view.to_screen(start), view.to_screen(now));
                Some(
                    div()
                        .absolute()
                        .left(px(r.x as f32))
                        .top(px(r.y as f32))
                        .w(px(r.w as f32))
                        .h(px(r.h as f32))
                        .border_1()
                        .border_color(theme.primary)
                        .bg(theme.primary.opacity(0.08)),
                )
            }
            _ => None,
        };

        let guides = self
            .guides
            .iter()
            .map(|g| match *g {
                Guide::Vertical(x) => {
                    let s = view.to_screen(Point { x, y: 0.0 });
                    div()
                        .absolute()
                        .left(px(s.x as f32))
                        .top_0()
                        .h_full()
                        .w(px(1.))
                        .bg(theme.danger)
                }
                Guide::Horizontal(y) => {
                    let s = view.to_screen(Point { x: 0.0, y });
                    div()
                        .absolute()
                        .top(px(s.y as f32))
                        .left_0()
                        .w_full()
                        .h(px(1.))
                        .bg(theme.danger)
                }
            })
            .collect::<Vec<_>>();

        let hint = (self.editor.nodes().is_empty() && !self.read_only).then(|| {
            div()
                .absolute()
                .top(px(24.))
                .w_full()
                .flex()
                .justify_center()
                .child(
                    div()
                        .px_4()
                        .py_2()
                        .rounded(px(12.))
                        .bg(theme.muted)
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(tr(cx, "canvas.hint")),
                )
        });

        let toolbar = self.render_toolbar(cx);
        let has_selection = !self.editor.selection().is_empty();
        div()
            .id("canvas")
            .test_support()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.background)
            .when(self.space_held, |d| d.cursor_grab())
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(Self::on_pinch))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.edit(cx, Editor::undo)))
            .on_action(cx.listener(|this, _: &Redo, _, cx| this.edit(cx, Editor::redo)))
            .on_action(
                cx.listener(|this, _: &Delete, _, cx| this.edit(cx, Editor::delete_selection)),
            )
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                this.copy(cx);
            }))
            .on_action(cx.listener(|this, _: &Cut, _, cx| {
                if this.copy(cx) {
                    this.edit(cx, Editor::delete_selection);
                }
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| this.paste(cx)))
            .on_action(cx.listener(|this, _: &Duplicate, _, cx| this.edit(cx, Editor::duplicate)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                this.editor.select_all();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Deselect, window, cx| {
                if this.editing.is_some() {
                    this.finish_editing(window, cx);
                } else {
                    this.editor.clear_selection();
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &BringToFront, _, cx| {
                this.edit(cx, |e| layer(e, Editor::bring_to_front))
            }))
            .on_action(cx.listener(|this, _: &SendToBack, _, cx| {
                this.edit(cx, |e| layer(e, Editor::send_to_back))
            }))
            .on_action(cx.listener(|this, _: &BringForward, _, cx| {
                this.edit(cx, |e| layer(e, Editor::bring_forward))
            }))
            .on_action(cx.listener(|this, _: &SendBackward, _, cx| {
                this.edit(cx, |e| layer(e, Editor::send_backward))
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_center(1.25, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_center(0.8, cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| {
                let z = this.view.zoom;
                this.zoom_center(1.0 / z, cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomFit, _, cx| this.zoom_fit(cx)))
            .on_action(cx.listener(|this, _: &EditText, window, cx| {
                if let [id] = this.editor.selection() {
                    let id = id.clone();
                    this.start_editing(&id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| this.nudge(-1.0, 0.0, false, cx)))
            .on_action(cx.listener(|this, _: &MoveLeftGrid, _, cx| this.nudge(-1.0, 0.0, true, cx)))
            .on_action(cx.listener(|this, _: &MoveRight, _, cx| this.nudge(1.0, 0.0, false, cx)))
            .on_action(cx.listener(|this, _: &MoveRightGrid, _, cx| this.nudge(1.0, 0.0, true, cx)))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.nudge(0.0, -1.0, false, cx)))
            .on_action(cx.listener(|this, _: &MoveUpGrid, _, cx| this.nudge(0.0, -1.0, true, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.nudge(0.0, 1.0, false, cx)))
            .on_action(cx.listener(|this, _: &MoveDownGrid, _, cx| this.nudge(0.0, 1.0, true, cx)))
            .context_menu(move |menu, window, cx| {
                let _ = window;
                if !has_selection {
                    return menu;
                }
                menu.item(
                    PopupMenuItem::new(tr(cx, "canvas.bring_to_front"))
                        .action(Box::new(BringToFront)),
                )
                .item(
                    PopupMenuItem::new(tr(cx, "canvas.bring_forward"))
                        .action(Box::new(BringForward)),
                )
                .item(
                    PopupMenuItem::new(tr(cx, "canvas.send_backward"))
                        .action(Box::new(SendBackward)),
                )
                .item(
                    PopupMenuItem::new(tr(cx, "canvas.send_to_back")).action(Box::new(SendToBack)),
                )
                .separator()
                .item(PopupMenuItem::new(tr(cx, "canvas.duplicate")).action(Box::new(Duplicate)))
                .item(PopupMenuItem::new(tr(cx, "common.delete")).action(Box::new(Delete)))
            })
            .child(grid)
            .children(overview_layer)
            .children(cards)
            .children(handles)
            .children(guides)
            .children(marquee)
            .children(hint)
            .child(toolbar)
    }
}

/// Порядок слоёв: правка есть, только если порядок действительно поменялся.
fn layer(e: &mut Editor, f: fn(&mut Editor)) -> bool {
    let before: Vec<String> = e.nodes().iter().map(|n| n.id.clone()).collect();
    f(e);
    e.nodes().iter().map(|n| &n.id).ne(before.iter())
}

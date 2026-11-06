//! Оформленный текст карточки на экране.
//!
//! Один и тот же элемент и показывает текст, и правит его: войдя в правку
//! двойным щелчком, человек видит тот же текст на том же месте. Вёрстка
//! хранится в [`TextLayout`]: по ней мышь находит место в тексте, а стрелки
//! вверх и вниз — строку выше и ниже.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bolh_core::rich::{Block, Kind, Pos, RichText};
use gpui_kit::{
    App, AvailableSpace, Bounds, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, PathBuilder, Pixels, Point, ShapedLine, SharedString, Size,
    StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle, Window, WrappedLine, fill,
    point, px, relative, size, transparent_black,
};

use crate::card_editor::CardEditor;

/// Цвета, которых нет в обычном стиле текста. Шрифт, размер и цвет текста
/// элемент берёт у родителя — у карточки.
#[derive(Clone, Copy)]
pub struct RichColors {
    /// Маркеры, номера, пустой квадрат и отмеченные пункты.
    pub muted: Hsla,
    /// Отмеченный квадрат и курсор.
    pub accent: Hsla,
    /// Галочка поверх отмеченного квадрата.
    pub on_accent: Hsla,
    pub selection: Hsla,
}

/// Вёрстка показанных карточек в последнем кадре и где они стояли. По ней
/// холст находит квадрат чек-листа под щелчком и место в тексте, с которого
/// начать правку. Собирается заново в каждом кадре.
pub type CardLayouts = Rc<RefCell<HashMap<String, (Rc<TextLayout>, Bounds<Pixels>)>>>;

/// Что стоит слева от текста блока.
enum Marker {
    None,
    /// Маркер списка; вид зависит от уровня вложенности.
    Bullet(u8),
    /// Номер пункта. В куче: свёрстанная строка весит около трёх
    /// килобайт, и каждый блок иначе занимал бы столько же.
    Number(Box<ShapedLine>),
    Check(bool),
}

/// Свёрстанный блок. Все места — относительно левого верхнего угла
/// элемента.
pub struct BlockLayout {
    /// Начало текста.
    pub origin: Point<Pixels>,
    pub line: WrappedLine,
    pub line_height: Pixels,
    pub height: Pixels,
    /// Ширина, в которую переносится текст.
    pub text_width: Pixels,
    font_size: Pixels,
    marker: Marker,
    /// Квадрат чек-листа.
    pub check_box: Option<Bounds<Pixels>>,
}

pub struct TextLayout {
    pub blocks: Vec<BlockLayout>,
    pub size: Size<Pixels>,
}

/// Шрифт и строка, от которых считается всё остальное.
#[derive(Clone)]
struct Base {
    font: gpui_kit::Font,
    font_size: Pixels,
    line_height: Pixels,
    color: Hsla,
}

fn base(window: &Window) -> Base {
    let s = window.text_style();
    let rem = window.rem_size();
    Base {
        font: s.font(),
        font_size: s.font_size.to_pixels(rem),
        line_height: s.line_height_in_pixels(rem),
        color: s.color,
    }
}

/// Толщина линий подчёркивания и галочки — растёт с масштабом, но не
/// тоньше точки.
fn stroke(font_size: Pixels) -> Pixels {
    px((f32::from(font_size) / 14.0).max(1.0))
}

fn runs(
    b: &Block,
    base: &Base,
    colors: &RichColors,
    heading: bool,
    marked: Option<(usize, usize)>,
) -> Vec<TextRun> {
    let done = matches!(b.kind, Kind::Check(true));
    let color = if done { colors.muted } else { base.color };
    let thick = stroke(base.font_size);
    let mut out = Vec::with_capacity(b.spans.len() + 2);
    let mut pos = 0;
    for s in &b.spans {
        let mut font = base.font.clone();
        if s.marks.bold || heading {
            font.weight = FontWeight::BOLD;
        }
        if s.marks.italic {
            font.style = FontStyle::Italic;
        }
        let run = TextRun {
            len: s.text.len(),
            font,
            color,
            background_color: None,
            underline: s.marks.underline.then_some(UnderlineStyle {
                thickness: thick,
                color: Some(color),
                wavy: false,
            }),
            // Отмеченный пункт зачёркнут: сделано.
            strikethrough: (s.marks.strike || done).then_some(StrikethroughStyle {
                thickness: thick,
                color: Some(color),
            }),
        };
        // Набираемое через систему ввода (иероглифы, значки) подчёркнуто,
        // пока не подтверждено.
        match marked {
            Some((a, z)) if a < pos + s.text.len() && z > pos => {
                let (a, z) = (a.max(pos), z.min(pos + s.text.len()));
                let mut push = |len: usize, underline: bool| {
                    if len > 0 {
                        let mut r = run.clone();
                        r.len = len;
                        if underline {
                            r.underline = Some(UnderlineStyle {
                                thickness: thick,
                                color: Some(color),
                                wavy: false,
                            });
                        }
                        out.push(r);
                    }
                };
                push(a - pos, false);
                push(z - a, true);
                push(pos + s.text.len() - z, false);
            }
            _ => out.push(run),
        }
        pos += s.text.len();
    }
    out
}

/// Вёрстка текста в заданную ширину. `marked` — промежуток, который сейчас
/// набирается через систему ввода.
fn layout(
    rich: &RichText,
    base: &Base,
    colors: &RichColors,
    width: Pixels,
    marked: Option<(Pos, Pos)>,
    window: &mut Window,
) -> TextLayout {
    let numbers = rich.numbers();
    let mut y = px(0.);
    let mut blocks = Vec::with_capacity(rich.blocks.len());
    for (i, b) in rich.blocks.iter().enumerate() {
        let (scale, heading) = match b.kind {
            Kind::Heading(1) => (1.5, true),
            Kind::Heading(2) => (1.25, true),
            Kind::Heading(_) => (1.1, true),
            _ => (1.0, false),
        };
        let font_size = base.font_size * scale;
        let line_height = base.line_height * scale;
        let em = font_size;
        let indent = em * (1.5 * f32::from(b.indent));
        let (marker, gutter) = match b.kind {
            Kind::Bullet => (Marker::Bullet(b.indent), em * 1.25),
            Kind::Check(done) => (Marker::Check(done), em * 1.5),
            Kind::Numbered => {
                let label: SharedString = format!("{}.", numbers[i].unwrap_or(1)).into();
                let run = TextRun {
                    len: label.len(),
                    font: base.font.clone(),
                    color: colors.muted,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped = window
                    .text_system()
                    .shape_line(label, font_size, &[run], None);
                let w = (shaped.width() + em * 0.4).max(em * 1.25);
                (Marker::Number(Box::new(shaped)), w)
            }
            _ => (Marker::None, px(0.)),
        };
        let x = indent + gutter;
        let text_width = (width - x).max(px(1.));
        let block_base = Base {
            font_size,
            ..base.clone()
        };
        let marked_here = marked.and_then(|(a, z)| {
            (a.block <= i && z.block >= i).then(|| {
                let from = if a.block == i { a.offset } else { 0 };
                let to = if z.block == i { z.offset } else { b.len() };
                (from, to)
            })
        });
        let runs = runs(b, &block_base, colors, heading, marked_here);
        let line = window
            .text_system()
            .shape_text(b.text().into(), font_size, &runs, Some(text_width), None)
            .ok()
            .and_then(|mut lines| lines.pop())
            .unwrap_or_default();
        let height = line.size(line_height).height.max(line_height);
        let check_box = matches!(b.kind, Kind::Check(_)).then(|| {
            let s = em;
            Bounds::new(point(indent, y + (line_height - s) / 2.), size(s, s))
        });
        blocks.push(BlockLayout {
            origin: point(x, y),
            line,
            line_height,
            height,
            text_width,
            font_size,
            marker,
            check_box,
        });
        y += height;
    }
    TextLayout {
        blocks,
        size: size(width, y),
    }
}

impl TextLayout {
    fn block_at_y(&self, y: Pixels) -> usize {
        self.blocks
            .iter()
            .position(|b| y < b.origin.y + b.height)
            .unwrap_or(self.blocks.len().saturating_sub(1))
    }

    /// Место в тексте под точкой (относительно элемента). Точка выше текста
    /// — первая строка, ниже — последняя.
    pub fn pos_at(&self, p: Point<Pixels>) -> Pos {
        if self.blocks.is_empty() {
            return Pos::default();
        }
        let i = self.block_at_y(p.y);
        let b = &self.blocks[i];
        // Ниже последней строки блока GPUI отвечает началом строки, поэтому
        // точку держим внутри блока.
        let local_y = (p.y - b.origin.y).clamp(px(0.), b.height - px(0.5));
        let local = point(p.x - b.origin.x, local_y);
        let offset = match b.line.closest_index_for_position(local, b.line_height) {
            Ok(i) | Err(i) => i,
        };
        Pos::new(i, offset.min(b.line.len()))
    }

    /// Левый верхний угол курсора в этом месте и высота строки.
    pub fn caret_at(&self, pos: Pos) -> (Point<Pixels>, Pixels) {
        let Some(b) = self.blocks.get(pos.block) else {
            return (point(px(0.), px(0.)), px(16.));
        };
        let p = b
            .line
            .position_for_index(pos.offset.min(b.line.len()), b.line_height)
            .unwrap_or_default();
        (b.origin + p, b.line_height)
    }

    /// Строка выше или ниже — для стрелок. `None` — выше первой или ниже
    /// последней строки.
    pub fn vertical(&self, pos: Pos, goal_x: Pixels, down: bool) -> Option<Pos> {
        let (caret, lh) = self.caret_at(pos);
        let y = if down {
            caret.y + lh * 1.5
        } else {
            caret.y - lh * 0.5
        };
        (y >= px(0.) && y < self.size.height).then(|| self.pos_at(point(goal_x, y)))
    }

    /// Чей квадрат чек-листа под точкой.
    pub fn check_at(&self, p: Point<Pixels>) -> Option<usize> {
        self.blocks.iter().position(|b| {
            b.check_box.is_some_and(|r| {
                // Попасть по маленькому квадрату легче с запасом вокруг.
                let pad = b.font_size * 0.25;
                p.x >= r.left() - pad
                    && p.x <= r.right() + pad
                    && p.y >= r.top() - pad
                    && p.y <= r.bottom() + pad
            })
        })
    }

    /// Прямоугольники выделения — по одному на строку.
    pub fn selection_rects(&self, start: Pos, end: Pos) -> Vec<Bounds<Pixels>> {
        let mut out = Vec::new();
        for i in start.block..=end.block.min(self.blocks.len().saturating_sub(1)) {
            let b = &self.blocks[i];
            let len = b.line.len();
            let a = if i == start.block { start.offset } else { 0 }.min(len);
            let z = if i == end.block { end.offset } else { len }.min(len);
            let lh = b.line_height;
            let (Some(pa), Some(pz)) = (
                b.line.position_for_index(a, lh),
                b.line.position_for_index(z, lh),
            ) else {
                continue;
            };
            // Перевод строки тоже выделен — видно, что блоки склеятся.
            let tail = if i < end.block {
                b.font_size * 0.4
            } else {
                px(0.)
            };
            let o = b.origin;
            let rect = |x0: Pixels, x1: Pixels, y: Pixels| {
                Bounds::new(point(o.x + x0, o.y + y), size((x1 - x0).max(px(0.)), lh))
            };
            if pa.y == pz.y {
                out.push(rect(pa.x, pz.x + tail, pa.y));
            } else {
                out.push(rect(pa.x, b.text_width, pa.y));
                let mut y = pa.y + lh;
                while y < pz.y {
                    out.push(rect(px(0.), b.text_width, y));
                    y += lh;
                }
                out.push(rect(px(0.), pz.x + tail, pz.y));
            }
        }
        out
    }

    fn paint(&self, origin: Point<Pixels>, colors: &RichColors, window: &mut Window, cx: &mut App) {
        for b in &self.blocks {
            let o = origin + b.origin;
            let em = b.font_size;
            let mid = o.y + b.line_height / 2.;
            match &b.marker {
                Marker::None => {}
                Marker::Bullet(level) => {
                    let d = em * 0.34;
                    let r = Bounds::new(point(o.x - em * 0.75 - d / 2., mid - d / 2.), size(d, d));
                    let quad = match level % 3 {
                        0 => fill(r, colors.muted).corner_radii(d / 2.),
                        1 => fill(r, transparent_black())
                            .corner_radii(d / 2.)
                            .border_widths(stroke(em))
                            .border_color(colors.muted),
                        _ => fill(r, colors.muted),
                    };
                    window.paint_quad(quad);
                }
                Marker::Number(label) => {
                    let x = o.x - em * 0.35 - label.width();
                    let _ = label.paint(
                        point(x, o.y),
                        b.line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                Marker::Check(done) => {
                    if let Some(r) = b.check_box {
                        let r = r + origin;
                        let radius = r.size.width * 0.22;
                        if *done {
                            window.paint_quad(fill(r, colors.accent).corner_radii(radius));
                            let s = r.size.width;
                            let mut path = PathBuilder::stroke(stroke(em) * 1.6);
                            path.move_to(point(r.left() + s * 0.24, r.top() + s * 0.52));
                            path.line_to(point(r.left() + s * 0.43, r.top() + s * 0.71));
                            path.line_to(point(r.left() + s * 0.77, r.top() + s * 0.31));
                            if let Ok(path) = path.build() {
                                window.paint_path(path, colors.on_accent);
                            }
                        } else {
                            window.paint_quad(
                                fill(r, transparent_black())
                                    .corner_radii(radius)
                                    .border_widths(stroke(em) * 1.5)
                                    .border_color(colors.muted),
                            );
                        }
                    }
                }
            }
            let _ = b
                .line
                .paint(o, b.line_height, TextAlign::Left, None, window, cx);
        }
    }
}

/// Показать или править.
enum Mode {
    /// Только показ. Вёрстка уходит холсту под этим id.
    Display {
        rich: RichText,
        sink: Option<(CardLayouts, String)>,
    },
    /// Правка: курсор, выделение, ввод с клавиатуры.
    Editing(Entity<CardEditor>),
}

pub struct RichTextElement {
    mode: Mode,
    colors: RichColors,
}

impl RichTextElement {
    pub fn display(
        rich: RichText,
        colors: RichColors,
        sink: Option<(CardLayouts, String)>,
    ) -> Self {
        RichTextElement {
            mode: Mode::Display { rich, sink },
            colors,
        }
    }

    pub fn editing(editor: Entity<CardEditor>, colors: RichColors) -> Self {
        RichTextElement {
            mode: Mode::Editing(editor),
            colors,
        }
    }
}

impl IntoElement for RichTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct LayoutState {
    rich: RichText,
    marked: Option<(Pos, Pos)>,
    base: Base,
    /// Вёрстка из замера — чтобы не верстать второй раз в той же ширине.
    measured: Rc<RefCell<Option<TextLayout>>>,
}

pub struct PaintState {
    layout: Rc<TextLayout>,
    checks: Vec<(Hitbox, usize)>,
    text: Option<Hitbox>,
}

impl Element for RichTextElement {
    type RequestLayoutState = LayoutState;
    type PrepaintState = PaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let (rich, marked) = match &self.mode {
            Mode::Display { rich, .. } => (rich.clone(), None),
            Mode::Editing(e) => {
                let e = e.read(cx);
                (e.text().clone(), e.marked())
            }
        };
        let base = base(window);
        let measured = Rc::new(RefCell::new(None));
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let id = window.request_measured_layout(style, {
            let (rich, base, colors, measured) =
                (rich.clone(), base.clone(), self.colors, measured.clone());
            move |known, available, window, _| {
                let width = known.width.unwrap_or(match available.width {
                    AvailableSpace::Definite(w) => w,
                    // Без заданной ширины — в одну строку, как есть.
                    _ => px(100_000.),
                });
                let l = layout(&rich, &base, &colors, width, marked, window);
                let size = l.size;
                *measured.borrow_mut() = Some(l);
                size
            }
        });
        (
            id,
            LayoutState {
                rich,
                marked,
                base,
                measured,
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        let measured = state
            .measured
            .borrow_mut()
            .take()
            .filter(|l| (l.size.width - bounds.size.width).abs() < px(0.5));
        let layout = measured.unwrap_or_else(|| {
            layout(
                &state.rich,
                &state.base,
                &self.colors,
                bounds.size.width,
                state.marked,
                window,
            )
        });
        let checks = layout
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                b.check_box.map(|r| {
                    (
                        window.insert_hitbox(r + bounds.origin, HitboxBehavior::Normal),
                        i,
                    )
                })
            })
            .collect();
        let text = matches!(self.mode, Mode::Editing(_))
            .then(|| window.insert_hitbox(bounds, HitboxBehavior::Normal));
        PaintState {
            layout: Rc::new(layout),
            checks,
            text,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        state: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = state.layout.clone();
        if let Some(hitbox) = &state.text {
            window.set_cursor_style(CursorStyle::IBeam, hitbox);
        }
        for (hitbox, _) in &state.checks {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        match &self.mode {
            Mode::Display { sink, .. } => {
                layout.paint(bounds.origin, &self.colors, window, cx);
                if let Some((sink, id)) = sink {
                    sink.borrow_mut().insert(id.clone(), (layout, bounds));
                }
            }
            Mode::Editing(entity) => {
                let (sel, focus) = {
                    let e = entity.read(cx);
                    (e.selection(), e.focus_handle.clone())
                };
                if !sel.is_empty() {
                    for r in layout.selection_rects(sel.start(), sel.end()) {
                        window.paint_quad(fill(r + bounds.origin, self.colors.selection));
                    }
                }
                layout.paint(bounds.origin, &self.colors, window, cx);
                if focus.is_focused(window) {
                    let (p, lh) = layout.caret_at(sel.head);
                    let w = stroke(
                        layout
                            .blocks
                            .first()
                            .map(|b| b.font_size)
                            .unwrap_or(px(16.)),
                    )
                    .max(px(1.5));
                    window.paint_quad(fill(
                        Bounds::new(bounds.origin + p, size(w, lh)),
                        self.colors.accent,
                    ));
                }
                window.handle_input(&focus, ElementInputHandler::new(bounds, entity.clone()), cx);
                entity.update(cx, |e, _| e.set_layout(layout, bounds));
            }
        }
    }
}

//! Правка холста без экрана: объекты, выделение, отмена и повтор,
//! прилипание к сетке и направляющим, порядок слоёв, копирование.
//!
//! Координаты здесь — «мировые», в пикселях холста при масштабе 100%.
//! Перевод в точки экрана — забота интерфейса.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::format::{Canvas, Node};

/// Шаг сетки. Кратен шагу 4 из дизайн-токенов.
pub const GRID: f64 = 20.0;
/// Размер новой карточки.
pub const CARD_SIZE: (f64, f64) = (240.0, 140.0);
/// Меньше этого карточку не сжать: в неё должна влезать хотя бы строка.
pub const MIN_SIZE: (f64, f64) = (60.0, 40.0);
/// Сколько шагов отмены помнить.
pub const UNDO_LIMIT: usize = 200;
/// Сдвиг вставленной копии, чтобы она не легла ровно поверх оригинала.
pub const PASTE_OFFSET: f64 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn of(n: &Node) -> Rect {
        Rect {
            x: n.x,
            y: n.y,
            w: n.width,
            h: n.height,
        }
    }

    /// Прямоугольник по двум углам в любом порядке — так тянут рамку
    /// выделения.
    pub fn from_corners(a: Point, b: Point) -> Rect {
        Rect {
            x: a.x.min(b.x),
            y: a.y.min(b.y),
            w: (a.x - b.x).abs(),
            h: (a.y - b.y).abs(),
        }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.x <= o.right() && o.x <= self.right() && self.y <= o.bottom() && o.y <= self.bottom()
    }

    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect {
            x,
            y,
            w: self.right().max(o.right()) - x,
            h: self.bottom().max(o.bottom()) - y,
        }
    }

    fn xs(&self) -> [f64; 3] {
        [self.x, self.x + self.w / 2.0, self.right()]
    }

    fn ys(&self) -> [f64; 3] {
        [self.y, self.y + self.h / 2.0, self.bottom()]
    }
}

/// Масштаб: от 10% до 400%.
pub const ZOOM_MIN: f64 = 0.1;
pub const ZOOM_MAX: f64 = 4.0;

/// Что видно на экране: где на экране начало координат холста и масштаб.
/// В файл не пишется — иначе каждая прокрутка была бы «правкой» и
/// гоняла бы файл через облако.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub offset_x: f64,
    pub offset_y: f64,
    pub zoom: f64,
}

impl Default for View {
    fn default() -> Self {
        View {
            offset_x: 0.0,
            offset_y: 0.0,
            zoom: 1.0,
        }
    }
}

impl View {
    pub fn to_screen(&self, p: Point) -> Point {
        Point {
            x: p.x * self.zoom + self.offset_x,
            y: p.y * self.zoom + self.offset_y,
        }
    }

    pub fn to_world(&self, s: Point) -> Point {
        Point {
            x: (s.x - self.offset_x) / self.zoom,
            y: (s.y - self.offset_y) / self.zoom,
        }
    }

    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.offset_x += dx;
        self.offset_y += dy;
    }

    /// Меняет масштаб в `factor` раз так, что точка холста под курсором
    /// остаётся под курсором.
    pub fn zoom_at(&mut self, screen: Point, factor: f64) {
        let before = self.to_world(screen);
        self.zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        self.offset_x = screen.x - before.x * self.zoom;
        self.offset_y = screen.y - before.y * self.zoom;
    }

    /// Показывает прямоугольник целиком по центру области `w`×`h`, с полями,
    /// но не крупнее 100%.
    pub fn fit(&mut self, r: Rect, w: f64, h: f64) {
        let margin = 48.0;
        let zx = (w - 2.0 * margin) / r.w.max(1.0);
        let zy = (h - 2.0 * margin) / r.h.max(1.0);
        self.zoom = zx.min(zy).clamp(ZOOM_MIN, 1.0);
        self.offset_x = w / 2.0 - (r.x + r.w / 2.0) * self.zoom;
        self.offset_y = h / 2.0 - (r.y + r.h / 2.0) * self.zoom;
    }

    /// Какая часть холста видна в области `w`×`h` — чтобы не рисовать
    /// невидимое.
    pub fn visible(&self, w: f64, h: f64) -> Rect {
        let a = self.to_world(Point { x: 0.0, y: 0.0 });
        Rect {
            x: a.x,
            y: a.y,
            w: w / self.zoom,
            h: h / self.zoom,
        }
    }
}

/// Направляющая, которую интерфейс рисует во время перетаскивания: другой
/// объект стоит ровно на этой линии.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Guide {
    /// Вертикальная линия на этом `x`.
    Vertical(f64),
    /// Горизонтальная линия на этом `y`.
    Horizontal(f64),
}

/// Поправка к сдвигу, чтобы объект прилип, и линии, к которым он прилип.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snap {
    pub dx: f64,
    pub dy: f64,
    pub guides: Vec<Guide>,
}

/// Куда прилипнуть перемещаемому прямоугольнику.
///
/// Сначала — к краям и серединам других объектов, если они ближе
/// `threshold`: выровнять по соседу важнее, чем по сетке. Иначе, если
/// сетка включена, — левый верхний угол к ближайшему узлу сетки. По осям
/// независимо: можно прилипнуть к соседу по горизонтали и к сетке по
/// вертикали.
pub fn snap(moving: Rect, others: &[Rect], grid: Option<f64>, threshold: f64) -> Snap {
    let mut out = Snap::default();
    let best = |mine: [f64; 3], theirs: &mut dyn Iterator<Item = [f64; 3]>| {
        let mut best: Option<(f64, f64)> = None; // (поправка, линия)
        for t in theirs {
            for m in mine {
                for line in t {
                    let d = line - m;
                    if d.abs() <= threshold && best.is_none_or(|(b, _)| d.abs() < b.abs()) {
                        best = Some((d, line));
                    }
                }
            }
        }
        best
    };
    match best(moving.xs(), &mut others.iter().map(Rect::xs)) {
        Some((d, line)) => {
            out.dx = d;
            out.guides.push(Guide::Vertical(line));
        }
        None => {
            if let Some(g) = grid {
                out.dx = (moving.x / g).round() * g - moving.x;
            }
        }
    }
    match best(moving.ys(), &mut others.iter().map(Rect::ys)) {
        Some((d, line)) => {
            out.dy = d;
            out.guides.push(Guide::Horizontal(line));
        }
        None => {
            if let Some(g) = grid {
                out.dy = (moving.y / g).round() * g - moving.y;
            }
        }
    }
    out
}

/// Формат буфера обмена: обычный текст JSON с меткой, чтобы чужой текст
/// не принимался за объекты холста.
#[derive(Serialize, Deserialize)]
struct Clip {
    #[serde(rename = "bolh-place-clipboard")]
    marker: u32,
    nodes: Vec<Node>,
}

/// Открытый холст: содержимое, выделение и история правок.
#[derive(Debug, Clone)]
pub struct Editor {
    canvas: Canvas,
    selection: Vec<String>,
    undo: Vec<Vec<Node>>,
    redo: Vec<Vec<Node>>,
}

impl Editor {
    pub fn new(canvas: Canvas) -> Self {
        Editor {
            canvas,
            selection: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    pub fn nodes(&self) -> &[Node] {
        &self.canvas.nodes
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.canvas.nodes.iter().find(|n| n.id == id)
    }

    pub fn selection(&self) -> &[String] {
        &self.selection
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.selection.iter().any(|s| s == id)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Запоминает состояние перед правкой. Перетаскивание вызывает его один
    /// раз в начале, и тогда вся протяжка отменяется одним Ctrl+Z.
    pub fn checkpoint(&mut self) {
        self.undo.push(self.canvas.nodes.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Убирает последнюю точку отмены, если после неё ничего не поменялось
    /// (щёлкнули по карточке, но не сдвинули).
    pub fn drop_empty_checkpoint(&mut self) {
        if self.undo.last() == Some(&self.canvas.nodes) {
            self.undo.pop();
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        let now = std::mem::replace(&mut self.canvas.nodes, prev);
        self.redo.push(now);
        self.keep_selection_valid();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        let now = std::mem::replace(&mut self.canvas.nodes, next);
        self.undo.push(now);
        self.keep_selection_valid();
        true
    }

    fn keep_selection_valid(&mut self) {
        let ids: HashSet<&str> = self.canvas.nodes.iter().map(|n| n.id.as_str()).collect();
        self.selection.retain(|s| ids.contains(s.as_str()));
    }

    // ---- Выделение -------------------------------------------------------

    /// Верхний объект под точкой.
    pub fn hit(&self, p: Point) -> Option<&str> {
        self.canvas
            .nodes
            .iter()
            .rev()
            .find(|n| Rect::of(n).contains(p))
            .map(|n| n.id.as_str())
    }

    pub fn select_only(&mut self, id: &str) {
        self.selection = vec![id.to_string()];
    }

    /// Ctrl/Shift+щелчок: добавить или убрать из выделения.
    pub fn toggle_selected(&mut self, id: &str) {
        if let Some(i) = self.selection.iter().position(|s| s == id) {
            self.selection.remove(i);
        } else {
            self.selection.push(id.to_string());
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn select_all(&mut self) {
        self.selection = self.canvas.nodes.iter().map(|n| n.id.clone()).collect();
    }

    /// Рамка выделения: всё, что задевает прямоугольник. С `add` —
    /// добавить к уже выделенному.
    pub fn select_in(&mut self, r: Rect, add: bool) {
        if !add {
            self.selection.clear();
        }
        for n in &self.canvas.nodes {
            if Rect::of(n).intersects(&r) && !self.selection.contains(&n.id) {
                self.selection.push(n.id.clone());
            }
        }
    }

    pub fn selection_bounds(&self) -> Option<Rect> {
        self.canvas
            .nodes
            .iter()
            .filter(|n| self.is_selected(&n.id))
            .map(Rect::of)
            .reduce(|a, b| a.union(&b))
    }

    /// Все объекты вне выделения — к ним прилипает перемещаемое.
    pub fn unselected_rects(&self) -> Vec<Rect> {
        self.canvas
            .nodes
            .iter()
            .filter(|n| !self.is_selected(&n.id))
            .map(Rect::of)
            .collect()
    }

    // ---- Правки ----------------------------------------------------------

    /// Новая карточка с центром в точке `at`, прилипшая к сетке.
    pub fn add_card(&mut self, at: Point, grid: Option<f64>) -> String {
        self.checkpoint();
        let (w, h) = CARD_SIZE;
        let mut x = at.x - w / 2.0;
        let mut y = at.y - h / 2.0;
        if let Some(g) = grid {
            x = (x / g).round() * g;
            y = (y / g).round() * g;
        }
        let id = Uuid::new_v4().to_string();
        self.canvas.nodes.push(Node::card(id.clone(), x, y, w, h));
        self.select_only(&id);
        id
    }

    /// Сдвигает выделенное. Точку отмены ставит вызывающий.
    pub fn move_selection(&mut self, dx: f64, dy: f64) {
        let sel: HashSet<String> = self.selection.iter().cloned().collect();
        for n in self.canvas.nodes.iter_mut().filter(|n| sel.contains(&n.id)) {
            n.x += dx;
            n.y += dy;
        }
    }

    /// Задаёт объекту размер и место, не меньше [`MIN_SIZE`]. Точку отмены
    /// ставит вызывающий.
    pub fn set_rect(&mut self, id: &str, r: Rect) {
        if let Some(n) = self.canvas.nodes.iter_mut().find(|n| n.id == id) {
            n.x = r.x;
            n.y = r.y;
            n.width = r.w.max(MIN_SIZE.0);
            n.height = r.h.max(MIN_SIZE.1);
        }
    }

    /// Меняет текст карточки. Точку отмены ставит вызывающий — один раз за
    /// всё время правки текста, а не на каждую букву.
    pub fn set_text(&mut self, id: &str, text: &str) {
        if let Some(n) = self.canvas.nodes.iter_mut().find(|n| n.id == id) {
            n.text = text.to_string();
        }
    }

    pub fn delete_selection(&mut self) -> bool {
        if self.selection.is_empty() {
            return false;
        }
        self.checkpoint();
        let sel: HashSet<String> = self.selection.drain(..).collect();
        self.canvas.nodes.retain(|n| !sel.contains(&n.id));
        true
    }

    // ---- Порядок слоёв ---------------------------------------------------

    fn reorder(&mut self, f: impl FnOnce(&mut Vec<Node>, &HashSet<String>)) {
        if self.selection.is_empty() {
            return;
        }
        let before = self.canvas.nodes.clone();
        let sel: HashSet<String> = self.selection.iter().cloned().collect();
        f(&mut self.canvas.nodes, &sel);
        if self.canvas.nodes != before {
            self.undo.push(before);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    /// На самый верх, сохраняя порядок выделенных между собой.
    pub fn bring_to_front(&mut self) {
        self.reorder(|nodes, sel| {
            let (picked, rest): (Vec<Node>, Vec<Node>) =
                nodes.drain(..).partition(|n| sel.contains(&n.id));
            nodes.extend(rest);
            nodes.extend(picked);
        });
    }

    pub fn send_to_back(&mut self) {
        self.reorder(|nodes, sel| {
            let (picked, rest): (Vec<Node>, Vec<Node>) =
                nodes.drain(..).partition(|n| sel.contains(&n.id));
            nodes.extend(picked);
            nodes.extend(rest);
        });
    }

    /// На один слой выше: каждый выделенный меняется с соседом сверху,
    /// если тот не выделен.
    pub fn bring_forward(&mut self) {
        self.reorder(|nodes, sel| {
            for i in (0..nodes.len().saturating_sub(1)).rev() {
                if sel.contains(&nodes[i].id) && !sel.contains(&nodes[i + 1].id) {
                    nodes.swap(i, i + 1);
                }
            }
        });
    }

    pub fn send_backward(&mut self) {
        self.reorder(|nodes, sel| {
            for i in 1..nodes.len() {
                if sel.contains(&nodes[i].id) && !sel.contains(&nodes[i - 1].id) {
                    nodes.swap(i, i - 1);
                }
            }
        });
    }

    // ---- Буфер обмена ----------------------------------------------------

    /// Выделенное как текст для буфера обмена. Пусто — нечего копировать.
    pub fn copy_selection(&self) -> Option<String> {
        let nodes: Vec<Node> = self
            .canvas
            .nodes
            .iter()
            .filter(|n| self.is_selected(&n.id))
            .cloned()
            .collect();
        if nodes.is_empty() {
            return None;
        }
        Some(serde_json::to_string(&Clip { marker: 1, nodes }).expect("JSON из своих данных"))
    }

    /// Вставляет объекты из буфера. Каждый получает новый id, вся вставка
    /// сдвигается на `offset` и становится выделением. Чужой текст —
    /// `false`, холст не меняется.
    pub fn paste(&mut self, text: &str, offset: f64) -> bool {
        let Ok(clip) = serde_json::from_str::<Clip>(text) else {
            return false;
        };
        if clip.nodes.is_empty() {
            return false;
        }
        self.checkpoint();
        self.selection.clear();
        for mut n in clip.nodes {
            n.id = Uuid::new_v4().to_string();
            n.x += offset;
            n.y += offset;
            self.selection.push(n.id.clone());
            self.canvas.nodes.push(n);
        }
        true
    }

    /// Ctrl+D: копия выделенного рядом.
    pub fn duplicate(&mut self) -> bool {
        match self.copy_selection() {
            Some(text) => self.paste(&text, PASTE_OFFSET),
            None => false,
        }
    }

    /// Холст с правками — для сохранения через ядро.
    pub fn into_canvas(self) -> Canvas {
        self.canvas
    }

    /// Подменяет содержимое (пришла версия из облака). История отмены
    /// сбрасывается: её шаги относились к старому содержимому.
    pub fn replace_canvas(&mut self, canvas: Canvas) {
        self.canvas = canvas;
        self.undo.clear();
        self.redo.clear();
        self.keep_selection_valid();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    fn ed() -> Editor {
        Editor::new(Canvas::new())
    }

    fn ids(e: &Editor) -> Vec<String> {
        e.nodes().iter().map(|n| n.id.clone()).collect()
    }

    #[test]
    fn add_card_is_centered_snapped_and_selected() {
        let mut e = ed();
        let id = e.add_card(p(125.0, 77.0), Some(GRID));
        let n = e.node(&id).unwrap();
        assert_eq!((n.x, n.y), (0.0, 0.0), "125-120=5 → 0, 77-70=7 → 0");
        assert_eq!((n.width, n.height), CARD_SIZE);
        assert_eq!(n.kind, "card");
        assert_eq!(e.selection(), std::slice::from_ref(&id));
        let id2 = e.add_card(p(125.0, 77.0), None);
        assert_eq!(e.node(&id2).unwrap().x, 5.0);
    }

    #[test]
    fn undo_redo_and_new_edit_clears_redo() {
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None);
        e.checkpoint();
        e.move_selection(10.0, 0.0);
        assert!(e.undo());
        assert_eq!(e.node(&a).unwrap().x, -120.0);
        assert!(e.redo());
        assert_eq!(e.node(&a).unwrap().x, -110.0);
        assert!(e.undo());
        assert!(e.undo());
        assert!(e.nodes().is_empty());
        assert!(
            e.selection().is_empty(),
            "выделение не указывает на пропавшее"
        );
        assert!(!e.undo());
        assert!(e.redo());
        e.add_card(p(0.0, 0.0), None);
        assert!(!e.can_redo(), "новая правка обрывает ветку повтора");
    }

    #[test]
    fn undo_history_is_limited() {
        let mut e = ed();
        for _ in 0..UNDO_LIMIT + 50 {
            e.add_card(p(0.0, 0.0), None);
        }
        let mut n = 0;
        while e.undo() {
            n += 1;
        }
        assert_eq!(n, UNDO_LIMIT);
        assert_eq!(e.nodes().len(), 50);
    }

    #[test]
    fn click_without_move_leaves_no_undo_step() {
        let mut e = ed();
        e.add_card(p(0.0, 0.0), None);
        e.checkpoint();
        e.drop_empty_checkpoint();
        assert!(e.undo());
        assert!(e.nodes().is_empty());
    }

    #[test]
    fn hit_finds_topmost() {
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None);
        let b = e.add_card(p(100.0, 0.0), None);
        assert_eq!(e.hit(p(50.0, 0.0)), Some(b.as_str()));
        assert_eq!(e.hit(p(-100.0, 0.0)), Some(a.as_str()));
        assert_eq!(e.hit(p(1000.0, 0.0)), None);
    }

    #[test]
    fn marquee_and_toggle_selection() {
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None);
        let b = e.add_card(p(1000.0, 0.0), None);
        e.select_in(
            Rect::from_corners(p(200.0, 200.0), p(-200.0, -200.0)),
            false,
        );
        assert_eq!(e.selection(), std::slice::from_ref(&a));
        e.toggle_selected(&b);
        assert_eq!(e.selection().len(), 2);
        e.toggle_selected(&a);
        assert_eq!(e.selection(), std::slice::from_ref(&b));
        e.select_in(r(-500.0, -500.0, 10.0, 10.0), true);
        assert_eq!(
            e.selection(),
            [b],
            "ничего не задели — выделение не пропало"
        );
        e.select_all();
        assert_eq!(e.selection().len(), 2);
    }

    #[test]
    fn move_and_resize_with_minimum() {
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None);
        e.move_selection(5.0, -5.0);
        assert_eq!(e.node(&a).unwrap().y, -75.0);
        e.set_rect(&a, r(0.0, 0.0, 1.0, 1.0));
        let n = e.node(&a).unwrap();
        assert_eq!((n.width, n.height), MIN_SIZE);
    }

    #[test]
    fn delete_is_undoable() {
        let mut e = ed();
        e.add_card(p(0.0, 0.0), None);
        assert!(e.delete_selection());
        assert!(e.nodes().is_empty());
        assert!(!e.delete_selection(), "пустое выделение — нечего удалять");
        e.undo();
        assert_eq!(e.nodes().len(), 1);
    }

    #[test]
    fn layer_order() {
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None);
        let b = e.add_card(p(0.0, 0.0), None);
        let c = e.add_card(p(0.0, 0.0), None);
        e.select_only(&a);
        e.bring_to_front();
        assert_eq!(ids(&e), [b.clone(), c.clone(), a.clone()]);
        e.send_to_back();
        assert_eq!(ids(&e), [a.clone(), b.clone(), c.clone()]);
        e.bring_forward();
        assert_eq!(ids(&e), [b.clone(), a.clone(), c.clone()]);
        e.send_backward();
        e.send_backward();
        assert_eq!(
            ids(&e),
            [a.clone(), b.clone(), c.clone()],
            "ниже дна некуда"
        );
        e.select_only(&c);
        let steps_before = {
            let mut copy = e.clone();
            let mut n = 0;
            while copy.undo() {
                n += 1;
            }
            n
        };
        e.bring_to_front();
        let mut n = 0;
        while e.undo() {
            n += 1;
        }
        assert_eq!(n, steps_before, "ничего не поменялось — шага отмены нет");
    }

    #[test]
    fn copy_paste_gives_new_ids_and_offset() {
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None);
        e.set_text(&a, "текст");
        let clip = e.copy_selection().unwrap();
        assert!(e.paste(&clip, 20.0));
        assert_eq!(e.nodes().len(), 2);
        let pasted = &e.nodes()[1];
        assert_ne!(pasted.id, a);
        assert_eq!(pasted.text, "текст");
        assert_eq!(pasted.x, e.nodes()[0].x + 20.0);
        assert_eq!(e.selection(), std::slice::from_ref(&pasted.id));
        assert!(e.duplicate());
        assert_eq!(e.nodes().len(), 3);
        assert!(!e.paste("просто текст", 20.0));
        assert!(!e.paste(r#"{"bolh-place-clipboard":1,"nodes":[]}"#, 20.0));
        e.clear_selection();
        assert_eq!(e.copy_selection(), None);
    }

    #[test]
    fn snap_to_grid_when_no_neighbour() {
        let s = snap(r(8.0, 29.0, 100.0, 50.0), &[], Some(20.0), 6.0);
        assert_eq!((s.dx, s.dy), (-8.0, -9.0));
        assert!(s.guides.is_empty());
        let s = snap(r(13.0, 29.0, 100.0, 50.0), &[], None, 6.0);
        assert_eq!((s.dx, s.dy), (0.0, 0.0));
    }

    #[test]
    fn snap_to_neighbour_edges_and_centers() {
        let other = r(200.0, 0.0, 100.0, 100.0);
        // Левый край почти у левого края соседа, центр по высоте — у его центра.
        let s = snap(r(203.0, 300.0, 50.0, 100.0), &[other], Some(20.0), 6.0);
        assert_eq!(s.dx, -3.0);
        assert!(s.guides.contains(&Guide::Vertical(200.0)));
        assert_eq!(
            s.dy, 0.0,
            "по вертикали соседа рядом нет — к сетке: 300 уже на сетке"
        );
        // Центр к центру: середина 52 ближе к середине соседа 50, чем верх 47.
        let s = snap(r(0.0, 47.0, 50.0, 10.0), &[other], None, 6.0);
        assert_eq!(s.dy, -2.0);
        assert!(s.guides.contains(&Guide::Horizontal(50.0)));
        // Дальше порога — не прилипает к соседу.
        let s = snap(r(330.0, 500.0, 50.0, 50.0), &[other], None, 6.0);
        assert_eq!((s.dx, s.dy), (0.0, 0.0));
        assert!(s.guides.is_empty());
    }

    #[test]
    fn snap_prefers_closest_line() {
        let a = r(100.0, 0.0, 10.0, 10.0);
        let b = r(104.0, 0.0, 10.0, 10.0);
        let s = snap(r(103.0, 400.0, 10.0, 10.0), &[a, b], None, 6.0);
        assert_eq!(s.dx, 1.0);
    }

    #[test]
    fn replace_from_cloud_resets_history() {
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None);
        let mut other = Canvas::new();
        other
            .nodes
            .push(Node::card(a.clone(), 5.0, 5.0, 10.0, 10.0));
        e.replace_canvas(other);
        assert!(!e.can_undo());
        assert_eq!(e.selection(), [a]);
        e.replace_canvas(Canvas::new());
        assert!(e.selection().is_empty());
    }

    #[test]
    fn view_round_trip_and_zoom_keeps_point_under_cursor() {
        let mut v = View::default();
        v.pan(100.0, 50.0);
        let w = p(30.0, -20.0);
        let s = v.to_screen(w);
        assert_eq!(v.to_world(s), w);
        let cursor = p(400.0, 300.0);
        let under = v.to_world(cursor);
        v.zoom_at(cursor, 2.0);
        assert_eq!(v.zoom, 2.0);
        let after = v.to_world(cursor);
        assert!((after.x - under.x).abs() < 1e-9 && (after.y - under.y).abs() < 1e-9);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut v = View::default();
        for _ in 0..50 {
            v.zoom_at(p(0.0, 0.0), 2.0);
        }
        assert_eq!(v.zoom, ZOOM_MAX);
        for _ in 0..50 {
            v.zoom_at(p(0.0, 0.0), 0.5);
        }
        assert_eq!(v.zoom, ZOOM_MIN);
    }

    #[test]
    fn fit_centers_and_never_magnifies() {
        let mut v = View::default();
        v.fit(r(0.0, 0.0, 100.0, 100.0), 1000.0, 800.0);
        assert_eq!(v.zoom, 1.0);
        let c = v.to_screen(p(50.0, 50.0));
        assert_eq!((c.x, c.y), (500.0, 400.0));
        v.fit(r(-5000.0, 0.0, 10000.0, 100.0), 1000.0, 800.0);
        assert!(v.zoom < 0.1 + 1e-9);
        let vis = v.visible(1000.0, 800.0);
        assert!(vis.contains(p(0.0, 50.0)));
    }
}

//! Правка холста без экрана: объекты, выделение, отмена и повтор,
//! прилипание к сетке и направляющим, порядок слоёв, копирование.
//!
//! Координаты здесь — «мировые», в пикселях холста при масштабе 100%.
//! Перевод в точки экрана — забота интерфейса.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::format::{Canvas, Node};
use crate::rich::RichText;

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

/// Ближайшая к одному из моих краёв чужая линия, если ближе `threshold`:
/// (поправка, линия).
fn nearest(mine: &[f64], lines: impl Iterator<Item = f64>, threshold: f64) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for line in lines {
        for m in mine {
            let d = line - m;
            if d.abs() <= threshold && best.is_none_or(|(b, _)| d.abs() < b.abs()) {
                best = Some((d, line));
            }
        }
    }
    best
}

/// Притяжение к линии сетки — только когда край уже рядом с ней. Всегда
/// округлять до сетки нельзя: объект шёл бы за мышью рывками по 20 точек,
/// и это ощущалось как тормоза.
fn grid_pull(v: f64, grid: f64, threshold: f64) -> Option<f64> {
    let d = (v / grid).round() * grid - v;
    (d.abs() <= threshold).then_some(d)
}

/// Прилипание по одной оси: к соседу важнее, чем к сетке.
fn snap_axis(
    mine: &[f64],
    lines: impl Iterator<Item = f64>,
    grid_edge: f64,
    grid: Option<f64>,
    threshold: f64,
) -> (f64, Option<f64>) {
    match nearest(mine, lines, threshold) {
        Some((d, line)) => (d, Some(line)),
        None => (
            grid.and_then(|g| grid_pull(grid_edge, g, threshold))
                .unwrap_or(0.0),
            None,
        ),
    }
}

/// Куда прилипнуть перемещаемому прямоугольнику.
///
/// К краям и серединам других объектов, если они ближе `threshold`;
/// иначе, если сетка включена и левый (верхний) край рядом с линией
/// сетки, — к ней. Оси независимы. Вдали от линий объект идёт точно за
/// мышью.
pub fn snap(moving: Rect, others: &[Rect], grid: Option<f64>, threshold: f64) -> Snap {
    let (dx, vx) = snap_axis(
        &moving.xs(),
        others.iter().flat_map(Rect::xs),
        moving.x,
        grid,
        threshold,
    );
    let (dy, hy) = snap_axis(
        &moving.ys(),
        others.iter().flat_map(Rect::ys),
        moving.y,
        grid,
        threshold,
    );
    Snap {
        dx,
        dy,
        guides: vx
            .map(Guide::Vertical)
            .into_iter()
            .chain(hy.map(Guide::Horizontal))
            .collect(),
    }
}

/// Прилипание тянущихся краёв при изменении размера: `x` — где сейчас
/// вертикальный край, который тянут, `y` — горизонтальный (`None` — этот
/// край не тянется).
pub fn snap_edges(
    x: Option<f64>,
    y: Option<f64>,
    others: &[Rect],
    grid: Option<f64>,
    threshold: f64,
) -> Snap {
    let mut out = Snap::default();
    if let Some(x) = x {
        let (d, line) = snap_axis(&[x], others.iter().flat_map(Rect::xs), x, grid, threshold);
        out.dx = d;
        out.guides.extend(line.map(Guide::Vertical));
    }
    if let Some(y) = y {
        let (d, line) = snap_axis(&[y], others.iter().flat_map(Rect::ys), y, grid, threshold);
        out.dy = d;
        out.guides.extend(line.map(Guide::Horizontal));
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

/// Одинаковы ли два набора объектов. Сначала сравниваем указатели: у
/// снимка отмены и текущего состояния неизменённые объекты общие.
fn same_nodes(a: &[Arc<Node>], b: &[Arc<Node>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y) || x == y)
}

/// Открытый холст: содержимое, выделение и история правок.
///
/// Объекты лежат в `Arc`: снимок для отмены копирует только указатели, а
/// сам объект копируется, лишь когда его меняют. Поэтому 200 шагов отмены
/// на холсте в 5000 объектов не съедают сотни мегабайт.
#[derive(Debug, Clone)]
pub struct Editor {
    /// Всё, кроме объектов: формат, id, связи, неизвестные поля.
    meta: Canvas,
    nodes: Vec<Arc<Node>>,
    /// id → место в `nodes`. Без него поиск по id перебирал бы весь холст,
    /// и на 5000 объектах кадр стоил десятки миллисекунд.
    index: HashMap<String, usize>,
    selection: Vec<String>,
    selected: HashSet<String>,
    /// Высота объекта на экране, если текст сделал его выше заданной, в
    /// точках холста. Только для показа и попадания мышью — в файл не
    /// пишется: файл меняется лишь от действий человека.
    shown: HashMap<String, f64>,
    undo: Vec<Vec<Arc<Node>>>,
    redo: Vec<Vec<Arc<Node>>>,
}

impl Editor {
    pub fn new(mut canvas: Canvas) -> Self {
        let nodes = std::mem::take(&mut canvas.nodes)
            .into_iter()
            .map(Arc::new)
            .collect();
        let mut e = Editor {
            meta: canvas,
            nodes,
            index: HashMap::new(),
            selection: Vec::new(),
            selected: HashSet::new(),
            shown: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        };
        e.reindex();
        e
    }

    fn reindex(&mut self) {
        self.index = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
    }

    /// Холст со всеми правками — для сохранения. Версия — текущая: в нём
    /// могут быть карточки с оформлением.
    pub fn to_canvas(&self) -> Canvas {
        let mut c = self.meta.clone();
        c.version = c.version.max(crate::format::CANVAS_VERSION);
        c.nodes = self.nodes.iter().map(|n| (**n).clone()).collect();
        c
    }

    pub fn nodes(&self) -> &[Arc<Node>] {
        &self.nodes
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.index.get(id).map(|&i| &*self.nodes[i])
    }

    fn node_mut(&mut self, id: &str) -> Option<&mut Node> {
        let i = *self.index.get(id)?;
        Some(Arc::make_mut(&mut self.nodes[i]))
    }

    /// Прямоугольник объекта, как его видит человек: с высотой по тексту.
    pub fn rect(&self, n: &Node) -> Rect {
        let mut r = Rect::of(n);
        if let Some(&h) = self.shown.get(&n.id) {
            r.h = r.h.max(h);
        }
        r
    }

    pub fn rect_of(&self, id: &str) -> Option<Rect> {
        self.node(id).map(|n| self.rect(n))
    }

    /// Интерфейс сообщает, какой высоты объект вышел на экране.
    pub fn set_shown_height(&mut self, id: &str, h: f64) {
        self.shown.insert(id.to_string(), h);
    }

    pub fn selection(&self) -> &[String] {
        &self.selection
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.selected.contains(id)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn push_undo(&mut self, snapshot: Vec<Arc<Node>>) {
        self.undo.push(snapshot);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Запоминает состояние перед правкой. Перетаскивание вызывает его один
    /// раз в начале, и тогда вся протяжка отменяется одним Ctrl+Z.
    pub fn checkpoint(&mut self) {
        self.push_undo(self.nodes.clone());
    }

    /// Убирает последнюю точку отмены, если после неё ничего не поменялось
    /// (щёлкнули по карточке, но не сдвинули).
    /// `true`, если точка оказалась пустой и убрана.
    pub fn drop_empty_checkpoint(&mut self) -> bool {
        if self.undo.last().is_some_and(|u| same_nodes(u, &self.nodes)) {
            self.undo.pop();
            true
        } else {
            false
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        let now = std::mem::replace(&mut self.nodes, prev);
        self.redo.push(now);
        self.reindex();
        self.keep_selection_valid();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        let now = std::mem::replace(&mut self.nodes, next);
        self.undo.push(now);
        self.reindex();
        self.keep_selection_valid();
        true
    }

    fn keep_selection_valid(&mut self) {
        let index = &self.index;
        self.selection.retain(|s| index.contains_key(s));
        self.selected.retain(|s| index.contains_key(s));
    }

    // ---- Выделение -------------------------------------------------------

    /// Верхний объект под точкой.
    pub fn hit(&self, p: Point) -> Option<&str> {
        self.nodes
            .iter()
            .rev()
            .find(|n| self.rect(n).contains(p))
            .map(|n| n.id.as_str())
    }

    fn set_selection(&mut self, ids: Vec<String>) {
        self.selected = ids.iter().cloned().collect();
        self.selection = ids;
    }

    pub fn select_only(&mut self, id: &str) {
        self.set_selection(vec![id.to_string()]);
    }

    /// Ctrl/Shift+щелчок: добавить или убрать из выделения.
    pub fn toggle_selected(&mut self, id: &str) {
        if self.selected.remove(id) {
            self.selection.retain(|s| s != id);
        } else {
            self.selected.insert(id.to_string());
            self.selection.push(id.to_string());
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.selected.clear();
    }

    pub fn select_all(&mut self) {
        let all = self.nodes.iter().map(|n| n.id.clone()).collect();
        self.set_selection(all);
    }

    /// Рамка выделения: всё, что задевает прямоугольник. С `add` —
    /// добавить к уже выделенному.
    pub fn select_in(&mut self, r: Rect, add: bool) {
        if !add {
            self.clear_selection();
        }
        let hits: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| self.rect(n).intersects(&r) && !self.selected.contains(&n.id))
            .map(|n| n.id.clone())
            .collect();
        for id in hits {
            self.selected.insert(id.clone());
            self.selection.push(id);
        }
    }

    pub fn selection_bounds(&self) -> Option<Rect> {
        self.selection
            .iter()
            .filter_map(|id| self.rect_of(id))
            .reduce(|a, b| a.union(&b))
    }

    /// Объекты вне выделения, задевающие `area`, — к ним прилипает
    /// перемещаемое. Дальние не нужны: прилипать к невидимому странно, а
    /// перебирать их каждый кадр — дорого.
    pub fn unselected_rects_in(&self, area: Rect) -> Vec<Rect> {
        self.nodes
            .iter()
            .filter(|n| !self.selected.contains(&n.id))
            .map(|n| self.rect(n))
            .filter(|r| r.intersects(&area))
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
        self.nodes
            .push(Arc::new(Node::card(id.clone(), x, y, w, h)));
        self.index.insert(id.clone(), self.nodes.len() - 1);
        self.select_only(&id);
        id
    }

    /// Сдвигает выделенное. Точку отмены ставит вызывающий.
    pub fn move_selection(&mut self, dx: f64, dy: f64) {
        for id in self.selection.clone() {
            if let Some(n) = self.node_mut(&id) {
                n.x += dx;
                n.y += dy;
            }
        }
    }

    /// Задаёт объекту размер и место, не меньше [`MIN_SIZE`]. Точку отмены
    /// ставит вызывающий.
    pub fn set_rect(&mut self, id: &str, r: Rect) {
        if let Some(n) = self.node_mut(id) {
            n.x = r.x;
            n.y = r.y;
            n.width = r.w.max(MIN_SIZE.0);
            n.height = r.h.max(MIN_SIZE.1);
        }
    }

    /// Меняет текст карточки. Точку отмены ставит вызывающий — один раз за
    /// всё время правки текста, а не на каждую букву.
    /// Меняет текст объекта. Точку отмены ставит вызывающий: правка текста
    /// карточки — один шаг от начала до конца.
    pub fn set_rich(&mut self, id: &str, rich: &RichText) {
        if self.node(id).is_some_and(|n| &n.rich != rich)
            && let Some(n) = self.node_mut(id)
        {
            n.rich = rich.clone();
        }
    }

    /// Отмечает пункт чек-листа щелчком вне правки текста — отдельный шаг
    /// отмены. `false`, если там не пункт чек-листа.
    pub fn toggle_check(&mut self, id: &str, block: usize) -> bool {
        let is_check = self
            .node(id)
            .and_then(|n| n.rich.blocks.get(block))
            .is_some_and(|b| matches!(b.kind, crate::rich::Kind::Check(_)));
        if !is_check {
            return false;
        }
        self.checkpoint();
        let n = self.node_mut(id).expect("проверено выше");
        crate::rich::toggle_check(&mut n.rich, block)
    }

    /// Меняет объект целиком через функцию. Точку отмены ставит вызывающий.
    pub fn update_node(&mut self, id: &str, f: impl FnOnce(&mut Node)) {
        if let Some(n) = self.node_mut(id) {
            f(n);
        }
    }

    pub fn delete_selection(&mut self) -> bool {
        if self.selection.is_empty() {
            return false;
        }
        self.checkpoint();
        let sel = std::mem::take(&mut self.selected);
        self.selection.clear();
        self.nodes.retain(|n| !sel.contains(&n.id));
        for id in &sel {
            self.shown.remove(id);
        }
        self.reindex();
        true
    }

    // ---- Порядок слоёв ---------------------------------------------------

    fn reorder(&mut self, f: impl FnOnce(&mut Vec<Arc<Node>>, &HashSet<String>)) {
        if self.selection.is_empty() {
            return;
        }
        let before = self.nodes.clone();
        let sel = self.selected.clone();
        f(&mut self.nodes, &sel);
        if !same_nodes(&self.nodes, &before) {
            self.push_undo(before);
            self.reindex();
        }
    }

    /// На самый верх, сохраняя порядок выделенных между собой.
    pub fn bring_to_front(&mut self) {
        self.reorder(|nodes, sel| {
            let (picked, rest): (Vec<_>, Vec<_>) =
                nodes.drain(..).partition(|n| sel.contains(&n.id));
            nodes.extend(rest);
            nodes.extend(picked);
        });
    }

    pub fn send_to_back(&mut self) {
        self.reorder(|nodes, sel| {
            let (picked, rest): (Vec<_>, Vec<_>) =
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
    /// Порядок — как на холсте, снизу вверх.
    pub fn copy_selection(&self) -> Option<String> {
        let nodes: Vec<Node> = self
            .nodes
            .iter()
            .filter(|n| self.selected.contains(&n.id))
            .map(|n| (**n).clone())
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
        let mut ids = Vec::new();
        for mut n in clip.nodes {
            n.id = Uuid::new_v4().to_string();
            n.x += offset;
            n.y += offset;
            ids.push(n.id.clone());
            self.nodes.push(Arc::new(n));
        }
        self.reindex();
        self.set_selection(ids);
        true
    }

    /// Ctrl+D: копия выделенного рядом.
    pub fn duplicate(&mut self) -> bool {
        match self.copy_selection() {
            Some(text) => self.paste(&text, PASTE_OFFSET),
            None => false,
        }
    }

    /// Подменяет содержимое (пришла версия из облака). История отмены
    /// сбрасывается: её шаги относились к старому содержимому.
    pub fn id(&self) -> uuid::Uuid {
        self.meta.id
    }

    /// Холст стал другим файлом — копией после расхождения — со своей
    /// историей. Отмене это не подлежит: id не правка человека.
    pub fn set_id(&mut self, id: uuid::Uuid) {
        self.meta.id = id;
    }

    pub fn replace_canvas(&mut self, canvas: Canvas) {
        let selection = std::mem::take(&mut self.selection);
        *self = Editor::new(canvas);
        self.set_selection(selection);
        self.keep_selection_valid();
    }
}

/// Текст карточек, скопированных на холсте, — если их вставляют внутрь
/// текста другой карточки. Не наш буфер или карточки без текста — `None`.
pub fn clip_text(text: &str) -> Option<RichText> {
    let clip = serde_json::from_str::<Clip>(text).ok()?;
    let blocks: Vec<_> = clip
        .nodes
        .into_iter()
        .filter(|n| n.has_text())
        .flat_map(|n| n.rich.blocks)
        .collect();
    (!blocks.is_empty()).then(|| RichText::from_blocks(blocks))
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
        assert!(e.drop_empty_checkpoint());
        e.checkpoint();
        e.move_selection(1.0, 0.0);
        assert!(!e.drop_empty_checkpoint(), "сдвинули — точка нужна");
        assert!(e.undo());
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
        e.set_rich(&a, &RichText::from_plain("текст"));
        let clip = e.copy_selection().unwrap();
        assert!(e.paste(&clip, 20.0));
        assert_eq!(e.nodes().len(), 2);
        let pasted = &e.nodes()[1];
        assert_ne!(pasted.id, a);
        assert_eq!(pasted.plain_text(), "текст");
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
    fn grid_pulls_only_when_near_a_line() {
        // Рядом с линией сетки — прилипает.
        let s = snap(r(17.0, 23.0, 100.0, 50.0), &[], Some(20.0), 6.0);
        assert_eq!((s.dx, s.dy), (3.0, -3.0));
        assert!(s.guides.is_empty());
        // Посередине между линиями — идёт точно за мышью, без рывка.
        let s = snap(r(10.0, 30.0, 100.0, 50.0), &[], Some(20.0), 6.0);
        assert_eq!((s.dx, s.dy), (0.0, 0.0));
        let s = snap(r(17.0, 23.0, 100.0, 50.0), &[], None, 6.0);
        assert_eq!((s.dx, s.dy), (0.0, 0.0), "сетка выключена");
    }

    #[test]
    fn resize_edges_snap_to_neighbours_then_grid() {
        let other = r(200.0, 0.0, 100.0, 100.0);
        // Правый край у левого края соседа — к нему, с направляющей.
        let s = snap_edges(Some(197.0), None, &[other], Some(20.0), 6.0);
        assert_eq!(s.dx, 3.0);
        assert_eq!(s.guides, [Guide::Vertical(200.0)]);
        // Нижний край у линии сетки — к ней, без направляющей.
        let s = snap_edges(None, Some(398.0), &[other], Some(20.0), 6.0);
        assert_eq!(s.dy, 2.0);
        assert!(s.guides.is_empty());
        // Далеко от всего — точно за мышью.
        let s = snap_edges(Some(150.0), Some(350.0), &[other], Some(20.0), 6.0);
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
    fn shown_height_counts_for_hits_and_selection_but_not_the_file() {
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None); // y от -70 до 70
        assert_eq!(e.hit(p(0.0, 100.0)), None);
        e.set_shown_height(&a, 200.0); // текст вытянул карточку до y=130
        assert_eq!(e.hit(p(0.0, 100.0)), Some(a.as_str()));
        assert_eq!(e.selection_bounds().unwrap().h, 200.0);
        assert_eq!(
            e.node(&a).unwrap().height,
            140.0,
            "в файле — заданная высота"
        );
        assert_eq!(e.to_canvas().nodes[0].height, 140.0);
        // Меньше заданной высота на экране не бывает.
        e.set_shown_height(&a, 10.0);
        assert_eq!(e.rect_of(&a).unwrap().h, 140.0);
    }

    #[test]
    fn undo_snapshots_share_unchanged_nodes() {
        let mut e = ed();
        for i in 0..100 {
            e.add_card(p(i as f64 * 300.0, 0.0), None);
        }
        let first = e.nodes()[0].id.clone();
        e.select_only(&first);
        e.checkpoint();
        e.move_selection(5.0, 0.0);
        let snap = e.undo.last().unwrap();
        let shared = snap
            .iter()
            .zip(e.nodes())
            .filter(|(a, b)| Arc::ptr_eq(a, b))
            .count();
        assert_eq!(shared, 99, "скопирован только сдвинутый объект");
    }

    #[test]
    fn lookups_stay_correct_after_structural_changes() {
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None);
        let b = e.add_card(p(300.0, 0.0), None);
        e.select_only(&a);
        e.bring_to_front();
        assert_eq!(e.node(&a).unwrap().id, a);
        assert_eq!(e.node(&b).unwrap().id, b);
        e.delete_selection();
        assert!(e.node(&a).is_none());
        assert_eq!(e.node(&b).unwrap().id, b);
        e.undo();
        assert_eq!(e.node(&a).unwrap().id, a);
        e.select_all();
        e.duplicate();
        assert_eq!(e.nodes().len(), 4);
        for n in e.nodes() {
            assert_eq!(e.node(&n.id).unwrap().id, n.id);
        }
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
        assert_eq!(e.selection(), std::slice::from_ref(&a));
        assert!(e.is_selected(&a));
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

    #[test]
    fn checkbox_click_is_its_own_undo_step() {
        use crate::rich::{Block, Kind};
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None);
        let list = RichText::from_blocks(vec![
            Block::new(Kind::Check(false)).with_text("молоко"),
            Block::paragraph("абзац"),
        ]);
        e.set_rich(&a, &list);
        assert!(e.toggle_check(&a, 0));
        assert_eq!(e.node(&a).unwrap().rich.blocks[0].kind, Kind::Check(true));
        assert!(!e.toggle_check(&a, 1), "абзац не отмечается");
        assert!(!e.toggle_check("нет такой", 0));
        assert!(e.undo());
        assert_eq!(e.node(&a).unwrap().rich.blocks[0].kind, Kind::Check(false));
    }

    #[test]
    fn copied_cards_paste_into_text_as_their_text() {
        let mut e = ed();
        let a = e.add_card(p(0.0, 0.0), None);
        e.set_rich(&a, &RichText::from_plain("раз\nдва"));
        let b = e.add_card(p(300.0, 0.0), None);
        e.set_rich(&b, &RichText::from_plain("три"));
        e.add_card(p(600.0, 0.0), None);
        e.select_all();
        let clip = e.copy_selection().unwrap();
        assert_eq!(clip_text(&clip).unwrap().plain_text(), "раз\nдва\nтри");
        assert_eq!(clip_text("обычный текст"), None);
    }
}

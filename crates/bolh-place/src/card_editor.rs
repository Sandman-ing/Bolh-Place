//! Правка текста карточки: курсор, выделение, клавиши, системный ввод и
//! буфер обмена. Сами правки — в `bolh_core::rich::TextEditor`; здесь их
//! связь с окном.

use std::ops::Range;
use std::rc::Rc;

use bolh_core::rich::{Block, Kind, Mark, Pos, RichText, Selection, TextEditor};
use gpui_kit::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    KeyBinding, Pixels, Point, UTF16Selection, Window, point, px,
};

use crate::rich_text::TextLayout;

pub const CONTEXT: &str = "CardText";

gpui_kit::actions!(
    card_text,
    [
        Left,
        Right,
        SelectLeft,
        SelectRight,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        Up,
        Down,
        SelectUp,
        SelectDown,
        Home,
        End,
        SelectHome,
        SelectEnd,
        TextStart,
        TextEnd,
        SelectTextStart,
        SelectTextEnd,
        Backspace,
        Delete,
        DeleteWordLeft,
        DeleteWordRight,
        Newline,
        Indent,
        Outdent,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Bold,
        Italic,
        Underline,
        Strike,
        Heading,
        Heading2,
        Paragraph,
        BulletList,
        NumberedList,
        CheckList,
    ]
);

pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("ctrl-left", WordLeft, c),
        KeyBinding::new("ctrl-right", WordRight, c),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, c),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("shift-home", SelectHome, c),
        KeyBinding::new("shift-end", SelectEnd, c),
        KeyBinding::new("ctrl-home", TextStart, c),
        KeyBinding::new("ctrl-end", TextEnd, c),
        KeyBinding::new("ctrl-shift-home", SelectTextStart, c),
        KeyBinding::new("ctrl-shift-end", SelectTextEnd, c),
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, c),
        KeyBinding::new("ctrl-delete", DeleteWordRight, c),
        KeyBinding::new("enter", Newline, c),
        KeyBinding::new("shift-enter", Newline, c),
        KeyBinding::new("tab", Indent, c),
        KeyBinding::new("shift-tab", Outdent, c),
        KeyBinding::new("ctrl-a", SelectAll, c),
        KeyBinding::new("ctrl-c", Copy, c),
        KeyBinding::new("ctrl-insert", Copy, c),
        KeyBinding::new("ctrl-x", Cut, c),
        KeyBinding::new("shift-delete", Cut, c),
        KeyBinding::new("ctrl-v", Paste, c),
        KeyBinding::new("shift-insert", Paste, c),
        KeyBinding::new("ctrl-z", Undo, c),
        KeyBinding::new("ctrl-y", Redo, c),
        KeyBinding::new("ctrl-shift-z", Redo, c),
        KeyBinding::new("ctrl-b", Bold, c),
        KeyBinding::new("ctrl-i", Italic, c),
        KeyBinding::new("ctrl-u", Underline, c),
        KeyBinding::new("ctrl-shift-x", Strike, c),
        // Как в Google Документах.
        KeyBinding::new("ctrl-alt-1", Heading, c),
        KeyBinding::new("ctrl-alt-2", Heading2, c),
        KeyBinding::new("ctrl-alt-0", Paragraph, c),
        KeyBinding::new("ctrl-shift-7", NumberedList, c),
        KeyBinding::new("ctrl-shift-8", BulletList, c),
        KeyBinding::new("ctrl-shift-9", CheckList, c),
    ]);
}

pub enum CardEditorEvent {
    /// Текст поменялся — холст кладёт его в карточку.
    Changed,
}

pub struct CardEditor {
    editor: TextEditor,
    /// Что сейчас набирается через систему ввода и ещё не подтверждено.
    marked: Option<(Pos, Pos)>,
    pub focus_handle: FocusHandle,
    /// Вёрстка последнего кадра и где он стоял в окне.
    layout: Option<Rc<TextLayout>>,
    bounds: Bounds<Pixels>,
    /// Куда стремится курсор при движении вверх-вниз: сквозь короткие
    /// строки он возвращается на прежнее место по горизонтали.
    goal_x: Option<Pixels>,
    /// Тянут мышью выделение.
    selecting: bool,
}

impl EventEmitter<CardEditorEvent> for CardEditor {}

impl Focusable for CardEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl CardEditor {
    pub fn new(text: RichText, cx: &mut Context<Self>) -> Self {
        CardEditor {
            editor: TextEditor::new(text),
            marked: None,
            focus_handle: cx.focus_handle(),
            layout: None,
            bounds: Bounds::default(),
            goal_x: None,
            selecting: false,
        }
    }

    pub fn text(&self) -> &RichText {
        self.editor.text()
    }

    pub fn selection(&self) -> Selection {
        self.editor.selection()
    }

    pub fn marked(&self) -> Option<(Pos, Pos)> {
        self.marked
    }

    pub fn has_mark(&self, mark: Mark) -> bool {
        self.editor.has_mark(mark)
    }

    pub fn block_kind(&self) -> Option<Kind> {
        self.editor.block_kind()
    }

    pub(crate) fn set_layout(&mut self, layout: Rc<TextLayout>, bounds: Bounds<Pixels>) {
        self.layout = Some(layout);
        self.bounds = bounds;
    }

    /// Курсор в это место окна — правку начали щелчком.
    pub fn place_caret(&mut self, layout: &TextLayout, at: Point<Pixels>, origin: Point<Pixels>) {
        let pos = layout.pos_at(at - origin);
        self.editor.move_to(pos, false);
    }

    // ---- Общие шаги -----------------------------------------------------

    /// Правка текста. Событие — только если текст и правда поменялся:
    /// иначе холст считал бы, что есть что сохранять.
    fn edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut TextEditor),
    ) {
        let before = self.editor.text().clone();
        f(&mut self.editor);
        self.marked = None;
        self.goal_x = None;
        if *self.editor.text() != before {
            window.notify_text_input_changed(&self.focus_handle);
            cx.emit(CardEditorEvent::Changed);
        }
        cx.notify();
    }

    fn motion(&mut self, window: &mut Window, cx: &mut Context<Self>, f: impl FnOnce(&mut Self)) {
        f(self);
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn go(&mut self, pos: Pos, extend: bool) {
        self.editor.move_to(pos, extend);
        self.goal_x = None;
    }

    fn local(&self, at: Point<Pixels>) -> Point<Pixels> {
        at - self.bounds.origin
    }

    // ---- Мышь (её приносит холст) --------------------------------------

    /// Нажали мышью в карточке: курсор, слово (двойной щелчок), строка
    /// (тройной) или галочка в квадрате чек-листа.
    pub fn mouse_down(
        &mut self,
        at: Point<Pixels>,
        shift: bool,
        clicks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.layout.clone() else {
            return;
        };
        let local = self.local(at);
        if clicks == 1
            && !shift
            && let Some(block) = layout.check_at(local)
        {
            return self.edit(window, cx, |e| {
                e.toggle_check(block);
            });
        }
        let pos = layout.pos_at(local);
        self.motion(window, cx, |this| {
            match clicks {
                1 => {
                    this.go(pos, shift);
                    this.selecting = true;
                }
                2 => this.editor.select_word(pos),
                _ => this.editor.set_selection(Selection {
                    anchor: this.editor.block_start(pos),
                    head: this.editor.block_end(pos),
                }),
            }
            this.goal_x = None;
        });
    }

    pub fn mouse_drag(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        if let Some(layout) = self.layout.clone() {
            let pos = layout.pos_at(self.local(at));
            self.motion(window, cx, |this| this.go(pos, true));
        }
    }

    pub fn mouse_up(&mut self) {
        self.selecting = false;
    }

    // ---- Клавиши --------------------------------------------------------

    fn horizontal(
        &mut self,
        right: bool,
        word: bool,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.motion(window, cx, |this| {
            let sel = this.editor.selection();
            // Стрелка без Shift при выделении — к его краю.
            if !extend && !sel.is_empty() && !word {
                let to = if right { sel.end() } else { sel.start() };
                return this.go(to, false);
            }
            let head = sel.head;
            let to = match (right, word) {
                (false, false) => this.editor.left_of(head),
                (true, false) => this.editor.right_of(head),
                (false, true) => this.editor.word_left_of(head),
                (true, true) => this.editor.word_right_of(head),
            };
            this.go(to, extend);
        });
    }

    fn vertical(&mut self, down: bool, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.clone() else {
            return;
        };
        self.motion(window, cx, |this| {
            let head = this.editor.selection().head;
            let x = this.goal_x.unwrap_or_else(|| layout.caret_at(head).0.x);
            let to = layout.vertical(head, x, down).unwrap_or(if down {
                this.editor.text().end()
            } else {
                Pos::default()
            });
            this.editor.move_to(to, extend);
            this.goal_x = Some(x);
        });
    }

    /// Начало или конец видимой строки — с переносами это не начало блока.
    fn line_edge(&mut self, end: bool, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.clone() else {
            return;
        };
        self.motion(window, cx, |this| {
            let head = this.editor.selection().head;
            let (caret, lh) = layout.caret_at(head);
            let x = if end { px(1_000_000.) } else { px(-1_000_000.) };
            let to = layout.pos_at(point(x, caret.y + lh / 2.));
            this.go(to, extend);
        });
    }

    fn text_edge(&mut self, end: bool, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.motion(window, cx, |this| {
            let to = if end {
                this.editor.text().end()
            } else {
                Pos::default()
            };
            this.go(to, extend);
        });
    }

    fn copy(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(rich) = self.editor.selected() else {
            return false;
        };
        // Простой текст — для чужих программ, оформление — рядом, для своих.
        cx.write_to_clipboard(ClipboardItem::new_string_with_json_metadata(
            rich.plain_text(),
            &rich.blocks,
        ));
        true
    }

    fn paste_item(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = item.text() else {
            return;
        };
        let text = text.replace("\r\n", "\n");
        let ours = item
            .metadata()
            .and_then(|m| serde_json::from_str::<Vec<Block>>(m).ok())
            .map(RichText::from_blocks)
            .filter(|r| r.plain_text() == text);
        // Карточки, скопированные на холсте, вставляются своим текстом, а
        // не служебной записью.
        let rich = ours.or_else(|| bolh_core::canvas::clip_text(&text));
        self.edit(window, cx, |e| match &rich {
            Some(rich) => e.paste_rich(rich),
            None => e.paste_plain(&text),
        });
    }

    /// Кнопки панели оформления.
    pub fn toggle_mark(&mut self, mark: Mark, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(window, cx, |e| e.toggle_mark(mark));
    }

    pub fn toggle_kind(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(window, cx, |e| e.toggle_kind(kind));
    }

    /// Все действия правки — на элемент с фокусом правки.
    pub fn actions(
        div: gpui_kit::Stateful<gpui_kit::Div>,
        editor: &gpui_kit::Entity<CardEditor>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        use gpui_kit::InteractiveElement as _;
        let on = |f: fn(&mut CardEditor, &mut Window, &mut Context<CardEditor>)| {
            let editor = editor.clone();
            move |window: &mut Window, cx: &mut App| editor.update(cx, |e, cx| f(e, window, cx))
        };
        macro_rules! act {
            ($div:expr, $action:ty, $f:expr) => {{
                let run = on($f);
                $div.on_action(move |_: &$action, window, cx| run(window, cx))
            }};
        }
        let d = div;
        let d = act!(d, Left, |e, w, cx| e.horizontal(false, false, false, w, cx));
        let d = act!(d, Right, |e, w, cx| e.horizontal(true, false, false, w, cx));
        let d = act!(d, SelectLeft, |e, w, cx| e
            .horizontal(false, false, true, w, cx));
        let d = act!(d, SelectRight, |e, w, cx| e
            .horizontal(true, false, true, w, cx));
        let d = act!(d, WordLeft, |e, w, cx| e
            .horizontal(false, true, false, w, cx));
        let d = act!(d, WordRight, |e, w, cx| e
            .horizontal(true, true, false, w, cx));
        let d = act!(d, SelectWordLeft, |e, w, cx| e
            .horizontal(false, true, true, w, cx));
        let d = act!(d, SelectWordRight, |e, w, cx| e
            .horizontal(true, true, true, w, cx));
        let d = act!(d, Up, |e, w, cx| e.vertical(false, false, w, cx));
        let d = act!(d, Down, |e, w, cx| e.vertical(true, false, w, cx));
        let d = act!(d, SelectUp, |e, w, cx| e.vertical(false, true, w, cx));
        let d = act!(d, SelectDown, |e, w, cx| e.vertical(true, true, w, cx));
        let d = act!(d, Home, |e, w, cx| e.line_edge(false, false, w, cx));
        let d = act!(d, End, |e, w, cx| e.line_edge(true, false, w, cx));
        let d = act!(d, SelectHome, |e, w, cx| e.line_edge(false, true, w, cx));
        let d = act!(d, SelectEnd, |e, w, cx| e.line_edge(true, true, w, cx));
        let d = act!(d, TextStart, |e, w, cx| e.text_edge(false, false, w, cx));
        let d = act!(d, TextEnd, |e, w, cx| e.text_edge(true, false, w, cx));
        let d = act!(d, SelectTextStart, |e, w, cx| e
            .text_edge(false, true, w, cx));
        let d = act!(d, SelectTextEnd, |e, w, cx| e.text_edge(true, true, w, cx));
        let d = act!(d, Backspace, |e, w, cx| e.edit(
            w,
            cx,
            TextEditor::backspace
        ));
        let d = act!(d, Delete, |e, w, cx| e.edit(
            w,
            cx,
            TextEditor::delete_forward
        ));
        let d = act!(d, DeleteWordLeft, |e, w, cx| e.edit(
            w,
            cx,
            TextEditor::delete_word_backward
        ));
        let d = act!(d, DeleteWordRight, |e, w, cx| e.edit(
            w,
            cx,
            TextEditor::delete_word_forward
        ));
        let d = act!(d, Newline, |e, w, cx| e.edit(w, cx, TextEditor::enter));
        let d = act!(d, Indent, |e, w, cx| e.edit(w, cx, |t| {
            t.indent(true);
        }));
        let d = act!(d, Outdent, |e, w, cx| e.edit(w, cx, |t| {
            t.indent(false);
        }));
        let d = act!(d, SelectAll, |e, w, cx| e
            .motion(w, cx, |this| this.editor.select_all()));
        let d = act!(d, Copy, |e, _, cx| {
            e.copy(cx);
        });
        let d = act!(d, Cut, |e, w, cx| {
            if e.copy(cx) {
                e.edit(w, cx, TextEditor::delete_selection);
            }
        });
        let d = act!(d, Paste, |e, w, cx| {
            if let Some(item) = cx.read_from_clipboard() {
                e.paste_item(item, w, cx);
            }
        });
        let d = act!(d, Undo, |e, w, cx| e.edit(w, cx, |t| {
            t.undo();
        }));
        let d = act!(d, Redo, |e, w, cx| e.edit(w, cx, |t| {
            t.redo();
        }));
        let d = act!(d, Bold, |e, w, cx| e.toggle_mark(Mark::Bold, w, cx));
        let d = act!(d, Italic, |e, w, cx| e.toggle_mark(Mark::Italic, w, cx));
        let d = act!(d, Underline, |e, w, cx| e.toggle_mark(
            Mark::Underline,
            w,
            cx
        ));
        let d = act!(d, Strike, |e, w, cx| e.toggle_mark(Mark::Strike, w, cx));
        let d = act!(d, Heading, |e, w, cx| e.toggle_kind(
            Kind::Heading(1),
            w,
            cx
        ));
        let d = act!(d, Heading2, |e, w, cx| e.toggle_kind(
            Kind::Heading(2),
            w,
            cx
        ));
        let d = act!(d, Paragraph, |e, w, cx| e.toggle_kind(
            Kind::Paragraph,
            w,
            cx
        ));
        let d = act!(d, BulletList, |e, w, cx| e.toggle_kind(Kind::Bullet, w, cx));
        let d = act!(d, NumberedList, |e, w, cx| e.toggle_kind(
            Kind::Numbered,
            w,
            cx
        ));
        act!(d, CheckList, |e, w, cx| e.toggle_kind(
            Kind::Check(false),
            w,
            cx
        ))
    }

    // ---- Системный ввод ---------------------------------------------------

    fn utf16_range(&self, range: &Range<usize>) -> (Pos, Pos) {
        let t = self.editor.text();
        (t.utf16_to_pos(range.start), t.utf16_to_pos(range.end))
    }

    fn pos_utf16(&self, pos: Pos) -> usize {
        self.editor.text().pos_to_utf16(pos)
    }
}

impl EntityInputHandler for CardEditor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let flat = self.editor.text().flat_text();
        let mut out = String::new();
        let (mut at, mut start, mut end) = (0, None, None);
        for ch in flat.chars() {
            if at >= range.start && start.is_none() {
                start = Some(at);
            }
            if at >= range.end {
                end = Some(at);
                break;
            }
            if start.is_some() {
                out.push(ch);
            }
            at += ch.len_utf16();
        }
        let start = start.unwrap_or(at);
        adjusted.replace(start..end.unwrap_or(at));
        Some(out)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let sel = self.editor.selection();
        Some(UTF16Selection {
            range: self.pos_utf16(sel.start())..self.pos_utf16(sel.end()),
            reversed: sel.head < sel.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .map(|(a, z)| self.pos_utf16(a)..self.pos_utf16(z))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        self.paste_item(item, window, cx);
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sel = self.editor.selection();
        let (start, end) = match (&range, self.marked) {
            (Some(r), _) => self.utf16_range(r),
            (None, Some(m)) => m,
            (None, None) => (sel.start(), sel.end()),
        };
        self.edit(window, cx, |e| e.replace(start, end, text));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sel = self.editor.selection();
        let (start, end) = match (&range, self.marked) {
            (Some(r), _) => self.utf16_range(r),
            (None, Some(m)) => m,
            (None, None) => (sel.start(), sel.end()),
        };
        self.edit(window, cx, |e| e.replace(start, end, text));
        let start = start.min(end);
        let inserted_end = self.editor.caret();
        self.marked = (!text.is_empty()).then_some((start, inserted_end));
        if let Some(r) = selected {
            let base = self.pos_utf16(start);
            let len = text.encode_utf16().count();
            let t = self.editor.text();
            let a = t.utf16_to_pos(base + r.start.min(len));
            let z = t.utf16_to_pos(base + r.end.min(len));
            self.editor.set_selection(Selection { anchor: a, head: z });
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let (a, z) = self.utf16_range(&range);
        let (pa, lh) = layout.caret_at(a);
        let (pz, _) = layout.caret_at(z);
        let width = if pz.y == pa.y {
            (pz.x - pa.x).max(px(1.))
        } else {
            px(1.)
        };
        Some(Bounds::new(
            element_bounds.origin + pa,
            gpui_kit::size(width, lh),
        ))
    }

    fn character_index_for_point(
        &mut self,
        at: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let pos = layout.pos_at(at - self.bounds.origin);
        Some(self.pos_utf16(pos))
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.editor.text().flat_text().encode_utf16().count())
    }
}

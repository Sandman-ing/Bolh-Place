//! Оформленный текст: карточки холста, а позже и документы (раздел 4).
//!
//! Текст — это блоки: абзац, заголовок, пункт маркированного или
//! нумерованного списка, пункт чек-листа. Внутри блока — куски с
//! оформлением: жирный, курсив, подчёркнутый, зачёркнутый.
//!
//! Здесь же все правки: ввод, Enter, Backspace, оформление, списки,
//! отступы, галочки, быстрые «- » и «[] » в начале строки, отмена. Без
//! экрана — поэтому правка проверяется тестами, а окно только рисует и
//! передаёт клавиши.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use unicode_segmentation::UnicodeSegmentation;

/// Глубже вкладывать списки в карточке не нужно: на узкой карточке текст
/// уехал бы за край.
pub const MAX_INDENT: u8 = 4;
/// Сколько шагов правки помнит отмена внутри одного сеанса правки.
const UNDO_LIMIT: usize = 500;

/// Оформление куска текста.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

/// Одно из оформлений — для кнопок и сочетаний клавиш.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Underline,
    Strike,
}

impl Marks {
    pub fn has(self, mark: Mark) -> bool {
        match mark {
            Mark::Bold => self.bold,
            Mark::Italic => self.italic,
            Mark::Underline => self.underline,
            Mark::Strike => self.strike,
        }
    }

    pub fn with(mut self, mark: Mark, on: bool) -> Marks {
        match mark {
            Mark::Bold => self.bold = on,
            Mark::Italic => self.italic = on,
            Mark::Underline => self.underline = on,
            Mark::Strike => self.strike = on,
        }
        self
    }

    pub fn is_plain(self) -> bool {
        self == Marks::default()
    }
}

/// Кусок текста с одним оформлением.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub marks: Marks,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Span {
        Span {
            text: text.into(),
            marks: Marks::default(),
        }
    }
}

/// Вид блока.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    /// Заголовок с уровнем 1–3.
    Heading(u8),
    Bullet,
    Numbered,
    /// Пункт чек-листа: отмечен или нет.
    Check(bool),
}

impl Kind {
    /// Пункт списка: у него есть отступ, Enter продолжает список.
    pub fn is_list(self) -> bool {
        matches!(self, Kind::Bullet | Kind::Numbered | Kind::Check(_))
    }

    /// Один ли это вид для кнопок: отмеченный и пустой пункт чек-листа —
    /// оба «чек-лист».
    fn same_family(self, other: Kind) -> bool {
        match (self, other) {
            (Kind::Check(_), Kind::Check(_)) => true,
            (a, b) => a == b,
        }
    }
}

/// Абзац, заголовок или пункт списка.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub kind: Kind,
    /// Уровень вложенности пункта списка, от 0 до [`MAX_INDENT`].
    pub indent: u8,
    /// Куски по порядку. Соседние куски с одним оформлением слиты, пустых
    /// нет; у пустого блока кусков нет вовсе.
    pub spans: Vec<Span>,
    /// Поля, которых эта версия не знает, — сохраняются как есть.
    pub extra: Map<String, Value>,
}

impl Block {
    pub fn new(kind: Kind) -> Block {
        Block {
            kind,
            indent: 0,
            spans: Vec::new(),
            extra: Map::new(),
        }
    }

    pub fn paragraph(text: impl Into<String>) -> Block {
        Block::new(Kind::Paragraph).with_text(text)
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Block {
        self.spans = vec![Span::plain(text)];
        normalize(&mut self.spans);
        self
    }

    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    pub fn len(&self) -> usize {
        self.spans.iter().map(|s| s.text.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Оформление, с которым продолжается ввод в этом месте: как у знака
    /// перед ним, а в начале блока — как у первого знака.
    pub fn marks_at(&self, offset: usize) -> Marks {
        let mut start = 0;
        for s in &self.spans {
            let end = start + s.text.len();
            if offset > start && offset <= end {
                return s.marks;
            }
            start = end;
        }
        self.spans.first().map(|s| s.marks).unwrap_or_default()
    }

    fn insert(&mut self, offset: usize, text: &str, marks: Marks) {
        let (mut head, tail) = split_spans(std::mem::take(&mut self.spans), offset);
        head.push(Span {
            text: text.to_string(),
            marks,
        });
        head.extend(tail);
        normalize(&mut head);
        self.spans = head;
    }

    fn remove(&mut self, start: usize, end: usize) {
        let (mut head, rest) = split_spans(std::mem::take(&mut self.spans), start);
        let (_, tail) = split_spans(rest, end - start);
        head.extend(tail);
        normalize(&mut head);
        self.spans = head;
    }

    /// Отрезает всё начиная с `offset` и возвращает отрезанное.
    fn split_off(&mut self, offset: usize) -> Vec<Span> {
        let (head, tail) = split_spans(std::mem::take(&mut self.spans), offset);
        self.spans = head;
        tail
    }

    fn append(&mut self, spans: Vec<Span>) {
        self.spans.extend(spans);
        normalize(&mut self.spans);
    }

    fn map_marks(&mut self, start: usize, end: usize, f: impl Fn(Marks) -> Marks) {
        let (mut head, rest) = split_spans(std::mem::take(&mut self.spans), start);
        let (mut mid, tail) = split_spans(rest, end - start);
        for s in &mut mid {
            s.marks = f(s.marks);
        }
        head.extend(mid);
        head.extend(tail);
        normalize(&mut head);
        self.spans = head;
    }

    /// Есть ли оформление на каждом знаке промежутка. У пустого промежутка
    /// ответа нет.
    fn all_have(&self, start: usize, end: usize, mark: Mark) -> Option<bool> {
        let mut pos = 0;
        let mut any = false;
        for s in &self.spans {
            let (a, b) = (pos, pos + s.text.len());
            pos = b;
            if b <= start || a >= end {
                continue;
            }
            any = true;
            if !s.marks.has(mark) {
                return Some(false);
            }
        }
        any.then_some(true)
    }

    /// Куски промежутка — для копирования.
    fn slice(&self, start: usize, end: usize) -> Vec<Span> {
        let (_, rest) = split_spans(self.spans.clone(), start);
        split_spans(rest, end - start).0
    }

    fn is_plain(&self) -> bool {
        self.kind == Kind::Paragraph
            && self.indent == 0
            && self.extra.is_empty()
            && self.spans.iter().all(|s| s.marks.is_plain())
    }
}

/// Делит куски по смещению в байтах: всё до него и всё после.
fn split_spans(spans: Vec<Span>, offset: usize) -> (Vec<Span>, Vec<Span>) {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    let mut pos = 0;
    for mut s in spans {
        let len = s.text.len();
        if pos + len <= offset {
            head.push(s);
        } else if pos >= offset {
            tail.push(s);
        } else {
            let rest = s.text.split_off(offset - pos);
            tail.push(Span {
                text: rest,
                marks: s.marks,
            });
            head.push(s);
        }
        pos += len;
    }
    (head, tail)
}

/// Сливает соседние куски с одним оформлением и выбрасывает пустые.
fn normalize(spans: &mut Vec<Span>) {
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans.drain(..) {
        if s.text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.marks == s.marks => last.text.push_str(&s.text),
            _ => out.push(s),
        }
    }
    *spans = out;
}

/// Место в тексте: номер блока и смещение в байтах внутри него — всегда на
/// границе знака.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Pos {
    pub block: usize,
    pub offset: usize,
}

impl Pos {
    pub fn new(block: usize, offset: usize) -> Pos {
        Pos { block, offset }
    }
}

/// Выделение: `anchor` — где начали, `head` — где курсор.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: Pos,
    pub head: Pos,
}

impl Selection {
    pub fn caret(pos: Pos) -> Selection {
        Selection {
            anchor: pos,
            head: pos,
        }
    }

    pub fn start(&self) -> Pos {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> Pos {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// Текст карточки целиком. В нём всегда есть хотя бы один блок — в нём
/// стоит курсор пустой карточки.
#[derive(Debug, Clone, PartialEq)]
pub struct RichText {
    pub blocks: Vec<Block>,
}

impl Default for RichText {
    fn default() -> Self {
        RichText {
            blocks: vec![Block::new(Kind::Paragraph)],
        }
    }
}

impl RichText {
    /// Простой текст: каждая строка — абзац.
    pub fn from_plain(text: &str) -> RichText {
        let blocks = text
            .split('\n')
            .map(|l| Block::paragraph(l.strip_suffix('\r').unwrap_or(l)))
            .collect();
        RichText { blocks }
    }

    pub fn from_blocks(blocks: Vec<Block>) -> RichText {
        if blocks.is_empty() {
            RichText::default()
        } else {
            RichText { blocks }
        }
    }

    /// Текст без оформления: блоки через перевод строки. Для поиска, для
    /// старых версий программы и для вставки в чужие программы.
    pub fn plain_text(&self) -> String {
        self.blocks
            .iter()
            .map(Block::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(Block::is_empty)
            && self.blocks.len() == 1
            && self.blocks[0].kind == Kind::Paragraph
    }

    /// Нет ничего, кроме абзацев без оформления, — такой текст хранится
    /// просто строкой.
    pub fn is_plain(&self) -> bool {
        self.blocks.iter().all(Block::is_plain)
    }

    pub fn end(&self) -> Pos {
        let last = self.blocks.len() - 1;
        Pos::new(last, self.blocks[last].len())
    }

    /// Ближайшее допустимое место: блок есть, смещение не дальше конца и
    /// на границе знака.
    pub fn clamp(&self, pos: Pos) -> Pos {
        let block = pos.block.min(self.blocks.len() - 1);
        let text = self.blocks[block].text();
        let mut offset = pos.offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        Pos::new(block, offset)
    }

    /// Номера пунктов нумерованных списков; у остальных блоков — `None`.
    /// Счёт идёт подряд на своём уровне: пункт другого вида на том же
    /// уровне или абзац начинают его заново, вложенные пункты — нет.
    pub fn numbers(&self) -> Vec<Option<usize>> {
        let mut counters = [0usize; MAX_INDENT as usize + 1];
        self.blocks
            .iter()
            .map(|b| {
                let level = b.indent.min(MAX_INDENT) as usize;
                if !b.kind.is_list() {
                    counters = [0; MAX_INDENT as usize + 1];
                    return None;
                }
                for c in &mut counters[level + 1..] {
                    *c = 0;
                }
                if b.kind == Kind::Numbered {
                    counters[level] += 1;
                    Some(counters[level])
                } else {
                    counters[level] = 0;
                    None
                }
            })
            .collect()
    }

    // ---- Сплошной текст для системного ввода ----------------------------
    //
    // Система ввода Windows видит текст одной строкой в UTF-16, где блоки
    // разделены переводом строки.

    pub fn flat_text(&self) -> String {
        self.plain_text()
    }

    pub fn pos_to_utf16(&self, pos: Pos) -> usize {
        let mut n = 0;
        for (i, b) in self.blocks.iter().enumerate() {
            let text = b.text();
            if i == pos.block {
                return n + text[..pos.offset.min(text.len())].encode_utf16().count();
            }
            n += text.encode_utf16().count() + 1;
        }
        n.saturating_sub(1)
    }

    pub fn utf16_to_pos(&self, utf16: usize) -> Pos {
        let mut left = utf16;
        for (i, b) in self.blocks.iter().enumerate() {
            let text = b.text();
            let len16 = text.encode_utf16().count();
            if left <= len16 || i + 1 == self.blocks.len() {
                let mut offset = 0;
                let mut seen = 0;
                for ch in text.chars() {
                    if seen >= left {
                        break;
                    }
                    seen += ch.len_utf16();
                    offset += ch.len_utf8();
                }
                return Pos::new(i, offset);
            }
            left -= len16 + 1;
        }
        self.end()
    }

    // ---- Правки ----------------------------------------------------------

    /// Удаляет промежуток и возвращает место, где он был. Блок начала
    /// сохраняет свой вид; удалили всё — остаётся пустой абзац.
    fn delete_range(&mut self, start: Pos, end: Pos) -> Pos {
        if start == end {
            return start;
        }
        if start == Pos::default() && end == self.end() {
            *self = RichText::default();
            return Pos::default();
        }
        if start.block == end.block {
            self.blocks[start.block].remove(start.offset, end.offset);
            return start;
        }
        let tail = self.blocks[end.block].split_off(end.offset);
        self.blocks.drain(start.block + 1..=end.block);
        let first = &mut self.blocks[start.block];
        first.split_off(start.offset);
        first.append(tail);
        start
    }

    /// Вставляет текст со своим оформлением. Перевод строки начинает новый
    /// блок того же вида, что и блок курсора: так вставка нескольких строк в
    /// список даёт несколько пунктов.
    fn insert_plain(&mut self, at: Pos, text: &str, marks: Marks) -> Pos {
        let mut pos = at;
        for (i, line) in text.split('\n').enumerate() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if i > 0 {
                pos = self.split_block(pos);
            }
            if !line.is_empty() {
                self.blocks[pos.block].insert(pos.offset, line, marks);
                pos.offset += line.len();
            }
        }
        pos
    }

    /// Вставляет оформленные блоки (из буфера обмена). Первый блок
    /// вливается в блок курсора, последний принимает хвост после курсора.
    /// Каждая новая строка — со своим видом, как у вставленного блока; в
    /// пустую строку и первый ложится со своим видом: вставленный список
    /// остаётся списком.
    fn insert_rich(&mut self, at: Pos, rich: &RichText) -> Pos {
        let mut pos = at;
        for (i, b) in rich.blocks.iter().enumerate() {
            if i > 0 {
                pos = self.split_block(pos);
            }
            let target = &mut self.blocks[pos.block];
            if i > 0 || target.is_empty() {
                target.kind = b.kind;
                target.indent = b.indent;
            }
            let len = b.len();
            let (mut head, tail) = split_spans(
                std::mem::take(&mut self.blocks[pos.block].spans),
                pos.offset,
            );
            head.extend(b.spans.iter().cloned());
            head.extend(tail);
            normalize(&mut head);
            self.blocks[pos.block].spans = head;
            pos.offset += len;
        }
        pos
    }

    /// Разрезает блок в этом месте; возвращает начало второй половины. Вид
    /// нового блока: пункт списка продолжает список (чек-лист — пустым
    /// квадратом), после заголовка и абзаца — абзац.
    fn split_block(&mut self, at: Pos) -> Pos {
        let tail = self.blocks[at.block].split_off(at.offset);
        let cur = &self.blocks[at.block];
        let kind = match cur.kind {
            Kind::Check(_) => Kind::Check(false),
            k if k.is_list() => k,
            _ => Kind::Paragraph,
        };
        let mut next = Block::new(kind);
        next.indent = if kind.is_list() { cur.indent } else { 0 };
        next.spans = tail;
        self.blocks.insert(at.block + 1, next);
        Pos::new(at.block + 1, 0)
    }

    fn block_range(&self, sel: Selection) -> std::ops::RangeInclusive<usize> {
        let (s, e) = (sel.start(), sel.end());
        // Выделение, кончающееся в самом начале блока, этот блок не задевает.
        let last = if e.block > s.block && e.offset == 0 {
            e.block - 1
        } else {
            e.block
        };
        s.block..=last
    }

    /// Куски выделения — для копирования.
    pub fn slice(&self, start: Pos, end: Pos) -> RichText {
        let mut blocks = Vec::new();
        for i in start.block..=end.block {
            let b = &self.blocks[i];
            let a = if i == start.block { start.offset } else { 0 };
            let z = if i == end.block { end.offset } else { b.len() };
            let mut part = Block::new(b.kind);
            part.indent = b.indent;
            part.spans = b.slice(a, z);
            blocks.push(part);
        }
        RichText::from_blocks(blocks)
    }
}

/// Какая правка была последней — чтобы набор букв подряд отменялся одним
/// шагом, а не по букве.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastEdit {
    None,
    /// Набор текста; курсор после него стоял здесь.
    Typing(Pos),
}

/// Правка текста карточки: текст, выделение и отмена в пределах одного
/// сеанса правки.
#[derive(Debug, Clone)]
pub struct TextEditor {
    text: RichText,
    sel: Selection,
    /// Оформление для следующего набора, если его включили без выделения:
    /// нажали Ctrl+B и печатают дальше жирным.
    pending: Option<Marks>,
    undo: Vec<(RichText, Selection)>,
    redo: Vec<(RichText, Selection)>,
    last: LastEdit,
}

impl TextEditor {
    pub fn new(text: RichText) -> TextEditor {
        let text = RichText::from_blocks(text.blocks);
        let end = text.end();
        TextEditor {
            text,
            sel: Selection::caret(end),
            pending: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last: LastEdit::None,
        }
    }

    pub fn text(&self) -> &RichText {
        &self.text
    }

    pub fn selection(&self) -> Selection {
        self.sel
    }

    pub fn caret(&self) -> Pos {
        self.sel.head
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    // ---- Выделение и курсор -----------------------------------------------

    pub fn set_selection(&mut self, sel: Selection) {
        let sel = Selection {
            anchor: self.text.clamp(sel.anchor),
            head: self.text.clamp(sel.head),
        };
        if sel != self.sel {
            self.pending = None;
            self.last = LastEdit::None;
        }
        self.sel = sel;
    }

    /// Курсор сюда; `extend` — продлить выделение (Shift).
    pub fn move_to(&mut self, pos: Pos, extend: bool) {
        let anchor = if extend { self.sel.anchor } else { pos };
        self.set_selection(Selection { anchor, head: pos });
    }

    pub fn select_all(&mut self) {
        self.set_selection(Selection {
            anchor: Pos::default(),
            head: self.text.end(),
        });
    }

    /// Выделяет слово под местом — двойной щелчок.
    pub fn select_word(&mut self, at: Pos) {
        let at = self.text.clamp(at);
        let text = self.text.blocks[at.block].text();
        let mut start = 0;
        for (i, w) in text.split_word_bound_indices() {
            let end = i + w.len();
            if at.offset >= i && at.offset <= end && (at.offset < end || end == text.len()) {
                start = i;
                self.set_selection(Selection {
                    anchor: Pos::new(at.block, start),
                    head: Pos::new(at.block, end),
                });
                return;
            }
        }
        self.set_selection(Selection::caret(Pos::new(at.block, start)));
    }

    /// Соседняя граница знака слева, через границу блока — в конец
    /// предыдущего.
    pub fn left_of(&self, pos: Pos) -> Pos {
        if pos.offset == 0 {
            return if pos.block == 0 {
                pos
            } else {
                Pos::new(pos.block - 1, self.text.blocks[pos.block - 1].len())
            };
        }
        let text = self.text.blocks[pos.block].text();
        let prev = text[..pos.offset]
            .grapheme_indices(true)
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        Pos::new(pos.block, prev)
    }

    pub fn right_of(&self, pos: Pos) -> Pos {
        let b = &self.text.blocks[pos.block];
        if pos.offset >= b.len() {
            return if pos.block + 1 < self.text.blocks.len() {
                Pos::new(pos.block + 1, 0)
            } else {
                pos
            };
        }
        let text = b.text();
        let next = text[pos.offset..]
            .graphemes(true)
            .next()
            .map(|g| pos.offset + g.len())
            .unwrap_or(text.len());
        Pos::new(pos.block, next)
    }

    /// Начало слова слева — Ctrl+←.
    pub fn word_left_of(&self, pos: Pos) -> Pos {
        if pos.offset == 0 {
            return self.left_of(pos);
        }
        let text = self.text.blocks[pos.block].text();
        let chars: Vec<(usize, char)> = text[..pos.offset].char_indices().collect();
        let mut i = chars.len();
        while i > 0 && chars[i - 1].1.is_whitespace() {
            i -= 1;
        }
        if i > 0 {
            let word = chars[i - 1].1.is_alphanumeric();
            while i > 0
                && !chars[i - 1].1.is_whitespace()
                && chars[i - 1].1.is_alphanumeric() == word
            {
                i -= 1;
            }
        }
        Pos::new(pos.block, chars.get(i).map(|c| c.0).unwrap_or(pos.offset))
    }

    /// Конец слова справа — Ctrl+→.
    pub fn word_right_of(&self, pos: Pos) -> Pos {
        let b = &self.text.blocks[pos.block];
        if pos.offset >= b.len() {
            return self.right_of(pos);
        }
        let text = b.text();
        let mut it = text[pos.offset..].char_indices().peekable();
        while it.peek().is_some_and(|(_, c)| c.is_whitespace()) {
            it.next();
        }
        let word = it.peek().map(|(_, c)| c.is_alphanumeric());
        while let Some(&(_, c)) = it.peek() {
            if c.is_whitespace() || Some(c.is_alphanumeric()) != word {
                break;
            }
            it.next();
        }
        let end = it.peek().map(|(i, _)| pos.offset + i).unwrap_or(text.len());
        Pos::new(pos.block, end)
    }

    pub fn block_start(&self, pos: Pos) -> Pos {
        Pos::new(pos.block, 0)
    }

    pub fn block_end(&self, pos: Pos) -> Pos {
        Pos::new(pos.block, self.text.blocks[pos.block].len())
    }

    // ---- Отмена -----------------------------------------------------------

    fn snapshot(&mut self) {
        self.undo.push((self.text.clone(), self.sel));
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Точка отмены перед правкой; набор букв подряд — один шаг.
    fn begin(&mut self, typing: bool) {
        let continues =
            typing && self.sel.is_empty() && self.last == LastEdit::Typing(self.sel.head);
        if !continues {
            self.snapshot();
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some((text, sel)) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push((std::mem::replace(&mut self.text, text), self.sel));
        self.sel = sel;
        self.pending = None;
        self.last = LastEdit::None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((text, sel)) = self.redo.pop() else {
            return false;
        };
        self.undo
            .push((std::mem::replace(&mut self.text, text), self.sel));
        self.sel = sel;
        self.pending = None;
        self.last = LastEdit::None;
        true
    }

    // ---- Набор и удаление --------------------------------------------------

    /// Оформление, которым пойдёт набор: включённое кнопкой или как у
    /// знака перед курсором.
    pub fn typing_marks(&self) -> Marks {
        self.pending.unwrap_or_else(|| {
            let p = self.sel.start();
            self.text.blocks[p.block].marks_at(p.offset)
        })
    }

    /// Набор текста — то, что пришло с клавиатуры или из системы ввода.
    /// Заменяет выделение.
    pub fn type_text(&mut self, text: &str) {
        if text.is_empty() && self.sel.is_empty() {
            return;
        }
        let marks = self.typing_marks();
        let plain_char = !text.contains('\n');
        self.begin(plain_char);
        let start = self.text.delete_range(self.sel.start(), self.sel.end());
        let end = self.text.insert_plain(start, text, marks);
        self.sel = Selection::caret(end);
        // Набранное идёт тем же оформлением, пока курсор не сдвинули.
        if self.pending.is_some() && !plain_char {
            self.pending = None;
        }
        self.last = if plain_char {
            LastEdit::Typing(end)
        } else {
            LastEdit::None
        };
        if text == " " {
            self.markdown_shortcut();
        }
    }

    /// Заменяет промежуток — так система ввода правит набранное (иероглифы,
    /// знаки с надстрочными значками).
    pub fn replace(&mut self, start: Pos, end: Pos, text: &str) {
        let (start, end) = (self.text.clamp(start), self.text.clamp(end));
        self.sel = Selection {
            anchor: start.min(end),
            head: start.max(end),
        };
        self.type_text(text);
    }

    /// «- », «1. », «[] », «# » в начале абзаца превращают его в пункт
    /// списка, пункт чек-листа или заголовок. Отмена возвращает набранное.
    fn markdown_shortcut(&mut self) {
        let caret = self.sel.head;
        let b = &self.text.blocks[caret.block];
        if b.kind != Kind::Paragraph || caret.offset == 0 {
            return;
        }
        let text = b.text();
        let prefix = &text[..caret.offset - 1];
        let kind = match prefix {
            "-" | "*" | "•" => Kind::Bullet,
            "[]" | "[ ]" => Kind::Check(false),
            "[x]" | "[X]" | "[х]" | "[Х]" => Kind::Check(true),
            "#" => Kind::Heading(1),
            "##" => Kind::Heading(2),
            "###" => Kind::Heading(3),
            p if (p.ends_with('.') || p.ends_with(')'))
                && p.len() > 1
                && p[..p.len() - 1].chars().all(|c| c.is_ascii_digit()) =>
            {
                Kind::Numbered
            }
            _ => return,
        };
        self.snapshot();
        let b = &mut self.text.blocks[caret.block];
        b.remove(0, caret.offset);
        b.kind = kind;
        self.sel = Selection::caret(Pos::new(caret.block, 0));
        self.last = LastEdit::None;
    }

    /// Enter: новый блок. Пустой пункт списка вместо этого выходит из
    /// списка: сначала на уровень вверх, потом в обычный абзац.
    pub fn enter(&mut self) {
        self.begin(false);
        let marks = self.typing_marks();
        let at = self.text.delete_range(self.sel.start(), self.sel.end());
        let b = &mut self.text.blocks[at.block];
        if b.kind.is_list() && b.is_empty() {
            if b.indent > 0 {
                b.indent -= 1;
            } else {
                b.kind = Kind::Paragraph;
            }
            self.sel = Selection::caret(at);
        } else if at.offset == 0 && !b.is_empty() {
            // Enter в начале строки — пустая строка над ней, текст остаётся
            // своим блоком со своим видом.
            let mut above = Block::new(if b.kind.is_list() {
                match b.kind {
                    Kind::Check(_) => Kind::Check(false),
                    k => k,
                }
            } else {
                Kind::Paragraph
            });
            above.indent = if b.kind.is_list() { b.indent } else { 0 };
            self.text.blocks.insert(at.block, above);
            self.sel = Selection::caret(Pos::new(at.block + 1, 0));
        } else {
            let next = self.text.split_block(at);
            self.sel = Selection::caret(next);
        }
        // Жирный набор продолжается на новой строке.
        self.pending = (!marks.is_plain()).then_some(marks);
        self.last = LastEdit::None;
    }

    /// Backspace. В начале пункта списка или заголовка сначала убирает
    /// отступ, потом вид блока и только потом склеивает с предыдущим.
    pub fn backspace(&mut self) {
        if !self.sel.is_empty() {
            return self.delete_selection();
        }
        let at = self.sel.head;
        if at.offset == 0 {
            let b = &mut self.text.blocks[at.block];
            if b.kind.is_list() && b.indent > 0 {
                self.snapshot();
                self.text.blocks[at.block].indent -= 1;
                self.last = LastEdit::None;
                return;
            }
            if b.kind != Kind::Paragraph {
                self.snapshot();
                let b = &mut self.text.blocks[at.block];
                b.kind = Kind::Paragraph;
                b.indent = 0;
                self.last = LastEdit::None;
                return;
            }
            if at.block == 0 {
                return;
            }
        }
        let from = self.left_of(at);
        self.snapshot();
        let p = self.text.delete_range(from, at);
        self.sel = Selection::caret(p);
        self.last = LastEdit::None;
    }

    /// Delete — знак справа; в конце блока склеивает со следующим.
    pub fn delete_forward(&mut self) {
        if !self.sel.is_empty() {
            return self.delete_selection();
        }
        let at = self.sel.head;
        let to = self.right_of(at);
        if to == at {
            return;
        }
        self.snapshot();
        let p = self.text.delete_range(at, to);
        self.sel = Selection::caret(p);
        self.last = LastEdit::None;
    }

    /// Ctrl+Backspace — слово слева.
    pub fn delete_word_backward(&mut self) {
        if !self.sel.is_empty() {
            return self.delete_selection();
        }
        let at = self.sel.head;
        let from = self.word_left_of(at);
        if from == at {
            return self.backspace();
        }
        self.snapshot();
        let p = self.text.delete_range(from, at);
        self.sel = Selection::caret(p);
        self.last = LastEdit::None;
    }

    /// Ctrl+Delete — слово справа.
    pub fn delete_word_forward(&mut self) {
        if !self.sel.is_empty() {
            return self.delete_selection();
        }
        let at = self.sel.head;
        let to = self.word_right_of(at);
        if to == at {
            return;
        }
        self.snapshot();
        let p = self.text.delete_range(at, to);
        self.sel = Selection::caret(p);
        self.last = LastEdit::None;
    }

    pub fn delete_selection(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.snapshot();
        let p = self.text.delete_range(self.sel.start(), self.sel.end());
        self.sel = Selection::caret(p);
        self.last = LastEdit::None;
    }

    // ---- Буфер обмена -----------------------------------------------------

    /// Выделенное с оформлением.
    pub fn selected(&self) -> Option<RichText> {
        (!self.sel.is_empty()).then(|| self.text.slice(self.sel.start(), self.sel.end()))
    }

    /// Вставка из буфера с оформлением.
    pub fn paste_rich(&mut self, rich: &RichText) {
        self.begin(false);
        let at = self.text.delete_range(self.sel.start(), self.sel.end());
        let end = self.text.insert_rich(at, rich);
        self.sel = Selection::caret(end);
        self.last = LastEdit::None;
    }

    /// Вставка простого текста: строки — блоки того же вида.
    pub fn paste_plain(&mut self, text: &str) {
        self.begin(false);
        let marks = self.typing_marks();
        let at = self.text.delete_range(self.sel.start(), self.sel.end());
        let end = self.text.insert_plain(at, text, marks);
        self.sel = Selection::caret(end);
        self.last = LastEdit::None;
    }

    // ---- Оформление и вид блоков -------------------------------------------

    /// Есть ли оформление — для подсветки кнопки. Без выделения — то, чем
    /// пойдёт набор; с выделением — есть ли оно на всём выделенном.
    pub fn has_mark(&self, mark: Mark) -> bool {
        if self.sel.is_empty() {
            return self.typing_marks().has(mark);
        }
        self.all_have(mark).unwrap_or(false)
    }

    fn all_have(&self, mark: Mark) -> Option<bool> {
        let (s, e) = (self.sel.start(), self.sel.end());
        let mut any = None;
        for i in s.block..=e.block {
            let b = &self.text.blocks[i];
            let a = if i == s.block { s.offset } else { 0 };
            let z = if i == e.block { e.offset } else { b.len() };
            match b.all_have(a, z, mark) {
                Some(false) => return Some(false),
                Some(true) => any = Some(true),
                None => {}
            }
        }
        any
    }

    /// Ctrl+B и подобные. С выделением — на всё выделенное (снимается, если
    /// уже было везде); без — для дальнейшего набора.
    pub fn toggle_mark(&mut self, mark: Mark) {
        if self.sel.is_empty() {
            let marks = self.typing_marks();
            self.pending = Some(marks.with(mark, !marks.has(mark)));
            return;
        }
        let on = !self.all_have(mark).unwrap_or(false);
        self.snapshot();
        let (s, e) = (self.sel.start(), self.sel.end());
        for i in s.block..=e.block {
            let b = &mut self.text.blocks[i];
            let a = if i == s.block { s.offset } else { 0 };
            let z = if i == e.block { e.offset } else { b.len() };
            b.map_marks(a, z, |m| m.with(mark, on));
        }
        self.last = LastEdit::None;
    }

    /// Вид блоков под курсором или выделением, если он у всех один.
    pub fn block_kind(&self) -> Option<Kind> {
        let mut kinds = self
            .text
            .block_range(self.sel)
            .map(|i| self.text.blocks[i].kind);
        let first = kinds.next()?;
        kinds.all(|k| k.same_family(first)).then_some(first)
    }

    /// Кнопки «список», «чек-лист», «заголовок»: включают вид для блоков
    /// под курсором, а если он уже у всех — возвращают абзацы.
    pub fn toggle_kind(&mut self, kind: Kind) {
        let range = self.text.block_range(self.sel);
        let all = range
            .clone()
            .all(|i| self.text.blocks[i].kind.same_family(kind));
        self.snapshot();
        for i in range {
            let b = &mut self.text.blocks[i];
            if all {
                b.kind = Kind::Paragraph;
                b.indent = 0;
            } else if !b.kind.same_family(kind) {
                b.kind = kind;
                if !kind.is_list() {
                    b.indent = 0;
                }
            }
        }
        self.last = LastEdit::None;
    }

    /// Tab и Shift+Tab: вложенность пунктов списка. Абзацы не сдвигаются.
    /// `false`, если сдвигать нечего.
    pub fn indent(&mut self, deeper: bool) -> bool {
        let range = self.text.block_range(self.sel);
        let can = range.clone().any(|i| {
            let b = &self.text.blocks[i];
            b.kind.is_list()
                && if deeper {
                    b.indent < MAX_INDENT
                } else {
                    b.indent > 0
                }
        });
        if !can {
            return false;
        }
        self.snapshot();
        for i in range {
            let b = &mut self.text.blocks[i];
            if b.kind.is_list() {
                b.indent = if deeper {
                    (b.indent + 1).min(MAX_INDENT)
                } else {
                    b.indent.saturating_sub(1)
                };
            }
        }
        self.last = LastEdit::None;
        true
    }

    /// Отмечает пункт чек-листа или снимает отметку.
    pub fn toggle_check(&mut self, block: usize) -> bool {
        let Some(Kind::Check(done)) = self.text.blocks.get(block).map(|b| b.kind) else {
            return false;
        };
        self.snapshot();
        self.text.blocks[block].kind = Kind::Check(!done);
        self.last = LastEdit::None;
        true
    }
}

/// Отмечает пункт чек-листа вне правки — щелчком по квадрату на карточке.
pub fn toggle_check(text: &mut RichText, block: usize) -> bool {
    match text.blocks.get_mut(block) {
        Some(b) => match b.kind {
            Kind::Check(done) => {
                b.kind = Kind::Check(!done);
                true
            }
            _ => false,
        },
        None => false,
    }
}

// ---- Запись в файл -------------------------------------------------------
//
// Блок без оформления пишется коротко: `{"type": "check", "checked": true,
// "text": "молоко"}`. С оформлением — кусками: `"spans": [{"text": "Купить",
// "bold": true}, {"text": " молоко"}]`.

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &u8) -> bool {
    *n == 0
}

#[derive(Serialize, Deserialize)]
struct SpanJson {
    text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    bold: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    underline: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    strike: bool,
}

#[derive(Serialize, Deserialize)]
struct BlockJson {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    level: u8,
    #[serde(default, skip_serializing_if = "is_zero")]
    indent: u8,
    #[serde(default, skip_serializing_if = "is_false")]
    checked: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    spans: Vec<SpanJson>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl From<&Block> for BlockJson {
    fn from(b: &Block) -> Self {
        let (kind, level, checked) = match b.kind {
            Kind::Paragraph => ("paragraph", 0, false),
            Kind::Heading(l) => ("heading", if l > 1 { l } else { 0 }, false),
            Kind::Bullet => ("bullet", 0, false),
            Kind::Numbered => ("numbered", 0, false),
            Kind::Check(c) => ("check", 0, c),
        };
        let plain = b.spans.iter().all(|s| s.marks.is_plain());
        BlockJson {
            kind: kind.into(),
            level,
            indent: if b.kind.is_list() { b.indent } else { 0 },
            checked,
            text: if plain { b.text() } else { String::new() },
            spans: if plain {
                Vec::new()
            } else {
                b.spans
                    .iter()
                    .map(|s| SpanJson {
                        text: s.text.clone(),
                        bold: s.marks.bold,
                        italic: s.marks.italic,
                        underline: s.marks.underline,
                        strike: s.marks.strike,
                    })
                    .collect()
            },
            extra: b.extra.clone(),
        }
    }
}

impl From<BlockJson> for Block {
    fn from(j: BlockJson) -> Self {
        // Вид, которого эта версия не знает, показывается абзацем: текст
        // важнее вида. Файл с ним всё равно новее и только читается.
        let kind = match j.kind.as_str() {
            "heading" => Kind::Heading(j.level.clamp(1, 3)),
            "bullet" => Kind::Bullet,
            "numbered" => Kind::Numbered,
            "check" => Kind::Check(j.checked),
            _ => Kind::Paragraph,
        };
        let mut spans: Vec<Span> = if j.spans.is_empty() {
            vec![Span::plain(j.text)]
        } else {
            j.spans
                .into_iter()
                .map(|s| Span {
                    text: s.text,
                    marks: Marks {
                        bold: s.bold,
                        italic: s.italic,
                        underline: s.underline,
                        strike: s.strike,
                    },
                })
                .collect()
        };
        // Переводы строк внутри блока не бывают: строка — это блок.
        for s in &mut spans {
            if s.text.contains(['\n', '\r']) {
                s.text = s.text.replace(['\n', '\r'], " ");
            }
        }
        normalize(&mut spans);
        Block {
            kind,
            indent: if kind.is_list() {
                j.indent.min(MAX_INDENT)
            } else {
                0
            },
            spans,
            extra: j.extra,
        }
    }
}

impl Serialize for Block {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        BlockJson::from(self).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Block {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        BlockJson::deserialize(d).map(Block::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> Marks {
        Marks::default().with(Mark::Bold, true)
    }

    fn texts(t: &RichText) -> Vec<String> {
        t.blocks.iter().map(Block::text).collect()
    }

    fn kinds(t: &RichText) -> Vec<Kind> {
        t.blocks.iter().map(|b| b.kind).collect()
    }

    fn typed(s: &str) -> TextEditor {
        let mut e = TextEditor::new(RichText::default());
        for ch in s.chars() {
            if ch == '\n' {
                e.enter();
            } else {
                e.type_text(&ch.to_string());
            }
        }
        e
    }

    #[test]
    fn spans_split_merge_and_stay_normalized() {
        let mut b = Block::paragraph("купить молоко");
        b.map_marks(0, "купить".len(), |m| m.with(Mark::Bold, true));
        assert_eq!(b.spans.len(), 2);
        assert!(b.spans[0].marks.bold && !b.spans[1].marks.bold);
        // Сняли жирный — куски снова слились.
        b.map_marks(0, b.len(), |m| m.with(Mark::Bold, false));
        assert_eq!(b.spans, vec![Span::plain("купить молоко")]);
        b.remove(0, b.len());
        assert!(b.spans.is_empty(), "пустых кусков не остаётся");
    }

    #[test]
    fn typing_continues_the_marks_before_the_caret() {
        let mut b = Block::new(Kind::Paragraph);
        b.insert(0, "жир", bold());
        assert_eq!(b.marks_at(b.len()), bold());
        assert_eq!(b.marks_at(0), bold(), "в начале — как у первого знака");
        b.insert(b.len(), " обычный", Marks::default());
        assert_eq!(b.marks_at(b.len()), Marks::default());
    }

    #[test]
    fn enter_splits_and_lists_continue() {
        let mut e = typed("[] молоко\nхлеб");
        assert_eq!(texts(e.text()), ["молоко", "хлеб"]);
        assert_eq!(kinds(e.text()), [Kind::Check(false); 2]);
        // Отмеченный пункт продолжается пустым квадратом.
        e.toggle_check(1);
        e.enter();
        assert_eq!(e.text().blocks[2].kind, Kind::Check(false));
        // Enter в пустом пункте выходит из списка.
        e.enter();
        assert_eq!(
            kinds(e.text()),
            [Kind::Check(false), Kind::Check(true), Kind::Paragraph]
        );
        e.type_text("x");
        assert_eq!(e.text().blocks[2].kind, Kind::Paragraph);
    }

    #[test]
    fn enter_after_heading_gives_paragraph_and_at_start_keeps_the_heading() {
        let mut e = typed("# План\nшаг");
        assert_eq!(kinds(e.text()), [Kind::Heading(1), Kind::Paragraph]);
        e.move_to(Pos::new(0, 0), false);
        e.enter();
        assert_eq!(texts(e.text()), ["", "План", "шаг"]);
        assert_eq!(
            e.text().blocks[1].kind,
            Kind::Heading(1),
            "заголовок не стал абзацем"
        );
        assert_eq!(e.caret(), Pos::new(1, 0));
    }

    #[test]
    fn backspace_at_line_start_unindents_then_unlists_then_merges() {
        let mut e = typed("- a\nb");
        e.indent(true);
        assert_eq!(e.text().blocks[1].indent, 1);
        e.move_to(Pos::new(1, 0), false);
        e.backspace();
        assert_eq!(e.text().blocks[1].indent, 0);
        e.backspace();
        assert_eq!(e.text().blocks[1].kind, Kind::Paragraph);
        e.backspace();
        assert_eq!(texts(e.text()), ["ab"]);
        assert_eq!(e.caret(), Pos::new(0, 1));
        assert_eq!(
            e.text().blocks[0].kind,
            Kind::Bullet,
            "вид остаётся у первого"
        );
    }

    #[test]
    fn delete_joins_with_the_next_block() {
        let mut e = typed("раз\nдва");
        e.move_to(Pos::new(0, "раз".len()), false);
        e.delete_forward();
        assert_eq!(texts(e.text()), ["раздва"]);
    }

    #[test]
    fn graphemes_are_never_cut() {
        // Флаг — два знака Юникода, «й» — буква с отдельным значком.
        let mut e = typed("а🇷🇺й\u{0306}");
        e.backspace();
        assert_eq!(e.text().plain_text(), "а🇷🇺");
        e.backspace();
        assert_eq!(e.text().plain_text(), "а");
        let e = typed("я🇷🇺");
        let start = Pos::new(0, "я".len());
        assert_eq!(e.right_of(start), Pos::new(0, "я🇷🇺".len()));
    }

    #[test]
    fn markdown_shortcuts_and_undo_returns_the_typed_text() {
        for (input, kind) in [
            ("- ", Kind::Bullet),
            ("* ", Kind::Bullet),
            ("1. ", Kind::Numbered),
            ("12) ", Kind::Numbered),
            ("[] ", Kind::Check(false)),
            ("[ ] ", Kind::Check(false)),
            ("[x] ", Kind::Check(true)),
            ("[х] ", Kind::Check(true)),
            ("# ", Kind::Heading(1)),
            ("## ", Kind::Heading(2)),
            ("### ", Kind::Heading(3)),
        ] {
            let mut e = typed(input);
            assert_eq!(kinds(e.text()), [kind], "{input:?}");
            assert_eq!(e.text().plain_text(), "", "{input:?}");
            e.undo();
            assert_eq!(e.text().plain_text(), input, "отмена вернула набранное");
            assert_eq!(kinds(e.text()), [Kind::Paragraph]);
        }
        // Не в начале строки и не в абзаце — просто текст.
        assert_eq!(typed("а - ").text().plain_text(), "а - ");
        assert_eq!(kinds(typed("- - ").text()), [Kind::Bullet]);
        assert_eq!(typed("- - ").text().plain_text(), "- ");
        assert_eq!(kinds(typed("1x. ").text()), [Kind::Paragraph]);
    }

    #[test]
    fn toggle_mark_on_selection_and_for_further_typing() {
        let mut e = typed("купить молоко");
        e.set_selection(Selection {
            anchor: Pos::new(0, 0),
            head: Pos::new(0, "купить".len()),
        });
        e.toggle_mark(Mark::Bold);
        assert!(e.has_mark(Mark::Bold));
        assert_eq!(e.text().blocks[0].spans[0].text, "купить");
        assert!(e.text().blocks[0].spans[0].marks.bold);
        // Ещё раз — снимается.
        e.toggle_mark(Mark::Bold);
        assert!(e.text().is_plain());
        // Без выделения: включили курсив и печатаем им.
        e.move_to(e.text().end(), false);
        e.toggle_mark(Mark::Italic);
        assert!(e.has_mark(Mark::Italic));
        e.type_text("!");
        e.type_text("!");
        let last = e.text().blocks[0].spans.last().unwrap();
        assert_eq!(last.text, "!!");
        assert!(last.marks.italic);
        // Сдвинули курсор — включённое без выделения забыто.
        e.move_to(Pos::new(0, 0), false);
        assert!(!e.has_mark(Mark::Italic));
    }

    #[test]
    fn marks_across_blocks_apply_to_every_part() {
        let mut e = typed("раз\nдва\nтри");
        e.set_selection(Selection {
            anchor: Pos::new(0, 2),
            head: Pos::new(2, 2),
        });
        e.toggle_mark(Mark::Strike);
        let t = e.text();
        assert!(!t.blocks[0].spans[0].marks.strike && t.blocks[0].spans[1].marks.strike);
        assert!(t.blocks[1].spans[0].marks.strike && t.blocks[1].spans.len() == 1);
        assert!(t.blocks[2].spans[0].marks.strike && !t.blocks[2].spans[1].marks.strike);
    }

    #[test]
    fn toggle_kind_for_selected_blocks_and_back() {
        let mut e = typed("раз\nдва\nтри");
        // Смещения — в байтах: русская буква занимает два.
        e.set_selection(Selection {
            anchor: Pos::new(0, "р".len()),
            head: Pos::new(1, "д".len()),
        });
        e.toggle_kind(Kind::Check(false));
        assert_eq!(
            kinds(e.text()),
            [Kind::Check(false), Kind::Check(false), Kind::Paragraph]
        );
        assert_eq!(e.block_kind(), Some(Kind::Check(false)));
        e.toggle_kind(Kind::Check(false));
        assert_eq!(kinds(e.text()), [Kind::Paragraph; 3]);
        // Выделение до самого начала третьего блока его не задевает.
        e.set_selection(Selection {
            anchor: Pos::new(1, 0),
            head: Pos::new(2, 0),
        });
        e.toggle_kind(Kind::Bullet);
        assert_eq!(
            kinds(e.text()),
            [Kind::Paragraph, Kind::Bullet, Kind::Paragraph]
        );
    }

    #[test]
    fn indent_only_moves_list_items_and_stops_at_the_limit() {
        let mut e = typed("абзац");
        assert!(!e.indent(true), "абзац не сдвигается");
        let mut e = typed("- пункт");
        for _ in 0..10 {
            e.indent(true);
        }
        assert_eq!(e.text().blocks[0].indent, MAX_INDENT);
        assert!(!e.indent(true));
        e.indent(false);
        assert_eq!(e.text().blocks[0].indent, MAX_INDENT - 1);
    }

    #[test]
    fn numbering_counts_per_level_and_restarts() {
        let mut t = RichText::from_blocks(vec![
            Block::new(Kind::Numbered).with_text("a"),
            Block::new(Kind::Numbered).with_text("b"),
            {
                let mut b = Block::new(Kind::Numbered).with_text("b1");
                b.indent = 1;
                b
            },
            Block::new(Kind::Numbered).with_text("c"),
            Block::paragraph("перерыв"),
            Block::new(Kind::Numbered).with_text("снова"),
        ]);
        assert_eq!(
            t.numbers(),
            [Some(1), Some(2), Some(1), Some(3), None, Some(1)]
        );
        t.blocks[1].kind = Kind::Bullet;
        assert_eq!(
            t.numbers()[3],
            Some(1),
            "пункт другого вида начинает счёт заново"
        );
    }

    #[test]
    fn undo_groups_typing_into_one_step_per_run() {
        let mut e = typed("abc");
        e.enter();
        e.type_text("d");
        e.undo();
        assert_eq!(e.text().plain_text(), "abc\n");
        e.undo();
        assert_eq!(e.text().plain_text(), "abc");
        e.undo();
        assert_eq!(e.text().plain_text(), "", "три буквы — один шаг");
        assert!(!e.undo());
        e.redo();
        assert_eq!(e.text().plain_text(), "abc");
    }

    #[test]
    fn select_all_and_delete_leaves_one_empty_paragraph() {
        let mut e = typed("# Заголовок\n- пункт");
        e.select_all();
        e.backspace();
        assert_eq!(e.text(), &RichText::default());
    }

    #[test]
    fn paste_plain_lines_become_items_of_the_current_list() {
        let mut e = typed("- ");
        e.paste_plain("хлеб\r\nсыр\nчай");
        assert_eq!(texts(e.text()), ["хлеб", "сыр", "чай"]);
        assert_eq!(kinds(e.text()), [Kind::Bullet; 3]);
        assert_eq!(e.caret(), Pos::new(2, "чай".len()));
    }

    #[test]
    fn copy_and_paste_keep_marks_and_kinds() {
        let mut e = typed("[] молоко\nхлеб");
        e.set_selection(Selection {
            anchor: Pos::new(0, 0),
            head: Pos::new(0, "молоко".len()),
        });
        e.toggle_mark(Mark::Bold);
        e.select_all();
        let clip = e.selected().unwrap();
        let mut other = TextEditor::new(RichText::default());
        other.paste_rich(&clip);
        assert_eq!(other.text(), e.text());
        // В середину строки: первый блок вливается, последний берёт хвост.
        let mut mid = typed("начало конец");
        mid.move_to(Pos::new(0, "начало ".len()), false);
        mid.paste_rich(&clip);
        assert_eq!(texts(mid.text()), ["начало молоко", "хлебконец"]);
        assert_eq!(kinds(mid.text()), [Kind::Paragraph, Kind::Check(false)]);
    }

    #[test]
    fn utf16_positions_round_trip() {
        let t = RichText::from_plain("а😀б\nв");
        let positions = [
            Pos::new(0, 0),
            Pos::new(0, "а".len()),
            Pos::new(0, "а😀".len()),
            Pos::new(0, "а😀б".len()),
            Pos::new(1, 0),
            Pos::new(1, "в".len()),
        ];
        let utf16: Vec<usize> = positions.iter().map(|&p| t.pos_to_utf16(p)).collect();
        assert_eq!(utf16, [0, 1, 3, 4, 5, 6], "смайлик — две единицы UTF-16");
        for (p, u) in positions.iter().zip(utf16) {
            assert_eq!(t.utf16_to_pos(u), *p);
        }
        assert_eq!(t.utf16_to_pos(100), Pos::new(1, "в".len()));
        assert_eq!(t.flat_text().encode_utf16().count(), 6);
    }

    #[test]
    fn ime_replace_corrects_what_was_typed() {
        let mut e = typed("ab");
        e.replace(Pos::new(0, 1), Pos::new(0, 2), "é");
        assert_eq!(e.text().plain_text(), "aé");
        assert_eq!(e.caret(), Pos::new(0, "aé".len()));
    }

    #[test]
    fn word_moves_and_word_deletes() {
        let mut e = typed("купить свежий хлеб");
        assert_eq!(
            e.word_left_of(e.caret()),
            Pos::new(0, "купить свежий ".len())
        );
        e.delete_word_backward();
        assert_eq!(e.text().plain_text(), "купить свежий ");
        e.delete_word_backward();
        assert_eq!(e.text().plain_text(), "купить ");
        e.move_to(Pos::new(0, 0), false);
        assert_eq!(e.word_right_of(Pos::new(0, 0)), Pos::new(0, "купить".len()));
        e.delete_word_forward();
        assert_eq!(e.text().plain_text(), " ");
    }

    #[test]
    fn select_word_under_double_click() {
        let mut e = typed("купить свежий хлеб");
        e.select_word(Pos::new(0, "купить св".len()));
        let s = e.selection();
        assert_eq!(
            (s.start(), s.end()),
            (
                Pos::new(0, "купить ".len()),
                Pos::new(0, "купить свежий".len())
            )
        );
    }

    #[test]
    fn json_is_short_without_marks_and_round_trips() {
        let mut e = typed("[x] молоко\nсыр");
        e.set_selection(Selection {
            anchor: Pos::new(1, 0),
            head: Pos::new(1, "сыр".len()),
        });
        e.toggle_mark(Mark::Underline);
        e.indent(true);
        let blocks = &e.text().blocks;
        let json = serde_json::to_string(blocks).unwrap();
        assert_eq!(
            json,
            r#"[{"type":"check","checked":true,"text":"молоко"},{"type":"check","indent":1,"spans":[{"text":"сыр","underline":true}]}]"#
        );
        let back: Vec<Block> = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, blocks);
    }

    #[test]
    fn json_from_others_is_cleaned_up() {
        let json = r#"[{"type":"heading","level":9,"text":"a\nb"},
                      {"type":"quote","text":"q","color":"red"},
                      {"type":"paragraph","indent":3,"spans":[{"text":"x"},{"text":""},{"text":"y"}]}]"#;
        let blocks: Vec<Block> = serde_json::from_str(json).unwrap();
        assert_eq!(blocks[0].kind, Kind::Heading(3));
        assert_eq!(blocks[0].text(), "a b", "строка внутри блока не рвётся");
        assert_eq!(blocks[1].kind, Kind::Paragraph);
        assert_eq!(blocks[1].extra["color"], "red", "неизвестное сохранилось");
        assert_eq!(blocks[2].indent, 0, "у абзаца нет отступа");
        assert_eq!(blocks[2].spans, vec![Span::plain("xy")]);
    }

    #[test]
    fn plain_text_detection() {
        assert!(RichText::from_plain("раз\nдва").is_plain());
        assert!(!typed("- пункт").text().is_plain());
        assert!(RichText::default().is_empty());
        assert!(!typed("x").text().is_empty());
        assert!(
            !typed("- ").text().is_empty(),
            "пустой пункт списка — не пусто"
        );
    }

    #[test]
    fn standalone_toggle_check_for_clicks_outside_editing() {
        let mut t = RichText::from_blocks(vec![
            Block::new(Kind::Check(false)).with_text("молоко"),
            Block::paragraph("абзац"),
        ]);
        assert!(toggle_check(&mut t, 0));
        assert_eq!(t.blocks[0].kind, Kind::Check(true));
        assert!(!toggle_check(&mut t, 1), "не пункт чек-листа");
        assert!(!toggle_check(&mut t, 7));
    }
}

//! Текстовый формат файлов: документ `.bdoc` и холст `.bcanvas`.
//!
//! Это JSON с отступами, по одному полю на строку. Такой файл читается
//! глазами без программы и переживает синхронизацию через облачную папку:
//! он целиком текстовый, без двоичных вставок, а запись идёт атомарно.
//!
//! Поля, которых эта версия программы не знает, при чтении сохраняются и
//! при записи возвращаются на место. Поэтому старая версия на другом
//! компьютере не сотрёт то, что добавила новая.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::rich::{Block as RichBlock, RichText};

pub const DOC_EXT: &str = "bdoc";
pub const CANVAS_EXT: &str = "bcanvas";
const DOC_FORMAT: &str = "bolh-doc";
const CANVAS_FORMAT: &str = "bolh-canvas";

/// Версия формата документа, которую пишет эта программа. Файл с большей
/// версией открывается только для чтения: перезапись потеряла бы новые
/// данные.
pub const DOC_VERSION: u32 = 1;
/// То же для холста. Версия 2 — карточки с оформленным текстом. Холст
/// версии 1 читается как есть и записывается уже версией 2: программа 0.2,
/// увидев 2, откроет его только для чтения и не сотрёт оформление, которого
/// не понимает.
pub const CANVAS_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    Document,
    Canvas,
}

impl FileKind {
    pub fn ext(self) -> &'static str {
        match self {
            FileKind::Document => DOC_EXT,
            FileKind::Canvas => CANVAS_EXT,
        }
    }

    pub fn from_path(path: &Path) -> Option<FileKind> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            DOC_EXT => Some(FileKind::Document),
            CANVAS_EXT => Some(FileKind::Canvas),
            _ => None,
        }
    }
}

/// Абзац или другой блок документа. На этапе 1 программа сама создаёт
/// только абзацы; остальные виды блоков появятся с редактором.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Block {
    pub fn paragraph(text: impl Into<String>) -> Self {
        Block {
            kind: "paragraph".into(),
            text: text.into(),
            extra: Map::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub format: String,
    pub version: u32,
    pub id: Uuid,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Document {
    pub fn new() -> Self {
        Document {
            format: DOC_FORMAT.into(),
            version: DOC_VERSION,
            id: Uuid::new_v4(),
            blocks: Vec::new(),
            extra: Map::new(),
        }
    }

    /// Текст документа одной строкой с переносами: так его показывает
    /// заглушка редактора на этапе 1.
    pub fn plain_text(&self) -> String {
        self.blocks
            .iter()
            .filter(|b| b.kind == "paragraph")
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Заменяет текст из заглушки редактора. Каждая строка — абзац. Блоки,
    /// которые заглушка не умеет показывать, не трогаются: их нельзя
    /// потерять из-за того, что редактор пока простой.
    pub fn set_plain_text(&mut self, text: &str) {
        let others: Vec<Block> = self
            .blocks
            .drain(..)
            .filter(|b| b.kind != "paragraph")
            .collect();
        self.blocks = if text.is_empty() {
            Vec::new()
        } else {
            text.split('\n')
                .map(|l| Block::paragraph(l.strip_suffix('\r').unwrap_or(l)))
                .collect()
        };
        self.blocks.extend(others);
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub format: String,
    #[serde(serialize_with = "write_canvas_version")]
    pub version: u32,
    pub id: Uuid,
    /// Объекты холста по порядку слоёв: последний — сверху.
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Холст всегда пишется не ниже текущей версии: в нём могут быть карточки
/// с оформлением.
fn write_canvas_version<S: Serializer>(v: &u32, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_u32((*v).max(CANVAS_VERSION))
}

impl Canvas {
    pub fn new() -> Self {
        Canvas {
            format: CANVAS_FORMAT.into(),
            version: CANVAS_VERSION,
            id: Uuid::new_v4(),
            nodes: Vec::new(),
            edges: Vec::new(),
            extra: Map::new(),
        }
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

/// Объект на холсте. Поля координат названы как в формате JSON Canvas,
/// чтобы экспорт для Obsidian (раздел 8) был простым.
///
/// Текст в файле лежит в двух видах. `text` — простой, без оформления: его
/// видно глазами, по нему ищут, его показывает старая версия программы.
/// `blocks` — с оформлением и списками; пишется, только если оформление
/// есть, и при чтении главнее `text`.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    /// Вид объекта: `card` — карточка. Неизвестные виды от новых версий
    /// программы хранятся как есть.
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Текст объекта с оформлением.
    pub rich: RichText,
    pub extra: Map<String, Value>,
}

pub const CARD: &str = "card";

impl Node {
    pub fn card(id: impl Into<String>, x: f64, y: f64, width: f64, height: f64) -> Self {
        Node {
            id: id.into(),
            kind: CARD.into(),
            x,
            y,
            width,
            height,
            rich: RichText::default(),
            extra: Map::new(),
        }
    }

    /// Текст без оформления.
    pub fn plain_text(&self) -> String {
        self.rich.plain_text()
    }

    pub fn set_plain_text(&mut self, text: &str) {
        self.rich = RichText::from_plain(text);
    }

    pub fn has_text(&self) -> bool {
        !self.rich.is_empty()
    }
}

#[derive(Serialize)]
struct NodeOut<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[serde(skip_serializing_if = "String::is_empty")]
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocks: Option<&'a [RichBlock]>,
    #[serde(flatten)]
    extra: &'a Map<String, Value>,
}

#[derive(Deserialize)]
struct NodeIn {
    #[serde(default)]
    id: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    width: f64,
    #[serde(default)]
    height: f64,
    #[serde(default)]
    text: String,
    #[serde(default)]
    blocks: Option<Vec<RichBlock>>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        NodeOut {
            id: &self.id,
            kind: &self.kind,
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            text: self.rich.plain_text(),
            blocks: (!self.rich.is_plain()).then_some(&self.rich.blocks[..]),
            extra: &self.extra,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let n = NodeIn::deserialize(d)?;
        let rich = match n.blocks {
            Some(blocks) if !blocks.is_empty() => RichText::from_blocks(blocks),
            _ => RichText::from_plain(&n.text),
        };
        Ok(Node {
            id: n.id,
            kind: n.kind,
            x: n.x,
            y: n.y,
            width: n.width,
            height: n.height,
            rich,
            extra: n.extra,
        })
    }
}

/// Содержимое открытого файла любого вида.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Document(Document),
    Canvas(Canvas),
}

impl Content {
    pub fn new(kind: FileKind) -> Self {
        match kind {
            FileKind::Document => Content::Document(Document::new()),
            FileKind::Canvas => Content::Canvas(Canvas::new()),
        }
    }

    pub fn kind(&self) -> FileKind {
        match self {
            Content::Document(_) => FileKind::Document,
            Content::Canvas(_) => FileKind::Canvas,
        }
    }

    pub fn id(&self) -> Uuid {
        match self {
            Content::Document(d) => d.id,
            Content::Canvas(c) => c.id,
        }
    }

    pub fn set_id(&mut self, id: Uuid) {
        match self {
            Content::Document(d) => d.id = id,
            Content::Canvas(c) => c.id = id,
        }
    }

    pub fn version(&self) -> u32 {
        match self {
            Content::Document(d) => d.version,
            Content::Canvas(c) => c.version,
        }
    }

    /// Можно ли сохранять: файл более новой версии формата только читается.
    pub fn is_writable(&self) -> bool {
        match self {
            Content::Document(d) => d.version <= DOC_VERSION,
            Content::Canvas(c) => c.version <= CANVAS_VERSION,
        }
    }

    /// Байты для записи на диск: JSON с отступами, переводы строк `\n`,
    /// перевод строки в конце.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = match self {
            Content::Document(d) => serde_json::to_vec_pretty(d),
            Content::Canvas(c) => serde_json::to_vec_pretty(c),
        }
        .expect("JSON из своих структур всегда пишется");
        out.push(b'\n');
        out
    }

    /// Разбирает файл. Вид файла задаётся расширением; содержимое обязано
    /// ему соответствовать.
    pub fn parse(kind: FileKind, bytes: &[u8], path: &Path) -> Result<Content> {
        let corrupt = |reason: String| Error::Corrupt {
            path: path.to_path_buf(),
            reason,
        };
        // Блокнот Windows мог сохранить файл с меткой BOM — её пропускаем.
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        let head: Value = serde_json::from_slice(bytes).map_err(|e| corrupt(e.to_string()))?;
        let format = head.get("format").and_then(Value::as_str).unwrap_or("");
        let expected = match kind {
            FileKind::Document => DOC_FORMAT,
            FileKind::Canvas => CANVAS_FORMAT,
        };
        if format != expected {
            return Err(corrupt(format!(
                "ожидался формат {expected}, а не {format:?}"
            )));
        }
        let version = head
            .get("version")
            .and_then(Value::as_u64)
            .ok_or_else(|| corrupt("нет номера версии".into()))?;
        let version = u32::try_from(version).map_err(|_| corrupt("номер версии".into()))?;
        let content = match kind {
            FileKind::Document => {
                Content::Document(serde_json::from_value(head).map_err(|e| corrupt(e.to_string()))?)
            }
            FileKind::Canvas => {
                Content::Canvas(serde_json::from_value(head).map_err(|e| corrupt(e.to_string()))?)
            }
        };
        // Разобрать удалось и у более новой версии, но писать её нельзя —
        // это видно по is_writable.
        debug_assert_eq!(content.version(), version);
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> &'static Path {
        Path::new("x")
    }

    #[test]
    fn document_round_trip() {
        let mut d = Document::new();
        d.set_plain_text("Первая строка\nВторая «в кавычках» и \"так\"\n\nпосле пустой");
        let c = Content::Document(d);
        let bytes = c.to_bytes();
        let back = Content::parse(FileKind::Document, &bytes, p()).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.to_bytes(), bytes, "повторная запись даёт те же байты");
    }

    #[test]
    fn canvas_round_trip_keeps_unknown_objects() {
        let text = r#"{
  "format": "bolh-canvas",
  "version": 2,
  "id": "1b4e28ba-2fa1-11d2-883f-0016d3cca427",
  "nodes": [{"id": "a", "x": 10, "y": -5, "kind": "card"}],
  "edges": [],
  "viewport": {"zoom": 1.5}
}
"#;
        let c = Content::parse(FileKind::Canvas, text.as_bytes(), p()).unwrap();
        let again = Content::parse(FileKind::Canvas, &c.to_bytes(), p()).unwrap();
        assert_eq!(c, again);
        let s = String::from_utf8(c.to_bytes()).unwrap();
        assert!(s.contains("\"viewport\""), "неизвестное поле сохранилось");
        assert!(s.contains("\"kind\": \"card\""));
        let Content::Canvas(cv) = &c else { panic!() };
        assert_eq!((cv.nodes[0].x, cv.nodes[0].y), (10.0, -5.0));
    }

    #[test]
    fn canvas_of_version_1_is_read_and_written_as_version_2() {
        let text = r#"{"format":"bolh-canvas","version":1,"id":"1b4e28ba-2fa1-11d2-883f-0016d3cca427",
            "nodes":[{"id":"a","type":"card","x":0,"y":0,"width":240,"height":140,"text":"раз\nдва"}]}"#;
        let c = Content::parse(FileKind::Canvas, text.as_bytes(), p()).unwrap();
        assert!(c.is_writable(), "холст от версии 0.2 правится");
        let Content::Canvas(cv) = &c else { panic!() };
        assert_eq!(cv.nodes[0].plain_text(), "раз\nдва");
        let again = Content::parse(FileKind::Canvas, &c.to_bytes(), p()).unwrap();
        assert_eq!(again.version(), CANVAS_VERSION);
        // Холст новее этой программы — только для чтения.
        let newer = text.replace("\"version\":1", "\"version\":3");
        let c = Content::parse(FileKind::Canvas, newer.as_bytes(), p()).unwrap();
        assert!(!c.is_writable());
    }

    #[test]
    fn card_text_is_plain_in_the_file_unless_it_has_formatting() {
        use crate::rich::{Block as B, Kind, Mark, Marks, Span};
        let mut n = Node::card("a", 0.0, 0.0, 240.0, 140.0);
        n.set_plain_text("раз\nдва");
        let json = serde_json::to_value(&n).unwrap();
        assert_eq!(json["text"], "раз\nдва");
        assert!(json.get("blocks").is_none(), "без оформления — только text");

        n.rich.blocks = vec![
            B::new(Kind::Check(true)).with_text("молоко"),
            B {
                spans: vec![Span {
                    text: "важно".into(),
                    marks: Marks::default().with(Mark::Bold, true),
                }],
                ..B::new(Kind::Paragraph)
            },
        ];
        let json = serde_json::to_value(&n).unwrap();
        assert_eq!(json["text"], "молоко\nважно", "простой текст остаётся");
        assert_eq!(json["blocks"][0]["type"], "check");
        assert_eq!(json["blocks"][1]["spans"][0]["bold"], true);
        let back: Node = serde_json::from_value(json).unwrap();
        assert_eq!(back, n);

        // При чтении оформленный текст главнее простого.
        let mut json = serde_json::to_value(&n).unwrap();
        json["text"] = "устарело".into();
        let back: Node = serde_json::from_value(json).unwrap();
        assert_eq!(back.plain_text(), "молоко\nважно");

        // Пустая карточка пишется без текста вовсе.
        let empty = serde_json::to_value(Node::card("b", 0.0, 0.0, 1.0, 1.0)).unwrap();
        assert!(empty.get("text").is_none() && empty.get("blocks").is_none());
    }

    #[test]
    fn file_is_human_readable_text() {
        let mut d = Document::new();
        d.set_plain_text("Привет");
        let s = String::from_utf8(Content::Document(d).to_bytes()).unwrap();
        assert!(
            s.contains("\"text\": \"Привет\""),
            "кириллица без экранирования: {s}"
        );
        assert!(s.ends_with("}\n"));
        assert!(!s.contains('\r'));
        assert!(s.lines().count() > 3, "по полю на строку");
    }

    #[test]
    fn unknown_block_and_fields_survive_plain_text_edit() {
        let text = r#"{"format":"bolh-doc","version":1,"id":"1b4e28ba-2fa1-11d2-883f-0016d3cca427",
            "blocks":[{"type":"paragraph","text":"a"},{"type":"table","rows":3}],"theme":"x"}"#;
        let Content::Document(mut d) =
            Content::parse(FileKind::Document, text.as_bytes(), p()).unwrap()
        else {
            panic!()
        };
        d.set_plain_text("b\r\nc");
        assert_eq!(d.plain_text(), "b\nc");
        // Повторный проход через заглушку ничего не добавляет.
        let before = d.clone();
        d.set_plain_text(&before.plain_text());
        assert_eq!(d, before);
        assert!(
            d.blocks
                .iter()
                .any(|b| b.kind == "table" && b.extra["rows"] == 3)
        );
        assert_eq!(d.extra["theme"], "x");
    }

    #[test]
    fn empty_text_means_no_blocks() {
        let mut d = Document::new();
        d.set_plain_text("");
        assert!(d.blocks.is_empty());
        d.set_plain_text("\n");
        assert_eq!(d.plain_text(), "\n");
    }

    #[test]
    fn rejects_wrong_kind_and_garbage() {
        let c = Content::new(FileKind::Canvas).to_bytes();
        assert!(matches!(
            Content::parse(FileKind::Document, &c, p()),
            Err(Error::Corrupt { .. })
        ));
        assert!(Content::parse(FileKind::Document, b"not json", p()).is_err());
        assert!(Content::parse(FileKind::Document, b"{\"format\":\"bolh-doc\"}", p()).is_err());
    }

    #[test]
    fn newer_version_is_read_only() {
        let text =
            r#"{"format":"bolh-doc","version":7,"id":"1b4e28ba-2fa1-11d2-883f-0016d3cca427"}"#;
        let c = Content::parse(FileKind::Document, text.as_bytes(), p()).unwrap();
        assert!(!c.is_writable());
        assert!(Content::new(FileKind::Document).is_writable());
    }

    #[test]
    fn utf8_bom_is_accepted() {
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend(Content::new(FileKind::Document).to_bytes());
        assert!(Content::parse(FileKind::Document, &bytes, p()).is_ok());
    }

    #[test]
    fn kind_by_extension_ignores_case() {
        assert_eq!(
            FileKind::from_path(Path::new("a/B.BDOC")),
            Some(FileKind::Document)
        );
        assert_eq!(
            FileKind::from_path(Path::new("x.bcanvas")),
            Some(FileKind::Canvas)
        );
        assert_eq!(FileKind::from_path(Path::new("x.md")), None);
        assert_eq!(FileKind::from_path(Path::new("bdoc")), None);
    }
}

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

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::error::{Error, Result};

pub const DOC_EXT: &str = "bdoc";
pub const CANVAS_EXT: &str = "bcanvas";
const DOC_FORMAT: &str = "bolh-doc";
const CANVAS_FORMAT: &str = "bolh-canvas";

/// Версия формата, которую пишет эта программа. Файл с большей версией
/// открывается только для чтения: перезапись потеряла бы новые данные.
pub const FORMAT_VERSION: u32 = 1;

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
            version: FORMAT_VERSION,
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
    pub version: u32,
    pub id: Uuid,
    /// Объекты холста. Их устройство определит этап 2, пока они хранятся
    /// как есть и не теряются при сохранении.
    #[serde(default)]
    pub nodes: Vec<Value>,
    #[serde(default)]
    pub edges: Vec<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Canvas {
    pub fn new() -> Self {
        Canvas {
            format: CANVAS_FORMAT.into(),
            version: FORMAT_VERSION,
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
        self.version() <= FORMAT_VERSION
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
        if version > FORMAT_VERSION {
            // Разобрать удалось, но писать нельзя — это видно по is_writable.
            debug_assert!(!content.is_writable());
        }
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
  "version": 1,
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

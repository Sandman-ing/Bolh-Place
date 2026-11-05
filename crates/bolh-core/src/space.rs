//! Пространство — обычная папка на диске (раздел 3 спецификации).
//!
//! ```text
//! Работа/
//!   .bolh/space.json     признак пространства и версия его устройства
//!   .bolh/history/       история версий
//!   .bolh/trash/         корзина
//!   attachments/         картинки и вложения
//!   План.bdoc, Схема.bcanvas, папки…
//! ```

use std::fs;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result, io};
use crate::format::{Content, FileKind};
use crate::fsutil::{TEMP_PREFIX, free_name, read_optional, validate_name, write_atomic};
use crate::history::{History, Snapshot, history_dir};
use crate::trash::{Trash, TrashEntry};

pub const META_DIR: &str = ".bolh";
pub const ATTACHMENTS_DIR: &str = "attachments";
const SPACE_FILE: &str = "space.json";

#[derive(Debug, Serialize, Deserialize)]
struct SpaceFile {
    format: String,
    version: u32,
}

/// Время и часовой пояс снаружи: ядро не смотрит на часы само, чтобы
/// тесты были точными, а «день» в истории совпадал с днём человека.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    pub now: DateTime<Utc>,
    pub tz: FixedOffset,
}

impl Clock {
    pub fn system() -> Self {
        let local = chrono::Local::now();
        Clock {
            now: local.with_timezone(&Utc),
            tz: *local.offset(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    Folder(Vec<Entry>),
    File(FileKind),
}

/// Элемент дерева файлов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Путь относительно корня пространства.
    pub rel: PathBuf,
    /// Как показывать: для файлов — без расширения.
    pub name: String,
    pub kind: EntryKind,
}

/// Открытый файл и то, каким он был на диске, когда мы его видели в
/// последний раз. По этому «базовому» состоянию программа замечает, что
/// файл поменяли на другом компьютере.
#[derive(Debug, Clone)]
pub struct OpenFile {
    pub rel: PathBuf,
    pub content: Content,
    /// Байты на диске — по ним видно чужую правку.
    base: Vec<u8>,
    /// Те же байты, разобранные. Сравнение по смыслу, а не по байтам: файл,
    /// поправленный руками с другими отступами, не считается изменённым.
    base_content: Content,
}

impl OpenFile {
    fn new(rel: PathBuf, content: Content, base: Vec<u8>) -> Self {
        OpenFile {
            rel,
            base_content: content.clone(),
            content,
            base,
        }
    }

    /// Есть ли несохранённые правки.
    pub fn is_dirty(&self) -> bool {
        self.content != self.base_content
    }

    fn set_base(&mut self, base: Vec<u8>) {
        self.base = base;
        self.base_content = self.content.clone();
    }
}

/// Что случилось с файлом на диске, пока он был открыт.
#[derive(Debug)]
pub enum External {
    Unchanged,
    /// Файл изменили снаружи. Новое содержимое уже разобрано.
    Changed(Box<Content>, Vec<u8>),
    /// Файл изменили снаружи, но прочитать его не вышло — например,
    /// облако ещё докачивает.
    Unreadable(Error),
    Deleted,
}

#[derive(Debug, PartialEq)]
pub enum SaveOutcome {
    Saved,
    /// Файл одновременно правили здесь и на другом компьютере. Их версия
    /// осталась на месте, наша записана рядом, в `copy`. `OpenFile`
    /// теперь указывает на копию.
    Conflict {
        original: PathBuf,
        copy: PathBuf,
    },
}

pub struct Space {
    root: PathBuf,
}

impl Space {
    /// Создаёт пространство в папке (саму папку — тоже, если её нет).
    /// Обычная папка с файлами становится пространством, ничего не теряя.
    pub fn create(root: &Path) -> Result<Space> {
        let meta = root.join(META_DIR);
        fs::create_dir_all(&meta).map_err(io(&meta))?;
        let marker = meta.join(SPACE_FILE);
        if !marker.exists() {
            let f = SpaceFile {
                format: "bolh-space".into(),
                version: 1,
            };
            write_atomic(&marker, &serde_json::to_vec_pretty(&f).expect("JSON"))?;
        }
        let att = root.join(ATTACHMENTS_DIR);
        fs::create_dir_all(&att).map_err(io(&att))?;
        Space::open(root)
    }

    pub fn open(root: &Path) -> Result<Space> {
        if !root.join(META_DIR).join(SPACE_FILE).is_file() {
            return Err(Error::NotASpace(root.to_path_buf()));
        }
        Ok(Space {
            root: root.to_path_buf(),
        })
    }

    pub fn is_space(root: &Path) -> bool {
        root.join(META_DIR).join(SPACE_FILE).is_file()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Имя пространства — имя его папки.
    pub fn name(&self) -> String {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.to_string_lossy().into_owned())
    }

    pub fn history(&self) -> History {
        History::new(history_dir(&self.root.join(META_DIR)))
    }

    pub fn trash(&self) -> Trash {
        Trash::new(self.root.join(META_DIR).join("trash"))
    }

    /// Полный путь по относительному. Путь из интерфейса или из файла не
    /// должен выводить за пределы пространства и в служебную папку.
    pub fn resolve(&self, rel: &Path) -> Result<PathBuf> {
        let ok = rel
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
        let first = rel.components().next().map(|c| c.as_os_str().to_owned());
        if !ok || first.as_deref() == Some(META_DIR.as_ref()) {
            return Err(Error::OutsideSpace(rel.to_path_buf()));
        }
        Ok(self.root.join(rel))
    }

    /// Дерево документов, холстов и папок. Служебные и скрытые файлы,
    /// `attachments` и временные файлы сохранения не показываются.
    /// Сначала папки, потом файлы, по алфавиту без учёта регистра.
    pub fn tree(&self) -> Result<Vec<Entry>> {
        self.scan(Path::new(""))
    }

    fn scan(&self, rel: &Path) -> Result<Vec<Entry>> {
        let dir = self.root.join(rel);
        let rd = fs::read_dir(&dir).map_err(io(&dir))?;
        let mut out = Vec::new();
        for e in rd {
            let e = e.map_err(io(&dir))?;
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.starts_with(TEMP_PREFIX) {
                continue;
            }
            if rel.as_os_str().is_empty() && name.eq_ignore_ascii_case(ATTACHMENTS_DIR) {
                continue;
            }
            let child = rel.join(&name);
            let ft = e.file_type().map_err(io(e.path()))?;
            if ft.is_dir() {
                let children = self.scan(&child)?;
                out.push(Entry {
                    rel: child,
                    name,
                    kind: EntryKind::Folder(children),
                });
            } else if let Some(kind) = FileKind::from_path(&child) {
                let shown = Path::new(&name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or(name);
                out.push(Entry {
                    rel: child,
                    name: shown,
                    kind: EntryKind::File(kind),
                });
            }
        }
        out.sort_by(|a, b| {
            let folder = |e: &Entry| !matches!(e.kind, EntryKind::Folder(_));
            folder(a)
                .cmp(&folder(b))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(out)
    }

    fn parent_dir(&self, parent: &Path) -> Result<PathBuf> {
        let dir = self.resolve(parent)?;
        if !dir.is_dir() {
            return Err(Error::NotFound(parent.to_path_buf()));
        }
        Ok(dir)
    }

    /// Создаёт пустой документ или холст в папке `parent`. Если имя занято,
    /// добавляет номер. Возвращает путь нового файла.
    pub fn create_file(
        &self,
        parent: &Path,
        name: &str,
        kind: FileKind,
        clock: Clock,
    ) -> Result<PathBuf> {
        if !validate_name(name) {
            return Err(Error::InvalidName(name.into()));
        }
        let dir = self.parent_dir(parent)?;
        let path = free_name(&dir, name, Some(kind.ext()));
        let content = Content::new(kind);
        let bytes = content.to_bytes();
        write_atomic(&path, &bytes)?;
        self.history()
            .record(content.id(), kind, &bytes, clock.now, clock.tz, true)?;
        Ok(parent.join(path.file_name().expect("имя есть")))
    }

    pub fn create_folder(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        if !validate_name(name) {
            return Err(Error::InvalidName(name.into()));
        }
        let dir = self.parent_dir(parent)?;
        let path = free_name(&dir, name, None);
        fs::create_dir(&path).map_err(io(&path))?;
        Ok(parent.join(path.file_name().expect("имя есть")))
    }

    /// Переименовывает файл или папку. Для файлов `new_name` — без
    /// расширения: вид файла при переименовании не меняется.
    pub fn rename(&self, rel: &Path, new_name: &str) -> Result<PathBuf> {
        if !validate_name(new_name) {
            return Err(Error::InvalidName(new_name.into()));
        }
        let from = self.resolve(rel)?;
        if rel.as_os_str().is_empty() || !from.exists() {
            return Err(Error::NotFound(rel.to_path_buf()));
        }
        let file_name = match FileKind::from_path(rel) {
            Some(kind) if from.is_file() => format!("{new_name}.{}", kind.ext()),
            _ => new_name.to_string(),
        };
        let parent = rel.parent().unwrap_or(Path::new(""));
        let to_rel = parent.join(&file_name);
        let to = self.root.join(&to_rel);
        // Имена сравниваются без учёта регистра: в Windows и в облаке `b` и
        // `B` — один файл. Смена регистра у самого себя разрешена.
        let own = from
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let wanted = file_name.to_lowercase();
        if wanted != own {
            let dir = self.root.join(parent);
            let taken = fs::read_dir(&dir)
                .map_err(io(&dir))?
                .filter_map(|e| e.ok())
                .any(|e| e.file_name().to_string_lossy().to_lowercase() == wanted);
            if taken {
                return Err(Error::AlreadyExists(to_rel));
            }
        }
        fs::rename(&from, &to).map_err(io(&from))?;
        Ok(to_rel)
    }

    /// Удаляет в корзину.
    pub fn delete(&self, rel: &Path, clock: Clock) -> Result<TrashEntry> {
        let full = self.resolve(rel)?;
        if rel.as_os_str().is_empty() || !full.exists() {
            return Err(Error::NotFound(rel.to_path_buf()));
        }
        self.trash().put(&self.root, rel, clock.now)
    }

    pub fn restore(&self, entry: &TrashEntry) -> Result<PathBuf> {
        self.trash().restore(&self.root, entry)
    }

    /// Удаляет из корзины навсегда — вместе с историей версий.
    pub fn purge(&self, entry: &TrashEntry) -> Result<()> {
        let history = self.history();
        for id in ids_in(&entry.stored) {
            history.remove_all(id)?;
        }
        self.trash().purge(entry)
    }

    /// Кладёт копию файла в `attachments` и возвращает путь к ней
    /// относительно корня. Оригинал не трогается.
    pub fn add_attachment(&self, src: &Path) -> Result<PathBuf> {
        let dir = self.root.join(ATTACHMENTS_DIR);
        fs::create_dir_all(&dir).map_err(io(&dir))?;
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error::InvalidName(src.to_string_lossy().into_owned()))?;
        let p = Path::new(name);
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
        let target = free_name(&dir, stem, p.extension().and_then(|e| e.to_str()));
        let bytes = fs::read(src).map_err(io(src))?;
        write_atomic(&target, &bytes)?;
        Ok(Path::new(ATTACHMENTS_DIR).join(target.file_name().expect("имя есть")))
    }

    pub fn open_file(&self, rel: &Path) -> Result<OpenFile> {
        let path = self.resolve(rel)?;
        let kind = FileKind::from_path(rel).ok_or_else(|| Error::Corrupt {
            path: path.clone(),
            reason: "неизвестный вид файла".into(),
        })?;
        let bytes = fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(rel.to_path_buf()),
            _ => io(&path)(e),
        })?;
        let content = Content::parse(kind, &bytes, &path)?;
        Ok(OpenFile::new(rel.to_path_buf(), content, bytes))
    }

    /// Смотрит, не изменили ли файл снаружи. Ничего не меняет: решение —
    /// перечитать или оставить правки — за интерфейсом.
    pub fn check_external(&self, file: &OpenFile) -> External {
        let path = match self.resolve(&file.rel) {
            Ok(p) => p,
            Err(e) => return External::Unreadable(e),
        };
        match read_optional(&path) {
            Ok(None) => External::Deleted,
            Ok(Some(b)) if b == file.base => External::Unchanged,
            Ok(Some(b)) => match Content::parse(file.content.kind(), &b, &path) {
                Ok(c) => External::Changed(Box::new(c), b),
                Err(e) => External::Unreadable(e),
            },
            Err(e) => External::Unreadable(e),
        }
    }

    /// Принимает версию с диска, пришедшую снаружи (из `External::Changed`).
    pub fn accept_external(&self, file: &mut OpenFile, content: Content, bytes: Vec<u8>) {
        file.content = content;
        file.set_base(bytes);
    }

    /// Сохраняет открытый файл, не затирая чужих правок.
    ///
    /// Если файл на диске такой, каким мы его видели, — просто запись. Если
    /// его изменили снаружи, наша версия пишется рядом под именем
    /// «Имя (`conflict_label` дата)», а их версия остаётся. Если файл
    /// удалили снаружи, а у нас правки, — файл создаётся заново: правки
    /// дороже удаления.
    pub fn save(
        &self,
        file: &mut OpenFile,
        conflict_label: &str,
        clock: Clock,
    ) -> Result<SaveOutcome> {
        let path = self.resolve(&file.rel)?;
        if !file.content.is_writable() {
            return Err(Error::TooNew {
                path,
                found: file.content.version(),
            });
        }
        let bytes = file.content.to_bytes();
        let disk = read_optional(&path)?;
        let ours_or_same = match &disk {
            None => true,
            Some(d) => *d == file.base || *d == bytes,
        };
        if ours_or_same {
            if disk.as_deref() != Some(&bytes[..]) {
                write_atomic(&path, &bytes)?;
            }
            // Сначала база, потом снимок: если снимок не запишется, файл
            // всё равно сохранён, и своя же запись не примется за чужую.
            file.set_base(bytes.clone());
            self.snapshot(&file.content, &bytes, clock, false)?;
            return Ok(SaveOutcome::Saved);
        }

        // Расхождение. Наша копия получает новый id: у двух разных файлов
        // не может быть общей истории.
        let original = file.rel.clone();
        let dir = path.parent().expect("у файла есть папка");
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stamp = clock.now.with_timezone(&clock.tz).format("%Y-%m-%d %H-%M");
        let copy_stem = format!("{stem} ({conflict_label} {stamp})");
        let copy_path = free_name(dir, &copy_stem, Some(file.content.kind().ext()));
        file.content.set_id(Uuid::new_v4());
        let copy_bytes = file.content.to_bytes();
        write_atomic(&copy_path, &copy_bytes)?;
        let copy = original
            .parent()
            .unwrap_or(Path::new(""))
            .join(copy_path.file_name().expect("имя есть"));
        file.rel = copy.clone();
        file.set_base(copy_bytes.clone());
        self.snapshot(&file.content, &copy_bytes, clock, true)?;
        Ok(SaveOutcome::Conflict { original, copy })
    }

    fn snapshot(&self, content: &Content, bytes: &[u8], clock: Clock, force: bool) -> Result<()> {
        self.history()
            .record(
                content.id(),
                content.kind(),
                bytes,
                clock.now,
                clock.tz,
                force,
            )
            .map(|_| ())
    }

    /// Откатывает файл к снимку. Текущее состояние сначала тоже попадает в
    /// историю, поэтому откат можно отменить. Возвращает то, что записано.
    pub fn restore_snapshot(
        &self,
        file: &mut OpenFile,
        snap: &Snapshot,
        conflict_label: &str,
        clock: Clock,
    ) -> Result<SaveOutcome> {
        let bytes = self.history().read(snap)?;
        let path = self.resolve(&file.rel)?;
        let mut restored = Content::parse(file.content.kind(), &bytes, &path)?;
        // Снимок мог быть сделан до смены id (например, у копии при
        // расхождении) — файл сохраняет свой нынешний id.
        restored.set_id(file.content.id());
        self.snapshot(&file.content, &file.content.to_bytes(), clock, true)?;
        file.content = restored;
        self.save(file, conflict_label, clock)
    }
}

/// id всех документов и холстов внутри файла или папки.
fn ids_in(path: &Path) -> Vec<Uuid> {
    let mut out = Vec::new();
    if path.is_dir() {
        if let Ok(rd) = fs::read_dir(path) {
            for e in rd.flatten() {
                out.extend(ids_in(&e.path()));
            }
        }
    } else if let Some(kind) = FileKind::from_path(path)
        && let Ok(bytes) = fs::read(path)
        && let Ok(c) = Content::parse(kind, &bytes, path)
    {
        out.push(c.id());
    }
    out
}

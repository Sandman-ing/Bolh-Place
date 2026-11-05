//! Корзина: удалённые документы, холсты и папки, которые можно вернуть.
//!
//! Лежит в `.bolh/trash/<номер>/`: рядом с удалённым файлом — `entry.json`
//! с исходным путём и временем удаления. Корзина внутри пространства, так
//! что удалённое на одном компьютере можно вернуть на другом.

use std::fs;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result, io};
use crate::fsutil::{free_name, write_atomic};

const ENTRY_FILE: &str = "entry.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EntryMeta {
    /// Путь внутри пространства через `/`, чтобы файл читался одинаково
    /// на любом компьютере.
    original: String,
    deleted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrashEntry {
    pub key: Uuid,
    /// Где лежало, относительно корня пространства.
    pub original: PathBuf,
    pub deleted_at: DateTime<Utc>,
    pub is_dir: bool,
    /// Где лежит сейчас, в корзине.
    pub stored: PathBuf,
}

impl TrashEntry {
    pub fn name(&self) -> String {
        self.original
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

pub struct Trash {
    dir: PathBuf,
}

pub(crate) fn rel_to_slash(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

impl Trash {
    pub fn new(dir: PathBuf) -> Self {
        Trash { dir }
    }

    /// Переносит файл или папку `root/rel` в корзину.
    pub(crate) fn put(&self, root: &Path, rel: &Path, now: DateTime<Utc>) -> Result<TrashEntry> {
        let src = root.join(rel);
        let meta = fs::metadata(&src).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(rel.to_path_buf()),
            _ => io(&src)(e),
        })?;
        let name = rel
            .file_name()
            .ok_or_else(|| Error::InvalidName(rel_to_slash(rel)))?;
        let key = Uuid::new_v4();
        let slot = self.dir.join(key.to_string());
        fs::create_dir_all(&slot).map_err(io(&slot))?;
        let info = EntryMeta {
            original: rel_to_slash(rel),
            deleted_at: now,
        };
        let json = serde_json::to_vec_pretty(&info).expect("JSON из своей структуры");
        write_atomic(&slot.join(ENTRY_FILE), &json)?;
        let stored = slot.join(name);
        if let Err(e) = fs::rename(&src, &stored) {
            let _ = fs::remove_dir_all(&slot);
            return Err(io(&src)(e));
        }
        Ok(TrashEntry {
            key,
            original: rel.to_path_buf(),
            deleted_at: now,
            is_dir: meta.is_dir(),
            stored,
        })
    }

    /// Всё содержимое корзины, сначала удалённое последним. Испорченные
    /// записи пропускаются: из-за одной плохой корзина не должна пропасть.
    pub fn list(&self) -> Result<Vec<TrashEntry>> {
        let rd = match fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io(&self.dir)(e)),
        };
        let mut out: Vec<TrashEntry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| read_entry(&e.path()))
            .collect();
        out.sort_by_key(|e| std::cmp::Reverse(e.deleted_at));
        Ok(out)
    }

    /// Возвращает на старое место. Если там уже что-то есть — рядом, под
    /// свободным именем. Пропавшие папки по пути создаются заново.
    /// Возвращает новый путь относительно корня.
    pub(crate) fn restore(&self, root: &Path, entry: &TrashEntry) -> Result<PathBuf> {
        let parent_rel = entry.original.parent().unwrap_or(Path::new(""));
        let parent = root.join(parent_rel);
        fs::create_dir_all(&parent).map_err(io(&parent))?;
        let name = entry.name();
        let path = Path::new(&name);
        let (stem, ext) = if entry.is_dir {
            (name.as_str(), None)
        } else {
            (
                path.file_stem().and_then(|s| s.to_str()).unwrap_or(&name),
                path.extension().and_then(|s| s.to_str()),
            )
        };
        let target = free_name(&parent, stem, ext);
        fs::rename(&entry.stored, &target).map_err(io(&entry.stored))?;
        let slot = self.dir.join(entry.key.to_string());
        let _ = fs::remove_dir_all(&slot);
        Ok(parent_rel.join(target.file_name().expect("имя есть")))
    }

    /// Удаляет навсегда.
    pub(crate) fn purge(&self, entry: &TrashEntry) -> Result<()> {
        let slot = self.dir.join(entry.key.to_string());
        fs::remove_dir_all(&slot).map_err(io(&slot))
    }
}

fn read_entry(slot: &Path) -> Option<TrashEntry> {
    let key: Uuid = slot.file_name()?.to_str()?.parse().ok()?;
    let bytes = fs::read(slot.join(ENTRY_FILE)).ok()?;
    let meta: EntryMeta = serde_json::from_slice(&bytes).ok()?;
    let original: PathBuf = meta.original.split('/').collect();
    // Запись из облака могла быть подделана или испорчена: путь обязан
    // оставаться внутри пространства.
    if original.as_os_str().is_empty()
        || !original
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return None;
    }
    let stored = slot.join(original.file_name()?);
    let is_dir = fs::metadata(&stored).ok()?.is_dir();
    Some(TrashEntry {
        key,
        original,
        deleted_at: meta.deleted_at,
        is_dir,
        stored,
    })
}

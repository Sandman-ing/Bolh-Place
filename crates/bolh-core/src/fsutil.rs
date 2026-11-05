//! Работа с диском, общая для всего ядра.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Result, io};

/// Префикс временных файлов. Такие файлы пропускаются при обходе
/// пространства: облачный клиент может успеть их увидеть, но программа — нет.
pub const TEMP_PREFIX: &str = ".~bolh-";

/// Записывает файл целиком или не трогает его вовсе: сначала во временный
/// файл рядом, потом переименование. Облачная папка и сбой питания не
/// увидят половину файла.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(io(dir))?;
    let tmp = dir.join(format!("{TEMP_PREFIX}{}", uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut f = fs::File::create(&tmp).map_err(io(&tmp))?;
        f.write_all(bytes).map_err(io(&tmp))?;
        f.sync_all().map_err(io(&tmp))?;
        drop(f);
        replace(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Переименование с заменой. В Windows файл бывает ненадолго занят
/// антивирусом или облачным клиентом, поэтому несколько попыток.
fn replace(from: &Path, to: &Path) -> Result<()> {
    let mut last = None;
    for attempt in 0..10 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(50 * (attempt + 1)));
            }
            Err(e) => return Err(io(to)(e)),
        }
    }
    Err(io(to)(last.expect("была хотя бы одна попытка")))
}

/// Читает файл; если его нет — `None`, а не ошибка.
pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(path)(e)),
    }
}

/// Свободное имя в папке: `имя.ext`, затем `имя 2.ext`, `имя 3.ext`…
/// Сравнение без учёта регистра, потому что в Windows `A.bdoc` и `a.bdoc` —
/// один и тот же файл.
pub fn free_name(dir: &Path, stem: &str, ext: Option<&str>) -> PathBuf {
    let make = |n: u32| {
        let base = if n == 1 {
            stem.to_string()
        } else {
            format!("{stem} {n}")
        };
        match ext {
            Some(e) if !e.is_empty() => format!("{base}.{e}"),
            _ => base,
        }
    };
    let taken: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    (1..)
        .map(make)
        .find(|n| !taken.contains(&n.to_lowercase()))
        .map(|n| dir.join(n))
        .expect("бесконечный ряд имён")
}

/// Проверяет имя файла или папки, которое ввёл человек. Запрещено то, что
/// Windows не примет, и то, что выведет за пределы папки.
pub fn validate_name(name: &str) -> bool {
    const RESERVED: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed != name || name.ends_with('.') || name.starts_with('.') {
        return false;
    }
    if name.chars().count() > 200 {
        return false;
    }
    if name
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c < ' ')
    {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").to_lowercase();
    !RESERVED.contains(&stem.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1, "временные файлы должны исчезнуть");
    }

    #[test]
    fn free_name_skips_taken_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("План.bdoc"), "").unwrap();
        fs::write(dir.path().join("план 2.BDOC"), "").unwrap();
        let p = free_name(dir.path(), "План", Some("bdoc"));
        assert_eq!(p.file_name().unwrap(), "План 3.bdoc");
    }

    #[test]
    fn names_windows_rejects() {
        for bad in [
            "", " a", "a ", "a.", ".hidden", "a/b", "a\\b", "a:b", "a*", "con", "CON.bdoc", "lpt1",
        ] {
            assert!(!validate_name(bad), "{bad:?} должно быть запрещено");
        }
        for good in ["План на год", "a.b", "Работа 2026", "console"] {
            assert!(validate_name(good), "{good:?} должно быть разрешено");
        }
    }
}

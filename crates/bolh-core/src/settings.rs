//! Настройки программы: список пространств, тема, язык. Лежат в папке
//! пользователя Windows, а не в пространстве: у каждого компьютера свои.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::fsutil::{read_optional, write_atomic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// Тёмная по умолчанию — как у телефонного Bolh.
    #[default]
    Dark,
    Light,
    /// Как в Windows.
    System,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Папки пространств в порядке добавления.
    pub spaces: Vec<PathBuf>,
    /// Пространство, открытое в прошлый раз.
    pub active: Option<PathBuf>,
    pub theme: ThemeChoice,
    /// Код языка. Пусто — по языку Windows.
    pub language: Option<String>,
}

impl Settings {
    /// Читает настройки. Нет файла или он испорчен — настройки по
    /// умолчанию: из-за плохого файла программа должна открыться, а не
    /// упасть. Испорченный файл откладывается рядом, чтобы его можно было
    /// посмотреть.
    pub fn load(path: &Path) -> Settings {
        let bytes = match read_optional(path) {
            Ok(Some(b)) => b,
            _ => return Settings::default(),
        };
        match serde_json::from_slice::<Settings>(&bytes) {
            Ok(mut s) => {
                s.dedupe();
                s
            }
            Err(_) => {
                let _ = std::fs::rename(path, path.with_extension("broken.json"));
                Settings::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut bytes = serde_json::to_vec_pretty(self).expect("JSON из своей структуры");
        bytes.push(b'\n');
        write_atomic(path, &bytes)
    }

    /// Добавляет пространство и делает его активным. Повторно та же папка
    /// не добавляется.
    pub fn add_space(&mut self, root: &Path) {
        if !self.spaces.iter().any(|p| same_path(p, root)) {
            self.spaces.push(root.to_path_buf());
        }
        self.active = Some(root.to_path_buf());
    }

    /// Убирает пространство из списка. Папка на диске не трогается.
    pub fn forget_space(&mut self, root: &Path) {
        self.spaces.retain(|p| !same_path(p, root));
        if self.active.as_deref().is_some_and(|a| same_path(a, root)) {
            self.active = self.spaces.first().cloned();
        }
    }

    fn dedupe(&mut self) {
        let mut seen: Vec<PathBuf> = Vec::new();
        self.spaces.retain(|p| {
            if seen.iter().any(|s| same_path(s, p)) {
                false
            } else {
                seen.push(p.clone());
                true
            }
        });
    }
}

/// В Windows пути без учёта регистра и с любыми разделителями.
fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        assert_eq!(Settings::load(&p), Settings::default());
        assert_eq!(Settings::default().theme, ThemeChoice::Dark);
        let mut s = Settings::default();
        s.add_space(Path::new("C:/Работа"));
        s.theme = ThemeChoice::System;
        s.language = Some("ru".into());
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p), s);
    }

    #[test]
    fn broken_file_gives_defaults_and_is_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, "{ сломано").unwrap();
        assert_eq!(Settings::load(&p), Settings::default());
        assert!(dir.path().join("settings.broken.json").exists());
    }

    #[test]
    fn unknown_fields_and_missing_fields_are_fine() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        std::fs::write(&p, r#"{"theme":"light","future":1}"#).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.theme, ThemeChoice::Light);
        assert!(s.spaces.is_empty());
    }

    #[test]
    fn spaces_are_unique_and_forgetting_moves_active() {
        let mut s = Settings::default();
        s.add_space(Path::new("C:\\Работа"));
        s.add_space(Path::new("c:/работа/"));
        s.add_space(Path::new("D:/Личное"));
        assert_eq!(s.spaces.len(), 2);
        assert_eq!(s.active.as_deref(), Some(Path::new("D:/Личное")));
        s.forget_space(Path::new("d:\\личное"));
        assert_eq!(s.active.as_deref(), Some(Path::new("C:\\Работа")));
        s.forget_space(Path::new("C:\\Работа"));
        assert_eq!(s.active, None);
    }
}

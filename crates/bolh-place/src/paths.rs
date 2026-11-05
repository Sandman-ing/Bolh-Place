//! Где программа хранит свои файлы на компьютере.

use std::path::PathBuf;

const APP_DIR: &str = "Bolh Place";

/// `%APPDATA%\Bolh Place\settings.json`. У каждого компьютера свои
/// настройки, поэтому не в пространстве и не в облаке.
pub fn settings_file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join("settings.json"))
}

/// Куда по умолчанию предлагать новые пространства: `Документы\Bolh Place`.
pub fn default_spaces_dir() -> PathBuf {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR)
}

/// Временная папка для скачанного обновления.
pub fn update_dir() -> PathBuf {
    std::env::temp_dir().join("bolh-place-update")
}

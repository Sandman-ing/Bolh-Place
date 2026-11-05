use std::path::PathBuf;

/// Ошибки ядра. Текст здесь — для журнала и разработчика; пользователю
/// интерфейс показывает перевод по виду ошибки, а не этот текст.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ошибка ввода-вывода с {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("файл {path} повреждён: {reason}")]
    Corrupt { path: PathBuf, reason: String },
    #[error("файл {path} записан более новой версией программы (формат {found})")]
    TooNew { path: PathBuf, found: u32 },
    #[error("папка {0} не является пространством Bolh Place")]
    NotASpace(PathBuf),
    #[error("путь {0} выходит за пределы пространства")]
    OutsideSpace(PathBuf),
    #[error("{0} уже существует")]
    AlreadyExists(PathBuf),
    #[error("{0} не найден")]
    NotFound(PathBuf),
    #[error("недопустимое имя: {0}")]
    InvalidName(String),
    #[error("обновление: {0}")]
    Update(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Короткая запись для `map_err` у операций с файлами.
pub(crate) fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}

//! Ядро Bolh Place. Здесь всё, что не требует экрана: пространства,
//! формат файлов, история версий, корзина, вложения, настройки, переводы и
//! проверка обновлений. Поэтому ядро тестируется и в Linux, и в Windows.

pub mod canvas;
pub mod error;
pub mod format;
pub mod fsutil;
pub mod history;
pub mod i18n;
pub mod settings;
pub mod space;
pub mod trash;
pub mod update;

pub use error::{Error, Result};
/// id документов и холстов — чтобы интерфейсу не нужна была своя копия
/// библиотеки.
pub use uuid::Uuid;

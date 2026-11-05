//! Ядро Bolh Place. Здесь всё, что не требует экрана: пространства,
//! формат файлов, история версий, корзина, вложения, настройки, переводы и
//! проверка обновлений. Поэтому ядро тестируется и в Linux, и в Windows.

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

//! Команды и сочетания клавиш.

use gpui_kit::{App, KeyBinding};

pub const CONTEXT: &str = "Workspace";

gpui_kit::actions!(
    bolh,
    [
        OpenPalette,
        NewDocument,
        NewCanvas,
        NewFolder,
        Save,
        CloseTab,
        NextTab,
        PrevTab,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-p", OpenPalette, Some(CONTEXT)),
        KeyBinding::new("ctrl-n", NewDocument, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-n", NewCanvas, Some(CONTEXT)),
        KeyBinding::new("ctrl-s", Save, Some(CONTEXT)),
        KeyBinding::new("ctrl-w", CloseTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-tab", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some(CONTEXT)),
    ]);
}

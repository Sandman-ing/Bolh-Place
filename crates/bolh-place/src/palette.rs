//! Палитра команд Ctrl+P: что в ней есть и что делает каждый пункт.
//! Сам список собирается здесь без окна, чтобы его можно было проверить.

use std::path::PathBuf;

use bolh_core::i18n::{I18n, LANGUAGES};
use bolh_core::settings::ThemeChoice;
use bolh_core::space::{Entry, EntryKind};

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    NewDocument,
    NewCanvas,
    NewFolder,
    NewSpace,
    OpenSpace,
    SwitchSpace(PathBuf),
    Save,
    CloseTab,
    History,
    Trash,
    Theme(ThemeChoice),
    Language(&'static str),
    CheckUpdate,
    OpenFile(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    /// Дополнительные слова для поиска: путь файла, код языка.
    pub keywords: Vec<String>,
    pub command: Command,
}

/// Что палитре известно о состоянии программы.
pub struct Context<'a> {
    pub i18n: &'a I18n,
    pub has_space: bool,
    pub has_tab: bool,
    pub spaces: &'a [PathBuf],
    pub active_space: Option<&'a PathBuf>,
    pub tree: &'a [Entry],
}

fn item(label: impl Into<String>, command: Command) -> Item {
    Item {
        label: label.into(),
        keywords: Vec::new(),
        command,
    }
}

/// Команды. Недоступные сейчас (например, «Сохранить» без открытой
/// вкладки) в список не попадают.
pub fn commands(ctx: &Context) -> Vec<Item> {
    let t = |k| ctx.i18n.t(k).to_string();
    let mut out = Vec::new();
    if ctx.has_space {
        out.push(item(t("command.new_document"), Command::NewDocument));
        out.push(item(t("command.new_canvas"), Command::NewCanvas));
        out.push(item(t("command.new_folder"), Command::NewFolder));
    }
    if ctx.has_tab {
        out.push(item(t("command.save"), Command::Save));
        out.push(item(t("command.close_tab"), Command::CloseTab));
        out.push(item(t("command.history"), Command::History));
    }
    if ctx.has_space {
        out.push(item(t("command.trash"), Command::Trash));
    }
    out.push(item(t("command.new_space"), Command::NewSpace));
    out.push(item(t("command.open_space"), Command::OpenSpace));
    for p in ctx.spaces {
        if Some(p) == ctx.active_space {
            continue;
        }
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string_lossy().into_owned());
        out.push(item(
            ctx.i18n.tf("command.switch_space", &[("name", &name)]),
            Command::SwitchSpace(p.clone()),
        ));
    }
    out.push(item(
        t("command.theme_dark"),
        Command::Theme(ThemeChoice::Dark),
    ));
    out.push(item(
        t("command.theme_light"),
        Command::Theme(ThemeChoice::Light),
    ));
    out.push(item(
        t("command.theme_system"),
        Command::Theme(ThemeChoice::System),
    ));
    for (code, _) in LANGUAGES {
        let mut it = item(
            ctx.i18n
                .tf("command.language", &[("name", &I18n::native_name(code))]),
            Command::Language(code),
        );
        it.keywords.push(code.to_string());
        out.push(it);
    }
    out.push(item(t("command.check_update"), Command::CheckUpdate));
    out
}

/// Все документы и холсты пространства, для быстрого открытия.
pub fn files(tree: &[Entry]) -> Vec<Item> {
    let mut out = Vec::new();
    collect(tree, &mut out);
    out
}

fn collect(entries: &[Entry], out: &mut Vec<Item>) {
    for e in entries {
        match &e.kind {
            EntryKind::Folder(children) => collect(children, out),
            EntryKind::File(_) => out.push(Item {
                label: e.name.clone(),
                keywords: vec![e.rel.to_string_lossy().replace('\\', "/")],
                command: Command::OpenFile(e.rel.clone()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bolh_core::format::FileKind;

    fn tree() -> Vec<Entry> {
        vec![
            Entry {
                rel: "Папка".into(),
                name: "Папка".into(),
                kind: EntryKind::Folder(vec![Entry {
                    rel: PathBuf::from("Папка").join("План.bdoc"),
                    name: "План".into(),
                    kind: EntryKind::File(FileKind::Document),
                }]),
            },
            Entry {
                rel: "Схема.bcanvas".into(),
                name: "Схема".into(),
                kind: EntryKind::File(FileKind::Canvas),
            },
        ]
    }

    #[test]
    fn files_are_flattened_with_paths() {
        let f = files(&tree());
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].label, "План");
        assert_eq!(f[0].keywords, vec!["Папка/План.bdoc"]);
        assert_eq!(
            f[1].command,
            Command::OpenFile(PathBuf::from("Схема.bcanvas"))
        );
    }

    #[test]
    fn commands_depend_on_state() {
        let i18n = I18n::new("ru");
        let spaces = vec![PathBuf::from("C:/Работа"), PathBuf::from("C:/Личное")];
        let t = tree();
        let none = Context {
            i18n: &i18n,
            has_space: false,
            has_tab: false,
            spaces: &[],
            active_space: None,
            tree: &t,
        };
        let c = commands(&none);
        assert!(!c.iter().any(|i| i.command == Command::NewDocument));
        assert!(!c.iter().any(|i| i.command == Command::Save));
        assert!(c.iter().any(|i| i.command == Command::NewSpace));

        let full = Context {
            has_space: true,
            has_tab: true,
            spaces: &spaces,
            active_space: Some(&spaces[0]),
            ..none
        };
        let c = commands(&full);
        assert!(c.iter().any(|i| i.command == Command::Save));
        // В текущее пространство переходить незачем.
        let switches: Vec<_> = c
            .iter()
            .filter(|i| matches!(i.command, Command::SwitchSpace(_)))
            .collect();
        assert_eq!(switches.len(), 1);
        assert_eq!(switches[0].label, "Перейти в пространство «Личное»");
        // Каждый язык назван на нём самом.
        assert!(c.iter().any(|i| i.label == "Язык: English"));
        assert!(c.iter().any(|i| i.label == "Язык: Русский"));
        // Нигде нет непереведённых ключей.
        assert!(c.iter().all(|i| !i.label.starts_with("command.")));
    }
}

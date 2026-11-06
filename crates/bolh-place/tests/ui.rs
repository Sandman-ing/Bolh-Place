//! Сценарии в настоящем окне без экрана: клики, клавиши, ввод текста.
//! Проверяют и то, что видно в окне, и то, что легло на диск.

use std::path::{Path, PathBuf};

use bolh_core::format::{Content, FileKind};
use bolh_core::settings::{Settings, ThemeChoice};
use bolh_core::space::Space;
use bolh_place::palette::Command;
use bolh_place::workspace::Workspace;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions, px,
    size,
};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    settings_path: PathBuf,
}

fn fixture(spaces: &[&str]) -> (Fixture, Settings) {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings {
        language: Some("ru".into()),
        ..Settings::default()
    };
    for name in spaces {
        let root = dir.path().join(name);
        Space::create(&root).unwrap();
        settings.add_space(&root);
    }
    let root = dir.path().join(spaces[0]);
    settings.active = Some(root.clone());
    let settings_path = dir.path().join("settings.json");
    (
        Fixture {
            _dir: dir,
            root,
            settings_path,
        },
        settings,
    )
}

fn open(
    cx: &mut TestAppContext,
    settings: Settings,
    path: &Path,
) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.update(|cx| bolh_place::init(&settings, cx));
    let path = path.to_path_buf();
    let opened = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(1200.), px(800.)),
                ))),
                ..Default::default()
            },
            cx,
            move |window, cx| cx.new(|cx| Workspace::new(settings, Some(path), window, cx)),
        )
        .expect("окно открылось")
    });
    cx.run_until_parked();
    opened
}

/// Один шаг человека: действие в окне, затем программа доделывает всё,
/// что из него следует (события, перерисовка), как в настоящем окне.
fn step(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        f(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn doc_text(root: &Path, rel: &Path) -> String {
    let bytes = std::fs::read(root.join(rel)).unwrap();
    match Content::parse(FileKind::Document, &bytes, rel).unwrap() {
        Content::Document(d) => d.plain_text(),
        Content::Canvas(_) => panic!("ожидался документ"),
    }
}

#[gpui_kit::test]
fn new_document_button_creates_file_and_typing_is_saved(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, ws) = open(cx, settings, &fx.settings_path);

    step(cx, handle, |window, cx| window.click("new-document", cx));
    step(cx, handle, |window, cx| window.input("Привет, план", cx));
    step(cx, handle, |window, cx| window.press("ctrl-s", cx));

    let rel = cx.read(|cx| ws.read(cx).active_tab().unwrap().file.rel.clone());
    assert_eq!(rel, PathBuf::from("Без названия.bdoc"));
    assert_eq!(doc_text(&fx.root, &rel), "Привет, план");
    cx.read(|cx| assert!(!ws.read(cx).active_tab().unwrap().unsaved()));
}

#[gpui_kit::test]
fn ctrl_p_opens_palette_and_runs_a_command(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, ws) = open(cx, settings, &fx.settings_path);

    step(cx, handle, |window, cx| window.press("ctrl-p", cx));
    step(cx, handle, |window, cx| window.input("Создать холст", cx));
    step(cx, handle, |window, cx| window.press("enter", cx));

    cx.read(|cx| {
        let tab = ws.read(cx).active_tab().expect("холст открыт во вкладке");
        assert_eq!(tab.file.content.kind(), FileKind::Canvas);
    });
    assert!(fx.root.join("Новый холст.bcanvas").is_file());
}

#[gpui_kit::test]
fn tabs_open_once_switch_and_close(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, ws) = open(cx, settings, &fx.settings_path);
    cx.update_window(handle, |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.run(Command::NewDocument, window, cx);
            ws.run(Command::NewCanvas, window, cx);
            // Уже открытый файл не открывается второй раз.
            ws.run(Command::OpenFile("Без названия.bdoc".into()), window, cx);
            assert_eq!(ws.tabs().len(), 2);
            assert_eq!(ws.active_tab().unwrap().title(), "Без названия");
            ws.run(Command::CloseTab, window, cx);
            assert_eq!(ws.tabs().len(), 1);
            assert_eq!(ws.active_tab().unwrap().title(), "Новый холст");
        });
        window.render_frame(cx);
        window.press("ctrl-w", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| assert!(ws.read(cx).tabs().is_empty()));
    assert!(
        fx.root.join("Без названия.bdoc").is_file(),
        "закрытие не удаляет файл"
    );
}

#[gpui_kit::test]
fn theme_choice_applies_and_is_remembered(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, ws) = open(cx, settings, &fx.settings_path);
    cx.read(|cx| {
        assert_eq!(
            Theme::global(cx).mode,
            ThemeMode::Dark,
            "тёмная по умолчанию"
        )
    });
    cx.update_window(handle, |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.run(Command::Theme(ThemeChoice::Light), window, cx)
        });
    })
    .unwrap();
    cx.read(|cx| {
        assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
        assert_eq!(Theme::global(cx).theme_name().as_ref(), "Bolh Light");
    });
    assert_eq!(Settings::load(&fx.settings_path).theme, ThemeChoice::Light);
}

#[gpui_kit::test]
fn language_switch_changes_strings_and_is_remembered(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, ws) = open(cx, settings, &fx.settings_path);
    cx.update_window(handle, |_, window, cx| {
        ws.update(cx, |ws, cx| ws.run(Command::Language("en"), window, cx));
        ws.update(cx, |ws, cx| ws.run(Command::NewDocument, window, cx));
    })
    .unwrap();
    cx.read(|cx| {
        assert_eq!(bolh_place::i18n::current(cx).code(), "en");
        assert_eq!(ws.read(cx).active_tab().unwrap().title(), "Untitled");
    });
    assert_eq!(
        Settings::load(&fx.settings_path).language.as_deref(),
        Some("en")
    );
}

#[gpui_kit::test]
fn switching_spaces_saves_and_closes_tabs(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа", "Личное"]);
    let other = fx.root.parent().unwrap().join("Личное");
    let (handle, ws) = open(cx, settings, &fx.settings_path);
    step(cx, handle, |window, cx| window.click("new-document", cx));
    step(cx, handle, |window, cx| window.input("не потерять", cx));
    step(cx, handle, |window, cx| {
        ws.update(cx, |ws, cx| {
            ws.run(Command::SwitchSpace(other.clone()), window, cx)
        })
    });
    cx.read(|cx| {
        let ws = ws.read(cx);
        assert_eq!(ws.space().unwrap().root(), other.as_path());
        assert!(ws.tabs().is_empty());
    });
    assert_eq!(
        doc_text(&fx.root, Path::new("Без названия.bdoc")),
        "не потерять"
    );
    assert_eq!(
        Settings::load(&fx.settings_path).active.as_deref(),
        Some(other.as_path())
    );
}

#[gpui_kit::test]
fn change_from_cloud_is_shown_when_no_local_edits(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, ws) = open(cx, settings, &fx.settings_path);
    cx.update_window(handle, |_, window, cx| {
        ws.update(cx, |ws, cx| ws.run(Command::NewDocument, window, cx));
    })
    .unwrap();
    let rel = PathBuf::from("Без названия.bdoc");
    // Другой компьютер записал свою версию.
    let space = Space::open(&fx.root).unwrap();
    let mut other = space.open_file(&rel).unwrap();
    if let Content::Document(d) = &mut other.content {
        d.set_plain_text("с ноутбука");
    }
    std::fs::write(fx.root.join(&rel), other.content.to_bytes()).unwrap();

    cx.update_window(handle, |_, window, cx| {
        ws.update(cx, |ws, cx| ws.poll_external(window, cx));
    })
    .unwrap();
    // Проверка идёт в фоне.
    cx.run_until_parked();
    cx.read(|cx| {
        let tab = ws.read(cx).active_tab().unwrap();
        let Content::Document(d) = &tab.file.content else {
            panic!()
        };
        assert_eq!(d.plain_text(), "с ноутбука");
        assert!(!tab.unsaved());
    });
}

#[gpui_kit::test]
fn simultaneous_edit_keeps_both_versions(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, ws) = open(cx, settings, &fx.settings_path);
    step(cx, handle, |window, cx| window.click("new-document", cx));
    step(cx, handle, |window, cx| window.input("моё", cx));
    let rel = PathBuf::from("Без названия.bdoc");
    let space = Space::open(&fx.root).unwrap();
    let mut other = space.open_file(&rel).unwrap();
    if let Content::Document(d) = &mut other.content {
        d.set_plain_text("чужое");
    }
    std::fs::write(fx.root.join(&rel), other.content.to_bytes()).unwrap();

    cx.update_window(handle, |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.poll_external(window, cx);
            ws.run(Command::Save, window, cx);
        });
    })
    .unwrap();
    // Проверка и сохранение идут в фоне.
    cx.run_until_parked();
    assert_eq!(doc_text(&fx.root, &rel), "чужое", "чужая правка не затёрта");
    let copy = cx.read(|cx| ws.read(cx).active_tab().unwrap().file.rel.clone());
    assert_ne!(copy, rel);
    assert!(copy.to_string_lossy().contains("расхождение"));
    assert_eq!(doc_text(&fx.root, &copy), "моё");
}

#[gpui_kit::test]
fn delete_moves_to_trash_and_closes_tab(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, ws) = open(cx, settings, &fx.settings_path);
    cx.update_window(handle, |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.run(Command::NewCanvas, window, cx);
            ws.delete(Path::new("Новый холст.bcanvas"), window, cx);
            assert!(ws.tabs().is_empty());
        });
    })
    .unwrap();
    assert!(!fx.root.join("Новый холст.bcanvas").exists());
    let trash = Space::open(&fx.root).unwrap().trash().list().unwrap();
    assert_eq!(trash.len(), 1);
    assert_eq!(trash[0].name(), "Новый холст.bcanvas");
}

#[gpui_kit::test]
fn without_space_shows_welcome(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings {
        language: Some("ru".into()),
        ..Settings::default()
    };
    let (handle, ws) = open(cx, settings, &dir.path().join("s.json"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("welcome-new-space").is_some());
        assert!(window.try_find("new-document").is_none());
    })
    .unwrap();
    cx.read(|cx| assert!(ws.read(cx).space().is_none()));
}

#[gpui_kit::test]
fn edits_are_saved_automatically(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, _ws) = open(cx, settings, &fx.settings_path);
    step(cx, handle, |window, cx| window.click("new-document", cx));
    step(cx, handle, |window, cx| window.input("само", cx));
    let rel = Path::new("Без названия.bdoc");
    assert_eq!(
        doc_text(&fx.root, rel),
        "",
        "сразу не пишет на каждый символ"
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(doc_text(&fx.root, rel), "само");
}

#[gpui_kit::test]
fn clicking_a_file_in_the_tree_opens_it(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    Space::open(&fx.root)
        .unwrap()
        .create_file(
            Path::new(""),
            "Схема",
            FileKind::Canvas,
            bolh_core::space::Clock::system(),
        )
        .unwrap();
    let (handle, ws) = open(cx, settings, &fx.settings_path);
    step(cx, handle, |window, cx| {
        let item = window.find_all("tree:Схема.bcanvas");
        assert_eq!(item.len(), 1, "файл виден в дереве");
        window.click("tree:Схема.bcanvas", cx);
    });
    cx.read(|cx| assert_eq!(ws.read(cx).active_tab().unwrap().title(), "Схема"));
}

#[gpui_kit::test]
fn autosave_of_one_tab_is_not_cancelled_by_another(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, _ws) = open(cx, settings, &fx.settings_path);
    step(cx, handle, |window, cx| window.click("new-document", cx));
    step(cx, handle, |window, cx| window.input("первый", cx));
    step(cx, handle, |window, cx| window.click("new-document", cx));
    step(cx, handle, |window, cx| window.input("второй", cx));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(doc_text(&fx.root, Path::new("Без названия.bdoc")), "первый");
    assert_eq!(
        doc_text(&fx.root, Path::new("Без названия 2.bdoc")),
        "второй"
    );
}

#[gpui_kit::test]
fn typing_during_a_background_save_is_saved_next(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, ws) = open(cx, settings, &fx.settings_path);
    step(cx, handle, |window, cx| window.click("new-document", cx));
    step(cx, handle, |window, cx| window.input("один", cx));
    // Сохранение ушло в фон, а человек печатает дальше, не дожидаясь диска.
    cx.update_window(handle, |_, window, cx| {
        ws.update(cx, |ws, cx| ws.save_active(window, cx));
        window.input(" два", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let rel = Path::new("Без названия.bdoc");
    assert_eq!(doc_text(&fx.root, rel), "один", "записано то, что было");
    cx.read(|cx| assert!(ws.read(cx).active_tab().unwrap().unsaved()));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(doc_text(&fx.root, rel), "один два", "и допечатанное тоже");
    cx.read(|cx| assert!(!ws.read(cx).active_tab().unwrap().unsaved()));
}

#[gpui_kit::test]
fn closing_waits_for_a_background_save_without_a_false_conflict(cx: &mut TestAppContext) {
    let (fx, settings) = fixture(&["Работа"]);
    let (handle, ws) = open(cx, settings, &fx.settings_path);
    step(cx, handle, |window, cx| window.click("new-document", cx));
    step(cx, handle, |window, cx| window.input("один", cx));
    cx.update_window(handle, |_, window, cx| {
        ws.update(cx, |ws, cx| ws.save_active(window, cx));
        window.input(" два", cx);
    })
    .unwrap();
    // Окно закрывают, пока фоновое сохранение не дописано.
    cx.update_window(handle, |_, window, cx| {
        ws.update(cx, |ws, cx| ws.save_all(window, cx));
    })
    .unwrap();
    let rel = Path::new("Без названия.bdoc");
    assert_eq!(doc_text(&fx.root, rel), "один два", "всё записано сразу");
    cx.run_until_parked();
    // Две записи одного файла не приняли друг друга за чужую правку.
    let files: Vec<String> = std::fs::read_dir(&fx.root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bdoc"))
        .collect();
    assert_eq!(files, ["Без названия.bdoc"], "без копий-расхождений");
    assert_eq!(doc_text(&fx.root, rel), "один два");
    cx.read(|cx| {
        let tab = ws.read(cx).active_tab().unwrap();
        assert_eq!(tab.file.rel, rel);
        assert!(!tab.unsaved());
    });
}

//! Пространство целиком: дерево, файлы, корзина, история, вложения и
//! правки, пришедшие через облачную папку.

use std::fs;
use std::path::{Path, PathBuf};

use bolh_core::Error;
use bolh_core::format::{Content, FileKind};
use bolh_core::space::{Clock, EntryKind, External, SaveOutcome, Space};
use chrono::{Duration, FixedOffset, TimeZone, Utc};

fn clock(min: i64) -> Clock {
    Clock {
        now: Utc.with_ymd_and_hms(2026, 11, 5, 10, 0, 0).unwrap() + Duration::minutes(min),
        tz: FixedOffset::east_opt(3 * 3600).unwrap(),
    }
}

fn new_space() -> (tempfile::TempDir, Space) {
    let dir = tempfile::tempdir().unwrap();
    let space = Space::create(&dir.path().join("Работа")).unwrap();
    (dir, space)
}

fn text(space: &Space, rel: &Path) -> String {
    match space.open_file(rel).unwrap().content {
        Content::Document(d) => d.plain_text(),
        Content::Canvas(_) => panic!("ожидался документ"),
    }
}

fn set_text(f: &mut bolh_core::space::OpenFile, t: &str) {
    match &mut f.content {
        Content::Document(d) => d.set_plain_text(t),
        Content::Canvas(_) => panic!(),
    }
}

/// Имитирует правку на другом компьютере: облако заменяет файл целиком.
fn edit_elsewhere(space: &Space, rel: &Path, t: &str) {
    let mut other = space.open_file(rel).unwrap();
    set_text(&mut other, t);
    fs::write(space.root().join(rel), other.content.to_bytes()).unwrap();
}

#[test]
fn create_and_open_space() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Личное");
    assert!(matches!(Space::open(&root), Err(Error::NotASpace(_))));
    let s = Space::create(&root).unwrap();
    assert_eq!(s.name(), "Личное");
    assert!(root.join("attachments").is_dir());
    assert!(Space::is_space(&root));
    // Повторное создание поверх существующего ничего не ломает.
    fs::write(
        root.join("x.bdoc"),
        Content::new(FileKind::Document).to_bytes(),
    )
    .unwrap();
    Space::create(&root).unwrap();
    assert_eq!(s.tree().unwrap().len(), 1);
}

#[test]
fn tree_lists_folders_first_and_hides_service_files() {
    let (_d, s) = new_space();
    let r = s.root();
    s.create_folder(Path::new(""), "Проекты").unwrap();
    s.create_file(
        Path::new("Проекты"),
        "Вложенный",
        FileKind::Document,
        clock(0),
    )
    .unwrap();
    s.create_file(Path::new(""), "банан", FileKind::Document, clock(0))
        .unwrap();
    s.create_file(Path::new(""), "Арбуз", FileKind::Canvas, clock(0))
        .unwrap();
    fs::write(r.join("заметка.txt"), "x").unwrap();
    fs::write(r.join(".~bolh-123"), "x").unwrap();
    fs::write(r.join("attachments").join("img.png"), "x").unwrap();

    let tree = s.tree().unwrap();
    let names: Vec<_> = tree.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Проекты", "Арбуз", "банан"]);
    assert_eq!(tree[1].kind, EntryKind::File(FileKind::Canvas));
    let EntryKind::Folder(children) = &tree[0].kind else {
        panic!()
    };
    assert_eq!(
        children[0].rel,
        PathBuf::from("Проекты").join("Вложенный.bdoc")
    );
}

#[test]
fn create_file_picks_free_name_and_validates() {
    let (_d, s) = new_space();
    let a = s
        .create_file(Path::new(""), "План", FileKind::Document, clock(0))
        .unwrap();
    let b = s
        .create_file(Path::new(""), "План", FileKind::Document, clock(0))
        .unwrap();
    assert_eq!(a, PathBuf::from("План.bdoc"));
    assert_eq!(b, PathBuf::from("План 2.bdoc"));
    assert!(matches!(
        s.create_file(Path::new(""), "a/b", FileKind::Document, clock(0)),
        Err(Error::InvalidName(_))
    ));
    assert!(matches!(
        s.create_file(Path::new("нет такой"), "x", FileKind::Document, clock(0)),
        Err(Error::NotFound(_))
    ));
    // Новый файл сразу в истории.
    let f = s.open_file(&a).unwrap();
    assert_eq!(s.history().list(f.content.id()).unwrap().len(), 1);
}

#[test]
fn paths_cannot_escape_space() {
    let (_d, s) = new_space();
    for bad in ["../x.bdoc", ".bolh/space.json", "/etc/x.bdoc"] {
        assert!(
            matches!(s.resolve(Path::new(bad)), Err(Error::OutsideSpace(_))),
            "{bad}"
        );
    }
    assert!(s.open_file(Path::new("../x.bdoc")).is_err());
}

#[test]
fn save_and_reopen_gives_same_content() {
    let (_d, s) = new_space();
    let rel = s
        .create_file(Path::new(""), "Док", FileKind::Document, clock(0))
        .unwrap();
    let mut f = s.open_file(&rel).unwrap();
    assert!(!f.is_dirty());
    set_text(&mut f, "строка 1\nстрока 2");
    assert!(f.is_dirty());
    assert_eq!(
        s.save(&mut f, "конфликт", clock(1)).unwrap(),
        SaveOutcome::Saved
    );
    assert!(!f.is_dirty());
    let again = s.open_file(&rel).unwrap();
    assert_eq!(again.content, f.content);
    assert!(matches!(s.check_external(&f), External::Unchanged));
}

#[test]
fn rename_keeps_extension_and_refuses_collision() {
    let (_d, s) = new_space();
    let a = s
        .create_file(Path::new(""), "a", FileKind::Canvas, clock(0))
        .unwrap();
    s.create_file(Path::new(""), "b", FileKind::Canvas, clock(0))
        .unwrap();
    assert!(matches!(s.rename(&a, "b"), Err(Error::AlreadyExists(_))));
    assert!(matches!(s.rename(&a, "B"), Err(Error::AlreadyExists(_))));
    let to = s.rename(&a, "Схема").unwrap();
    assert_eq!(to, PathBuf::from("Схема.bcanvas"));
    // Смена только регистра у самого себя — можно.
    let to = s.rename(&to, "схема").unwrap();
    assert!(s.root().join(&to).is_file());
    let folder = s.create_folder(Path::new(""), "Папка").unwrap();
    assert_eq!(s.rename(&folder, "Новая").unwrap(), PathBuf::from("Новая"));
}

#[test]
fn trash_restore_and_purge() {
    let (_d, s) = new_space();
    s.create_folder(Path::new(""), "Папка").unwrap();
    let rel = s
        .create_file(Path::new("Папка"), "Док", FileKind::Document, clock(0))
        .unwrap();
    let mut f = s.open_file(&rel).unwrap();
    set_text(&mut f, "важное");
    s.save(&mut f, "к", clock(10)).unwrap();
    let id = f.content.id();

    let e = s.delete(&rel, clock(20)).unwrap();
    assert!(!s.root().join(&rel).exists());
    assert_eq!(s.trash().list().unwrap(), vec![e.clone()]);

    // Папку тоже удалили — при возврате она появится снова.
    s.delete(Path::new("Папка"), clock(21)).unwrap();
    let list = s.trash().list().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].name(), "Папка", "сначала удалённое последним");
    assert!(list[0].is_dir);

    let back = s.restore(&e).unwrap();
    assert_eq!(back, rel);
    assert_eq!(text(&s, &back), "важное");

    // Место занято — возвращается рядом под другим именем.
    let e2 = s.delete(&back, clock(30)).unwrap();
    s.create_file(Path::new("Папка"), "Док", FileKind::Document, clock(31))
        .unwrap();
    let back2 = s.restore(&e2).unwrap();
    assert_eq!(back2, PathBuf::from("Папка").join("Док 2.bdoc"));

    // Навсегда — вместе с историей.
    let e3 = s.delete(&back2, clock(40)).unwrap();
    assert!(!s.history().list(id).unwrap().is_empty());
    s.purge(&e3).unwrap();
    assert!(s.history().list(id).unwrap().is_empty());
    assert!(!s.trash().list().unwrap().iter().any(|x| x.key == e3.key));
}

#[test]
fn trash_ignores_broken_or_malicious_entries() {
    let (_d, s) = new_space();
    let trash = s.root().join(".bolh").join("trash");
    let slot = trash.join("6f9619ff-8b86-d011-b42d-00cf4fc964ff");
    fs::create_dir_all(&slot).unwrap();
    fs::write(
        slot.join("entry.json"),
        r#"{"original":"../../evil.bdoc","deleted_at":"2026-11-05T10:00:00Z"}"#,
    )
    .unwrap();
    fs::write(slot.join("evil.bdoc"), "x").unwrap();
    fs::create_dir_all(trash.join("мусор")).unwrap();
    assert!(s.trash().list().unwrap().is_empty());
}

#[test]
fn delete_missing_is_not_found() {
    let (_d, s) = new_space();
    assert!(matches!(
        s.delete(Path::new("нет.bdoc"), clock(0)),
        Err(Error::NotFound(_))
    ));
    assert!(s.delete(Path::new(""), clock(0)).is_err());
}

#[test]
fn history_snapshot_and_rollback() {
    let (_d, s) = new_space();
    let rel = s
        .create_file(Path::new(""), "Д", FileKind::Document, clock(0))
        .unwrap();
    let mut f = s.open_file(&rel).unwrap();
    for (i, t) in ["один", "два", "три"].iter().enumerate() {
        set_text(&mut f, t);
        s.save(&mut f, "к", clock(10 * (i as i64 + 1))).unwrap();
    }
    let h = s.history();
    let snaps = h.list(f.content.id()).unwrap();
    assert_eq!(snaps.len(), 4, "пустой при создании и три правки");
    let one = snaps
        .iter()
        .find(|x| x.at == clock(10).now)
        .unwrap()
        .clone();

    s.restore_snapshot(&mut f, &one, "к", clock(41)).unwrap();
    assert_eq!(text(&s, &rel), "один");
    // Откат отменяем: «три» осталось в истории.
    let snaps = h.list(f.content.id()).unwrap();
    let texts: Vec<String> = snaps
        .iter()
        .map(|x| String::from_utf8(h.read(x).unwrap()).unwrap())
        .collect();
    assert!(texts.iter().any(|t| t.contains("три")));
}

#[test]
fn external_change_is_noticed_and_accepted_when_no_local_edits() {
    let (_d, s) = new_space();
    let rel = s
        .create_file(Path::new(""), "Д", FileKind::Document, clock(0))
        .unwrap();
    let mut f = s.open_file(&rel).unwrap();
    edit_elsewhere(&s, &rel, "с ноутбука");
    let External::Changed(c, b) = s.check_external(&f) else {
        panic!("правка снаружи не замечена")
    };
    assert!(!f.is_dirty());
    s.accept_external(&mut f, *c, b);
    assert!(matches!(s.check_external(&f), External::Unchanged));
    assert!(!f.is_dirty());
    set_text(&mut f, "дальше");
    assert_eq!(s.save(&mut f, "к", clock(5)).unwrap(), SaveOutcome::Saved);
}

#[test]
fn simultaneous_edit_keeps_both_versions() {
    let (_d, s) = new_space();
    let rel = s
        .create_file(Path::new(""), "План", FileKind::Document, clock(0))
        .unwrap();
    let mut f = s.open_file(&rel).unwrap();
    set_text(&mut f, "моя правка");
    edit_elsewhere(&s, &rel, "чужая правка");

    let out = s.save(&mut f, "расхождение", clock(7)).unwrap();
    let SaveOutcome::Conflict { original, copy } = out else {
        panic!("ожидалось расхождение")
    };
    assert_eq!(original, rel);
    assert_eq!(
        copy,
        PathBuf::from("План (расхождение 2026-11-05 13-07).bdoc")
    );
    assert_eq!(text(&s, &rel), "чужая правка", "чужое не затёрто");
    assert_eq!(text(&s, &copy), "моя правка", "своё сохранено");
    assert_eq!(f.rel, copy);
    assert_ne!(
        s.open_file(&rel).unwrap().content.id(),
        f.content.id(),
        "у копии своя история"
    );
    // Дальше правки идут в копию без новых расхождений.
    set_text(&mut f, "моя правка 2");
    assert_eq!(
        s.save(&mut f, "расхождение", clock(8)).unwrap(),
        SaveOutcome::Saved
    );
}

#[test]
fn same_change_on_both_sides_is_not_a_conflict() {
    let (_d, s) = new_space();
    let rel = s
        .create_file(Path::new(""), "Д", FileKind::Document, clock(0))
        .unwrap();
    let mut f = s.open_file(&rel).unwrap();
    set_text(&mut f, "одинаково");
    fs::write(s.root().join(&rel), f.content.to_bytes()).unwrap();
    assert_eq!(s.save(&mut f, "к", clock(1)).unwrap(), SaveOutcome::Saved);
}

#[test]
fn deleted_elsewhere_with_local_edits_is_recreated() {
    let (_d, s) = new_space();
    let rel = s
        .create_file(Path::new(""), "Д", FileKind::Document, clock(0))
        .unwrap();
    let mut f = s.open_file(&rel).unwrap();
    fs::remove_file(s.root().join(&rel)).unwrap();
    assert!(matches!(s.check_external(&f), External::Deleted));
    set_text(&mut f, "не потерять");
    assert_eq!(s.save(&mut f, "к", clock(1)).unwrap(), SaveOutcome::Saved);
    assert_eq!(text(&s, &rel), "не потерять");
}

#[test]
fn half_synced_file_is_unreadable_not_a_crash() {
    let (_d, s) = new_space();
    let rel = s
        .create_file(Path::new(""), "Д", FileKind::Document, clock(0))
        .unwrap();
    let f = s.open_file(&rel).unwrap();
    fs::write(s.root().join(&rel), b"{\"format\": \"bolh-d").unwrap();
    assert!(matches!(s.check_external(&f), External::Unreadable(_)));
}

#[test]
fn hand_formatted_file_is_not_dirty() {
    let (_d, s) = new_space();
    let rel = PathBuf::from("ручной.bdoc");
    fs::write(
        s.root().join(&rel),
        r#"{"format":"bolh-doc","version":1,"id":"1b4e28ba-2fa1-11d2-883f-0016d3cca427","blocks":[]}"#,
    )
    .unwrap();
    let f = s.open_file(&rel).unwrap();
    assert!(!f.is_dirty());
}

#[test]
fn newer_format_is_not_overwritten() {
    let (_d, s) = new_space();
    let rel = PathBuf::from("новый.bdoc");
    let original = r#"{"format":"bolh-doc","version":9,"id":"1b4e28ba-2fa1-11d2-883f-0016d3cca427","blocks":[]}"#;
    fs::write(s.root().join(&rel), original).unwrap();
    let mut f = s.open_file(&rel).unwrap();
    set_text(&mut f, "x");
    assert!(matches!(
        s.save(&mut f, "к", clock(1)),
        Err(Error::TooNew { found: 9, .. })
    ));
    assert_eq!(fs::read_to_string(s.root().join(&rel)).unwrap(), original);
}

#[test]
fn attachments_are_copied_with_free_names() {
    let (d, s) = new_space();
    let src = d.path().join("фото.png");
    fs::write(&src, b"\x89PNG").unwrap();
    let a = s.add_attachment(&src).unwrap();
    let b = s.add_attachment(&src).unwrap();
    assert_eq!(a, Path::new("attachments").join("фото.png"));
    assert_eq!(b, Path::new("attachments").join("фото 2.png"));
    assert_eq!(fs::read(s.root().join(&b)).unwrap(), b"\x89PNG");
    assert!(src.exists(), "оригинал на месте");
}

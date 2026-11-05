//! Холст в настоящем окне без экрана: мышь, клавиши, и что легло на диск.

use std::path::{Path, PathBuf};

use bolh_core::format::{Content, FileKind, Node};
use bolh_core::settings::Settings;
use bolh_core::space::{Clock, Space};
use bolh_place::workspace::Workspace;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, ScrollDelta, TestAppContext, WindowBounds,
    WindowOptions, point, px, size,
};

struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    handle: AnyWindowHandle,
    ws: Entity<Workspace>,
}

const REL: &str = "Схема.bcanvas";

/// Пространство с одним холстом, открытым во вкладке. `nodes` — что
/// лежит на холсте заранее.
fn open_canvas(cx: &mut TestAppContext, nodes: Vec<Node>) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Работа");
    let space = Space::create(&root).unwrap();
    space
        .create_file(Path::new(""), "Схема", FileKind::Canvas, Clock::system())
        .unwrap();
    if !nodes.is_empty() {
        let mut f = space.open_file(Path::new(REL)).unwrap();
        if let Content::Canvas(c) = &mut f.content {
            c.nodes = nodes;
        }
        space.save(&mut f, "x", Clock::system()).unwrap();
    }
    let mut settings = Settings {
        language: Some("ru".into()),
        ..Settings::default()
    };
    settings.add_space(&root);
    cx.update(|cx| bolh_place::init(&settings, cx));
    let path = dir.path().join("s.json");
    let (handle, ws) = cx.update(|cx| {
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
        .unwrap()
    });
    cx.run_until_parked();
    let fx = Fx {
        _dir: dir,
        root,
        handle,
        ws,
    };
    step(cx, &fx, |ws, window, cx| {
        ws.open_file(Path::new(REL), window, cx)
    });
    fx
}

fn step(
    cx: &mut TestAppContext,
    fx: &Fx,
    f: impl FnOnce(&mut Workspace, &mut gpui_kit::Window, &mut gpui_kit::Context<Workspace>),
) {
    let ws = fx.ws.clone();
    cx.update_window(fx.handle, |_, window, cx| {
        ws.update(cx, |ws, cx| f(ws, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn act(
    cx: &mut TestAppContext,
    fx: &Fx,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
) {
    cx.update_window(fx.handle, |_, window, cx| {
        window.render_frame(cx);
        f(window, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn nodes_in_view(cx: &mut TestAppContext, fx: &Fx) -> Vec<Node> {
    cx.read(|cx| {
        let ws = fx.ws.read(cx);
        let view = ws.active_tab().unwrap().canvas_view().unwrap().read(cx);
        view.canvas().nodes.clone()
    })
}

fn nodes_on_disk(fx: &Fx) -> Vec<Node> {
    let bytes = std::fs::read(fx.root.join(REL)).unwrap();
    match Content::parse(FileKind::Canvas, &bytes, Path::new(REL)).unwrap() {
        Content::Canvas(c) => c.nodes,
        _ => panic!(),
    }
}

fn save_now(cx: &mut TestAppContext, fx: &Fx) {
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    let _ = fx;
}

fn card(id: &str, x: f64, y: f64) -> Node {
    Node::card(id, x, y, 240.0, 140.0)
}

#[gpui_kit::test]
fn double_click_creates_card_and_typed_text_is_saved(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![]);
    act(cx, &fx, |w, cx| w.double_click("canvas", cx));
    assert_eq!(
        nodes_in_view(cx, &fx).len(),
        1,
        "двойной щелчок — новая карточка"
    );
    act(cx, &fx, |w, cx| w.input("План на неделю", cx));
    act(cx, &fx, |w, cx| w.press("escape", cx));
    let editing = cx.read(|cx| {
        let ws = fx.ws.read(cx);
        ws.active_tab()
            .unwrap()
            .canvas_view()
            .unwrap()
            .read(cx)
            .is_editing()
    });
    assert!(!editing, "Esc заканчивает правку текста");
    save_now(cx, &fx);
    let disk = nodes_on_disk(&fx);
    assert_eq!(disk.len(), 1);
    assert_eq!(disk[0].text, "План на неделю");
    assert_eq!(disk[0].kind, "card");
}

#[gpui_kit::test]
fn drag_moves_card_with_snapping_and_undo_restores(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    let before = nodes_in_view(cx, &fx)[0].clone();
    act(cx, &fx, |w, cx| {
        let b = w.find("node:a").bounds();
        let from = b.center();
        w.drag(from, point(from.x + px(53.), from.y + px(27.)), cx);
    });
    let moved = &nodes_in_view(cx, &fx)[0];
    assert_ne!(
        (moved.x, moved.y),
        (before.x, before.y),
        "карточка сдвинулась"
    );
    assert_eq!(moved.x % 20.0, 0.0, "прилипла к сетке по x: {}", moved.x);
    assert_eq!(moved.y % 20.0, 0.0, "прилипла к сетке по y: {}", moved.y);
    act(cx, &fx, |w, cx| w.press("ctrl-z", cx));
    let back = &nodes_in_view(cx, &fx)[0];
    assert_eq!(
        (back.x, back.y),
        (before.x, before.y),
        "Ctrl+Z вернул на место"
    );
    act(cx, &fx, |w, cx| w.press("ctrl-y", cx));
    assert_eq!(nodes_in_view(cx, &fx)[0].x, moved.x, "Ctrl+Y повторил");
}

#[gpui_kit::test]
fn delete_and_undo_through_keys(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0), card("b", 400.0, 0.0)]);
    act(cx, &fx, |w, cx| w.click("node:a", cx));
    act(cx, &fx, |w, cx| w.press("delete", cx));
    let left: Vec<String> = nodes_in_view(cx, &fx)
        .iter()
        .map(|n| n.id.clone())
        .collect();
    assert_eq!(left, ["b"]);
    save_now(cx, &fx);
    assert_eq!(nodes_on_disk(&fx).len(), 1, "удаление сохранено");
    act(cx, &fx, |w, cx| w.press("ctrl-z", cx));
    assert_eq!(nodes_in_view(cx, &fx).len(), 2);
}

#[gpui_kit::test]
fn copy_paste_and_select_all(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.click("node:a", cx));
    act(cx, &fx, |w, cx| w.press("ctrl-c", cx));
    act(cx, &fx, |w, cx| w.press("ctrl-v", cx));
    let nodes = nodes_in_view(cx, &fx);
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[1].x, nodes[0].x + 20.0, "копия сдвинута");
    act(cx, &fx, |w, cx| w.press("ctrl-a", cx));
    act(cx, &fx, |w, cx| w.press("ctrl-d", cx));
    assert_eq!(
        nodes_in_view(cx, &fx).len(),
        4,
        "дублировано всё выделенное"
    );
}

#[gpui_kit::test]
fn wheel_zooms_and_touchpad_pans(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![]);
    let view = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            let ws = fx.ws.read(cx);
            ws.active_tab()
                .unwrap()
                .canvas_view()
                .unwrap()
                .read(cx)
                .view()
        })
    };
    let v0 = view(cx);
    act(cx, &fx, |w, cx| {
        w.scroll("canvas", ScrollDelta::Lines(point(0., 3.)), cx)
    });
    let v1 = view(cx);
    assert!(
        v1.zoom > v0.zoom,
        "колесо вверх — крупнее: {} → {}",
        v0.zoom,
        v1.zoom
    );
    act(cx, &fx, |w, cx| {
        w.scroll("canvas", ScrollDelta::Pixels(point(px(0.), px(-100.))), cx)
    });
    let v2 = view(cx);
    assert_eq!(v2.zoom, v1.zoom, "тачпад не меняет масштаб");
    assert_eq!(v2.offset_y, v1.offset_y - 100.0, "тачпад сдвигает поле");
    // Вид не пишется в файл: прокрутка — не правка.
    cx.read(|cx| assert!(!fx.ws.read(cx).active_tab().unwrap().file.is_dirty()));
}

#[gpui_kit::test]
fn marquee_selects_and_arrows_move(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0), card("b", 400.0, 0.0)]);
    act(cx, &fx, |w, cx| {
        let a = w.find("node:a").bounds();
        let b = w.find("node:b").bounds();
        let from = point(a.origin.x - px(30.), a.origin.y - px(30.));
        let to = point(
            b.origin.x + b.size.width + px(30.),
            b.origin.y + b.size.height + px(30.),
        );
        w.drag(from, to, cx);
    });
    let sel = cx.read(|cx| {
        let ws = fx.ws.read(cx);
        let v = ws.active_tab().unwrap().canvas_view().unwrap().read(cx);
        v.editor().selection().len()
    });
    assert_eq!(sel, 2, "рамка выделила обе карточки");
    act(cx, &fx, |w, cx| w.press("right", cx));
    act(cx, &fx, |w, cx| w.press("shift-down", cx));
    let n = nodes_in_view(cx, &fx);
    assert_eq!((n[0].x, n[0].y), (1.0, 20.0));
    assert_eq!((n[1].x, n[1].y), (401.0, 20.0));
}

#[gpui_kit::test]
fn layer_order_from_keys(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0), card("b", 100.0, 50.0)]);
    act(cx, &fx, |w, cx| {
        w.click_at("node:a", point(px(10.), px(10.)), cx)
    });
    act(cx, &fx, |w, cx| w.press("ctrl-shift-]", cx));
    let order: Vec<String> = nodes_in_view(cx, &fx)
        .iter()
        .map(|n| n.id.clone())
        .collect();
    assert_eq!(order, ["b", "a"]);
}

#[gpui_kit::test]
fn reopened_canvas_has_the_same_cards(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![]);
    act(cx, &fx, |w, cx| w.double_click("canvas", cx));
    act(cx, &fx, |w, cx| w.input("раз", cx));
    step(cx, &fx, |ws, window, cx| ws.save_all(window, cx));
    let saved = nodes_in_view(cx, &fx);
    step(cx, &fx, |ws, window, cx| {
        ws.close_tab(0, window, cx);
        ws.open_file(Path::new(REL), window, cx);
    });
    assert_eq!(nodes_in_view(cx, &fx), saved);
}

fn editing(cx: &mut TestAppContext, fx: &Fx) -> bool {
    cx.read(|cx| {
        let ws = fx.ws.read(cx);
        let v = ws.active_tab().unwrap().canvas_view().unwrap().read(cx);
        v.is_editing()
    })
}

#[gpui_kit::test]
fn click_inside_edited_card_keeps_editing(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    act(cx, &fx, |w, cx| w.input("раз", cx));
    act(cx, &fx, |w, cx| w.click("node:a", cx));
    assert!(
        editing(cx, &fx),
        "щелчок внутри карточки не прерывает правку"
    );
    act(cx, &fx, |w, cx| w.input("два", cx));
    assert!(nodes_in_view(cx, &fx)[0].text.contains("два"));
}

#[gpui_kit::test]
fn after_escape_canvas_keys_work_again(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    act(cx, &fx, |w, cx| w.input("т", cx));
    act(cx, &fx, |w, cx| w.press("escape", cx));
    assert!(!editing(cx, &fx));
    act(cx, &fx, |w, cx| w.press("right", cx));
    assert_eq!(
        nodes_in_view(cx, &fx)[0].x,
        1.0,
        "стрелка двигает карточку после Esc"
    );
}

#[gpui_kit::test]
fn plain_click_on_one_of_selected_keeps_only_it(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0), card("b", 400.0, 0.0)]);
    act(cx, &fx, |w, cx| w.press("ctrl-a", cx));
    act(cx, &fx, |w, cx| w.click("node:b", cx));
    let sel = cx.read(|cx| {
        let ws = fx.ws.read(cx);
        let v = ws.active_tab().unwrap().canvas_view().unwrap().read(cx);
        v.editor().selection().to_vec()
    });
    assert_eq!(sel, ["b"]);
}

#[gpui_kit::test]
fn long_text_makes_card_taller(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    let long = "очень длинная строка текста ".repeat(12);
    act(cx, &fx, |w, cx| w.input(&long, cx));
    act(cx, &fx, |w, cx| w.press("escape", cx));
    act(cx, &fx, |_, _| {});
    let h = nodes_in_view(cx, &fx)[0].height;
    assert!(h > 140.0, "карточка выросла под текст: {h}");
    save_now(cx, &fx);
    assert_eq!(nodes_on_disk(&fx)[0].height, h, "высота сохранена");
}

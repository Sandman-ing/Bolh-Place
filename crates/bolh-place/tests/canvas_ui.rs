//! Холст в настоящем окне без экрана: мышь, клавиши, и что легло на диск.

use std::path::{Path, PathBuf};

use bolh_core::format::{Content, FileKind, Node};
use bolh_core::rich::{Block, Kind, RichText};
use bolh_core::settings::Settings;
use bolh_core::space::{Clock, Space};
use bolh_place::canvas_view::Tool;
use bolh_place::workspace::Workspace;
use gpui_kit::test::{ClickOptions, TestWindowExt};
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
    assert_eq!(disk[0].plain_text(), "План на неделю");
    assert_eq!(disk[0].kind, "card");
}

#[gpui_kit::test]
fn drag_follows_the_mouse_snaps_near_grid_and_undo_restores(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    let drag_by = |cx: &mut TestAppContext, dx: f32, dy: f32| {
        act(cx, &fx, |w, cx| {
            let from = w.find("node:a").bounds().center();
            w.drag(from, point(from.x + px(dx), from.y + px(dy)), cx);
        });
        let n = &nodes_in_view(cx, &fx)[0];
        (n.x, n.y)
    };
    // Вдали от линий сетки карточка идёт точно за мышью: 53 и 27 — в семи
    // точках от ближайших линий, дальше порога прилипания.
    assert_eq!(drag_by(cx, 53., 27.), (53.0, 27.0), "идёт за мышью");
    // Рядом с линией — прилипает к ней: 58 → 60, 42 → 40.
    assert_eq!(drag_by(cx, 5., 15.), (60.0, 40.0), "прилипла к сетке");
    act(cx, &fx, |w, cx| w.press("ctrl-z", cx));
    let n = &nodes_in_view(cx, &fx)[0];
    assert_eq!((n.x, n.y), (53.0, 27.0), "Ctrl+Z вернул на шаг назад");
    act(cx, &fx, |w, cx| w.press("ctrl-z", cx));
    let n = &nodes_in_view(cx, &fx)[0];
    assert_eq!((n.x, n.y), (0.0, 0.0), "и ещё на шаг");
    act(cx, &fx, |w, cx| w.press("ctrl-y", cx));
    assert_eq!(nodes_in_view(cx, &fx)[0].x, 53.0, "Ctrl+Y повторил");
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
    cx.read(|cx| assert!(!fx.ws.read(cx).active_tab().unwrap().unsaved()));
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
    assert!(nodes_in_view(cx, &fx)[0].plain_text().contains("два"));
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
fn long_text_makes_card_taller_on_screen_but_not_in_the_file(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    let long = "очень длинная строка текста ".repeat(12);
    act(cx, &fx, |w, cx| w.input(&long, cx));
    act(cx, &fx, |w, cx| w.press("escape", cx));
    act(cx, &fx, |_, _| {});
    let zoom = cx.read(|cx| {
        let ws = fx.ws.read(cx);
        ws.active_tab()
            .unwrap()
            .canvas_view()
            .unwrap()
            .read(cx)
            .view()
            .zoom
    });
    let on_screen = cx
        .update_window(fx.handle, |_, w, _| w.find("node:a").bounds().size.height)
        .unwrap();
    let on_screen = f64::from(f32::from(on_screen)) / zoom;
    assert!(on_screen > 140.0, "карточка выросла под текст: {on_screen}");
    // Мышь и выделение знают настоящую высоту.
    let shown = cx.read(|cx| {
        let ws = fx.ws.read(cx);
        let v = ws.active_tab().unwrap().canvas_view().unwrap().read(cx);
        v.editor().rect_of("a").unwrap().h
    });
    assert!(
        (shown - on_screen).abs() < 1.0,
        "{shown} против {on_screen}"
    );
    // В файле — заданная человеком высота: подгонка под текст не правка, и
    // карточка не растёт сама при каждом открытии.
    save_now(cx, &fx);
    assert_eq!(nodes_on_disk(&fx)[0].height, 140.0);
    assert_eq!(nodes_on_disk(&fx)[0].plain_text(), long);
}

#[gpui_kit::test]
fn shrinking_a_card_by_its_corner_works_and_saves_once(cx: &mut TestAppContext) {
    let mut a = card("a", 0.0, 0.0);
    a.set_plain_text("коротко");
    let fx = open_canvas(cx, vec![a]);
    let view = cx.read(|cx| {
        fx.ws
            .read(cx)
            .active_tab()
            .unwrap()
            .canvas_view()
            .unwrap()
            .clone()
    });
    let changes = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = changes.clone();
    let _sub = cx.update(|cx| {
        cx.subscribe(
            &view,
            move |_, _: &bolh_place::canvas_view::CanvasEvent, _| counter.set(counter.get() + 1),
        )
    });
    act(cx, &fx, |w, cx| w.click("node:a", cx));
    act(cx, &fx, |w, cx| {
        let b = w.find("node:a").bounds();
        let corner = point(b.origin.x + b.size.width, b.origin.y + b.size.height);
        w.drag(corner, point(corner.x, corner.y - px(60.)), cx);
    });
    let n = nodes_in_view(cx, &fx)[0].clone();
    assert_eq!(n.height, 80.0, "карточка уменьшилась вслед за уголком");
    assert_eq!(
        changes.get(),
        1,
        "одно изменение на всю протяжку, а не на каждый кадр"
    );
}

#[gpui_kit::test]
fn after_a_conflict_the_canvas_copy_keeps_its_own_id(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    let original_id = {
        let bytes = std::fs::read(fx.root.join(REL)).unwrap();
        Content::parse(FileKind::Canvas, &bytes, Path::new(REL))
            .unwrap()
            .id()
    };
    act(cx, &fx, |w, cx| w.press("ctrl-a", cx));
    act(cx, &fx, |w, cx| w.press("right", cx));
    // Тем временем на другом компьютере холст поменяли по-своему.
    let space = Space::open(&fx.root).unwrap();
    let mut other = space.open_file(Path::new(REL)).unwrap();
    if let Content::Canvas(c) = &mut other.content {
        c.nodes.push(card("чужая", 500.0, 0.0));
    }
    std::fs::write(fx.root.join(REL), other.content.to_bytes()).unwrap();
    save_now(cx, &fx);
    let copy = cx.read(|cx| fx.ws.read(cx).active_tab().unwrap().file.rel.clone());
    assert_ne!(copy, Path::new(REL), "своё ушло в копию");
    // Следующая правка — в копию, и у копии остаётся своя история.
    act(cx, &fx, |w, cx| w.press("right", cx));
    save_now(cx, &fx);
    let bytes = std::fs::read(fx.root.join(&copy)).unwrap();
    let saved = Content::parse(FileKind::Canvas, &bytes, &copy).unwrap();
    assert_ne!(saved.id(), original_id, "у копии свой id");
    let Content::Canvas(c) = saved else { panic!() };
    assert_eq!(c.nodes[0].x, 2.0, "вторая правка тоже в копии");
}

#[gpui_kit::test]
fn own_save_during_a_check_is_not_taken_for_a_change_elsewhere(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.press("ctrl-a", cx));
    act(cx, &fx, |w, cx| w.press("right", cx));
    // Фоновая проверка уже началась, когда вкладку сохранили — например,
    // перед переименованием. На диске она найдёт нашу же запись.
    step(cx, &fx, |ws, window, cx| {
        ws.poll_external(window, cx);
        ws.save_all(window, cx);
    });
    // Холст не перечитан как «пришедший из облака»: отмена на месте.
    act(cx, &fx, |w, cx| w.press("ctrl-z", cx));
    assert_eq!(nodes_in_view(cx, &fx)[0].x, 0.0, "Ctrl+Z работает");
}

#[gpui_kit::test]
fn calm_canvas_does_not_redraw_by_itself(cx: &mut TestAppContext) {
    let mut a = card("a", 0.0, 0.0);
    a.set_plain_text("коротко");
    let mut b = card("b", 300.0, 0.0);
    b.set_plain_text(&"очень длинная строка текста ".repeat(12));
    let fx = open_canvas(cx, vec![a, b]);
    // Первые кадры подгоняют высоту карточки под текст…
    for _ in 0..3 {
        act(cx, &fx, |w, cx| {
            w.simulate_next_frame(cx);
        });
    }
    // …а дальше всё стоит: кадр не просит следующего. Иначе окно
    // перерисовывалось бы без конца и тормозило всё остальное.
    let asked = cx
        .update_window(fx.handle, |_, w, cx| {
            w.render_frame(cx);
            w.simulate_next_frame(cx)
        })
        .unwrap();
    assert_eq!(asked, 0, "спокойный холст сам себя не перерисовывает");
}

fn tool(cx: &mut TestAppContext, fx: &Fx) -> Tool {
    cx.read(|cx| {
        let ws = fx.ws.read(cx);
        ws.active_tab()
            .unwrap()
            .canvas_view()
            .unwrap()
            .read(cx)
            .tool()
    })
}

fn view_of(cx: &mut TestAppContext, fx: &Fx) -> bolh_core::canvas::View {
    cx.read(|cx| {
        let ws = fx.ws.read(cx);
        ws.active_tab()
            .unwrap()
            .canvas_view()
            .unwrap()
            .read(cx)
            .view()
    })
}

#[gpui_kit::test]
fn card_tool_places_a_card_where_clicked(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.click("canvas-tool-card", cx));
    assert_eq!(tool(cx, &fx), Tool::Card);
    // Пустое место у левого верхнего угла поля: карточка — посередине.
    let at = bolh_core::canvas::Point { x: 60.0, y: 60.0 };
    act(cx, &fx, |w, cx| {
        w.click_at("canvas", point(px(at.x as f32), px(at.y as f32)), cx)
    });
    let world = view_of(cx, &fx).to_world(at);
    let nodes = nodes_in_view(cx, &fx);
    assert_eq!(nodes.len(), 2, "карточка поставлена");
    let new = &nodes[1];
    let (cx_, cy_) = (new.x + new.width / 2.0, new.y + new.height / 2.0);
    // Угол карточки встаёт на сетку — середина сдвигается не больше чем на
    // полшага.
    assert!(
        (cx_ - world.x).abs() <= 10.0 && (cy_ - world.y).abs() <= 10.0,
        "карточка там, где щёлкнули: {cx_},{cy_} против {},{}",
        world.x,
        world.y
    );
    assert_eq!(tool(cx, &fx), Tool::Select, "дальше — снова выделение");
    assert!(editing(cx, &fx), "сразу можно печатать");
}

#[gpui_kit::test]
fn hand_tool_moves_the_field_even_over_a_card(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.press("h", cx));
    assert_eq!(tool(cx, &fx), Tool::Hand);
    let v0 = view_of(cx, &fx);
    act(cx, &fx, |w, cx| {
        let from = w.find("node:a").bounds().center();
        w.drag(from, point(from.x + px(50.), from.y + px(30.)), cx);
    });
    let v1 = view_of(cx, &fx);
    assert_eq!(
        (v1.offset_x - v0.offset_x, v1.offset_y - v0.offset_y),
        (50.0, 30.0),
        "поле сдвинулось за мышью"
    );
    assert_eq!(
        nodes_in_view(cx, &fx)[0].x,
        0.0,
        "карточка осталась на месте"
    );
    act(cx, &fx, |w, cx| w.press("v", cx));
    assert_eq!(tool(cx, &fx), Tool::Select);
}

#[gpui_kit::test]
fn escape_puts_the_card_tool_down(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.press("c", cx));
    assert_eq!(tool(cx, &fx), Tool::Card);
    act(cx, &fx, |w, cx| w.press("escape", cx));
    assert_eq!(tool(cx, &fx), Tool::Select);
    act(cx, &fx, |w, cx| {
        w.click_at("canvas", point(px(60.), px(60.)), cx)
    });
    assert_eq!(
        nodes_in_view(cx, &fx).len(),
        1,
        "щелчок больше не ставит карточку"
    );
}

#[gpui_kit::test]
fn canvas_keys_stay_quiet_while_typing_in_a_card(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    // Поле сдвинуто — «показать всё» было бы заметно.
    act(cx, &fx, |w, cx| {
        w.scroll("canvas", ScrollDelta::Pixels(point(px(0.), px(-100.))), cx)
    });
    let v0 = view_of(cx, &fx);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    act(cx, &fx, |w, cx| w.press("shift-1", cx));
    act(cx, &fx, |w, cx| w.press("c", cx));
    assert_eq!(view_of(cx, &fx), v0, "Shift+1 в тексте не показывает всё");
    assert_eq!(
        tool(cx, &fx),
        Tool::Select,
        "буква в тексте не включает инструмент"
    );
    assert!(editing(cx, &fx), "правка текста продолжается");
}

#[gpui_kit::test]
fn toolbar_clicks_do_not_reach_the_field(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.click("node:a", cx));
    act(cx, &fx, |w, cx| w.click("canvas-zoom-in", cx));
    let selected = cx.read(|cx| {
        let ws = fx.ws.read(cx);
        let v = ws.active_tab().unwrap().canvas_view().unwrap().read(cx);
        v.editor().selection().len()
    });
    assert_eq!(selected, 1, "кнопка масштаба не сняла выделение");
    act(cx, &fx, |w, cx| w.click("canvas-tool-card", cx));
    act(cx, &fx, |w, cx| w.click("canvas-tool-hand", cx));
    assert_eq!(
        nodes_in_view(cx, &fx).len(),
        1,
        "щелчок по панели не поставил карточку"
    );
    assert_eq!(tool(cx, &fx), Tool::Hand);
}

#[gpui_kit::test]
fn undo_and_redo_buttons(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.click("node:a", cx));
    act(cx, &fx, |w, cx| w.press("right", cx));
    assert_eq!(nodes_in_view(cx, &fx)[0].x, 1.0);
    act(cx, &fx, |w, cx| w.click("canvas-undo", cx));
    assert_eq!(nodes_in_view(cx, &fx)[0].x, 0.0, "отменено");
    act(cx, &fx, |w, cx| w.click("canvas-redo", cx));
    assert_eq!(nodes_in_view(cx, &fx)[0].x, 1.0, "повторено");
}

/// Печатает по одному знаку, как человек: быстрые «- » и «[] » срабатывают
/// на пробеле.
fn type_chars(cx: &mut TestAppContext, fx: &Fx, text: &str) {
    for ch in text.chars() {
        let ch = ch.to_string();
        act(cx, fx, |w, cx| w.input(&ch, cx));
    }
}

fn rich_in_view(cx: &mut TestAppContext, fx: &Fx, i: usize) -> RichText {
    nodes_in_view(cx, fx)[i].rich.clone()
}

fn kinds(r: &RichText) -> Vec<Kind> {
    r.blocks.iter().map(|b| b.kind).collect()
}

#[gpui_kit::test]
fn checklist_typed_with_brackets_is_saved_with_its_blocks(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    type_chars(cx, &fx, "# План");
    act(cx, &fx, |w, cx| w.press("enter", cx));
    type_chars(cx, &fx, "[] молоко");
    act(cx, &fx, |w, cx| w.press("enter", cx));
    type_chars(cx, &fx, "хлеб");
    act(cx, &fx, |w, cx| w.press("escape", cx));
    let r = rich_in_view(cx, &fx, 0);
    assert_eq!(
        kinds(&r),
        [Kind::Heading(1), Kind::Check(false), Kind::Check(false)]
    );
    assert_eq!(r.plain_text(), "План\nмолоко\nхлеб");
    save_now(cx, &fx);
    let disk = nodes_on_disk(&fx);
    assert_eq!(disk[0].rich, r, "в файле то же самое");
    let json = std::fs::read_to_string(fx.root.join(REL)).unwrap();
    assert!(json.contains("\"blocks\""), "оформление в файле");
    assert!(json.contains("\"version\": 2"), "холст новой версии");
}

#[gpui_kit::test]
fn plain_typing_keeps_the_file_plain(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    type_chars(cx, &fx, "просто");
    act(cx, &fx, |w, cx| w.press("escape", cx));
    save_now(cx, &fx);
    let json = std::fs::read_to_string(fx.root.join(REL)).unwrap();
    assert!(json.contains("\"text\": \"просто\""));
    assert!(
        !json.contains("\"blocks\""),
        "без оформления — только текст"
    );
}

#[gpui_kit::test]
fn bold_by_keys_and_italic_from_the_format_bar(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    type_chars(cx, &fx, "важно");
    act(cx, &fx, |w, cx| w.press("ctrl-a", cx));
    act(cx, &fx, |w, cx| w.press("ctrl-b", cx));
    act(cx, &fx, |w, cx| w.click("canvas-italic", cx));
    let r = rich_in_view(cx, &fx, 0);
    let span = &r.blocks[0].spans[0];
    assert_eq!(span.text, "важно");
    assert!(span.marks.bold && span.marks.italic, "{:?}", span.marks);
    assert!(editing(cx, &fx), "кнопка панели не прервала правку");
    // Дальше набор идёт тем же оформлением.
    act(cx, &fx, |w, cx| w.press("end", cx));
    type_chars(cx, &fx, "!");
    let r = rich_in_view(cx, &fx, 0);
    assert_eq!(r.blocks[0].spans.len(), 1, "«!» тоже жирный курсив");
    // Кнопка списка с панели.
    act(cx, &fx, |w, cx| w.click("canvas-checklist", cx));
    assert_eq!(kinds(&rich_in_view(cx, &fx, 0)), [Kind::Check(false)]);
}

fn checklist_card(id: &str, items: &[(&str, bool)]) -> Node {
    let mut n = card(id, 0.0, 0.0);
    n.rich = RichText::from_blocks(
        items
            .iter()
            .map(|(t, done)| Block::new(Kind::Check(*done)).with_text(*t))
            .collect(),
    );
    n
}

#[gpui_kit::test]
fn clicking_a_checkbox_ticks_it_without_editing_and_undo_unticks(cx: &mut TestAppContext) {
    let fx = open_canvas(
        cx,
        vec![checklist_card("a", &[("молоко", false), ("хлеб", false)])],
    );
    act(cx, &fx, |_, _| {});
    // Квадрат первого пункта: рамка 1 + отступ 12, по высоте — середина
    // первой строки.
    act(cx, &fx, |w, cx| {
        w.click_at("node:a", point(px(21.), px(24.)), cx)
    });
    let r = rich_in_view(cx, &fx, 0);
    assert_eq!(kinds(&r), [Kind::Check(true), Kind::Check(false)]);
    assert!(!editing(cx, &fx), "галочка не начинает правку");
    let selected = cx.read(|cx| {
        let ws = fx.ws.read(cx);
        let v = ws.active_tab().unwrap().canvas_view().unwrap().read(cx);
        v.editor().selection().len()
    });
    assert_eq!(selected, 0, "и не выделяет карточку");
    save_now(cx, &fx);
    assert_eq!(kinds(&nodes_on_disk(&fx)[0].rich)[0], Kind::Check(true));
    act(cx, &fx, |w, cx| w.press("ctrl-z", cx));
    assert_eq!(kinds(&rich_in_view(cx, &fx, 0))[0], Kind::Check(false));
}

#[gpui_kit::test]
fn checklist_tool_places_a_card_with_an_empty_item(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.press("l", cx));
    assert_eq!(tool(cx, &fx), Tool::Checklist);
    act(cx, &fx, |w, cx| {
        w.click_at("canvas", point(px(60.), px(60.)), cx)
    });
    assert!(editing(cx, &fx));
    type_chars(cx, &fx, "купить");
    act(cx, &fx, |w, cx| w.press("enter", cx));
    type_chars(cx, &fx, "позвонить");
    let r = rich_in_view(cx, &fx, 1);
    assert_eq!(kinds(&r), [Kind::Check(false), Kind::Check(false)]);
    assert_eq!(r.plain_text(), "купить\nпозвонить");
    assert_eq!(tool(cx, &fx), Tool::Select);
}

#[gpui_kit::test]
fn double_click_puts_the_caret_where_clicked(cx: &mut TestAppContext) {
    let mut a = card("a", 0.0, 0.0);
    a.set_plain_text("раз два");
    let fx = open_canvas(cx, vec![a]);
    act(cx, &fx, |_, _| {});
    // У самого начала текста.
    act(cx, &fx, |w, cx| {
        let at = ClickOptions::new()
            .with_offset(point(px(14.), px(24.)))
            .with_count(2);
        w.click_with_options("node:a", at, cx)
    });
    type_chars(cx, &fx, "X");
    assert_eq!(rich_in_view(cx, &fx, 0).plain_text(), "Xраз два");
}

#[gpui_kit::test]
fn copy_and_paste_inside_text_keep_formatting(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    type_chars(cx, &fx, "ж");
    act(cx, &fx, |w, cx| w.press("ctrl-a", cx));
    act(cx, &fx, |w, cx| w.press("ctrl-b", cx));
    act(cx, &fx, |w, cx| w.press("ctrl-c", cx));
    act(cx, &fx, |w, cx| w.press("ctrl-end", cx));
    act(cx, &fx, |w, cx| w.press("ctrl-b", cx));
    type_chars(cx, &fx, " о ");
    act(cx, &fx, |w, cx| w.press("ctrl-v", cx));
    let r = rich_in_view(cx, &fx, 0);
    assert_eq!(r.plain_text(), "ж о ж");
    let bold: Vec<bool> = r.blocks[0].spans.iter().map(|s| s.marks.bold).collect();
    assert_eq!(bold, [true, false, true], "вставленное — жирное");
}

#[gpui_kit::test]
fn undo_inside_text_then_whole_edit_on_the_canvas(cx: &mut TestAppContext) {
    let mut a = card("a", 0.0, 0.0);
    a.set_plain_text("было");
    let fx = open_canvas(cx, vec![a]);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    act(cx, &fx, |w, cx| w.press("ctrl-end", cx));
    type_chars(cx, &fx, "abc");
    act(cx, &fx, |w, cx| w.press("ctrl-z", cx));
    assert_eq!(
        rich_in_view(cx, &fx, 0).plain_text(),
        "было",
        "отмена внутри текста"
    );
    type_chars(cx, &fx, "x");
    act(cx, &fx, |w, cx| w.press("escape", cx));
    assert_eq!(rich_in_view(cx, &fx, 0).plain_text(), "былоx");
    // Вне правки Ctrl+Z отменяет всю правку текста разом.
    act(cx, &fx, |w, cx| w.press("ctrl-z", cx));
    assert_eq!(rich_in_view(cx, &fx, 0).plain_text(), "было");
}

#[gpui_kit::test]
fn tab_nests_a_list_item_and_numbers_follow(cx: &mut TestAppContext) {
    let fx = open_canvas(cx, vec![card("a", 0.0, 0.0)]);
    act(cx, &fx, |w, cx| w.double_click("node:a", cx));
    type_chars(cx, &fx, "1. раз");
    act(cx, &fx, |w, cx| w.press("enter", cx));
    type_chars(cx, &fx, "вложенный");
    act(cx, &fx, |w, cx| w.press("tab", cx));
    act(cx, &fx, |w, cx| w.press("enter", cx));
    act(cx, &fx, |w, cx| w.press("shift-tab", cx));
    type_chars(cx, &fx, "два");
    let r = rich_in_view(cx, &fx, 0);
    let indents: Vec<u8> = r.blocks.iter().map(|b| b.indent).collect();
    assert_eq!(indents, [0, 1, 0]);
    assert_eq!(r.numbers(), [Some(1), Some(1), Some(2)]);
}

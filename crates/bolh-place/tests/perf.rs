//! Замеры скорости холста (раздел 11: 5000 объектов — плавно, 60 кадров в
//! секунду, то есть кадр не дольше 16 мс). Это не проверка «прошло/не
//! прошло», а цифры для сравнения до и после правок. Запуск:
//!
//! ```text
//! cargo test --release -p bolh-place --test perf -- --ignored --nocapture
//! ```
//!
//! Окно без экрана считает раскладку и строит кадр — это и есть работа
//! процессора; растеризацию делает видеокарта, её здесь нет.

use std::path::Path;
use std::time::{Duration, Instant};

use bolh_core::format::{Content, FileKind, Node};
use bolh_core::settings::Settings;
use bolh_core::space::{Clock, Space};
use bolh_place::workspace::Workspace;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, ScrollDelta, TestAppContext, WindowBounds,
    WindowOptions, point, px, size,
};

const REL: &str = "Большой.bcanvas";

fn open_big(
    cx: &mut TestAppContext,
    n: usize,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Работа");
    let space = Space::create(&root).unwrap();
    space
        .create_file(Path::new(""), "Большой", FileKind::Canvas, Clock::system())
        .unwrap();
    let mut f = space.open_file(Path::new(REL)).unwrap();
    if let Content::Canvas(c) = &mut f.content {
        let side = (n as f64).sqrt().ceil() as usize;
        for i in 0..n {
            let (col, row) = (i % side, i / side);
            let mut card = Node::card(
                format!("n{i}"),
                col as f64 * 280.0,
                row as f64 * 180.0,
                240.0,
                140.0,
            );
            card.set_plain_text(&format!("Карточка {i}: купить, позвонить, сделать"));
            c.nodes.push(card);
        }
    }
    space.save(&mut f, "x", Clock::system()).unwrap();
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
                    size(px(1600.), px(1000.)),
                ))),
                ..Default::default()
            },
            cx,
            move |window, cx| cx.new(|cx| Workspace::new(settings, Some(path), window, cx)),
        )
        .unwrap()
    });
    cx.run_until_parked();
    let ws2 = ws.clone();
    cx.update_window(handle, |_, window, cx| {
        ws2.update(cx, |ws, cx| ws.open_file(Path::new(REL), window, cx));
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    (dir, handle, ws)
}

fn report(name: &str, mut t: Vec<Duration>) {
    t.sort();
    let avg = t.iter().sum::<Duration>() / t.len() as u32;
    let p95 = t[t.len() * 95 / 100];
    eprintln!(
        "{name}: кадров {}, среднее {:.2} мс, 95% {:.2} мс, худший {:.2} мс",
        t.len(),
        avg.as_secs_f64() * 1e3,
        p95.as_secs_f64() * 1e3,
        t.last().unwrap().as_secs_f64() * 1e3
    );
}

fn pan_frames(cx: &mut TestAppContext, handle: AnyWindowHandle, frames: usize) -> Vec<Duration> {
    let mut times = Vec::new();
    cx.update_window(handle, |_, w, cx| {
        for i in 0..frames {
            let d = if i % 2 == 0 { -40. } else { 40. };
            let t = Instant::now();
            w.scroll("canvas", ScrollDelta::Pixels(point(px(d), px(0.))), cx);
            times.push(t.elapsed());
        }
    })
    .unwrap();
    times
}

#[gpui_kit::test]
#[ignore]
fn perf_resize_card_among_40(cx: &mut TestAppContext) {
    let (_d, handle, _ws) = open_big(cx, 40);
    let mut times = Vec::new();
    cx.update_window(handle, |_, w, cx| {
        w.click("node:n0", cx);
        let b = w.find("node:n0").bounds();
        let corner = point(b.origin.x + b.size.width, b.origin.y + b.size.height);
        for step in 0..60 {
            let off = px((step % 30) as f32 * 3.0);
            let t = Instant::now();
            w.drag(corner, point(corner.x + off, corner.y - off), cx);
            times.push(t.elapsed() / 10);
        }
    })
    .unwrap();
    report("размер карточки среди 40 (на один шаг мыши)", times);
}

#[gpui_kit::test]
#[ignore]
fn perf_pan_5000_at_100(cx: &mut TestAppContext) {
    let (_d, handle, _ws) = open_big(cx, 5000);
    cx.update_window(handle, |_, w, cx| {
        w.press("ctrl-0", cx);
    })
    .unwrap();
    let t = pan_frames(cx, handle, 60);
    report("5000 объектов, масштаб 100%, сдвиг", t);
}

#[gpui_kit::test]
#[ignore]
fn perf_pan_5000_all_visible(cx: &mut TestAppContext) {
    let (_d, handle, _ws) = open_big(cx, 5000);
    // Открытый холст сразу показан целиком — все 5000 видны.
    let t = pan_frames(cx, handle, 30);
    report("5000 объектов, видны все, сдвиг", t);
}

// Без чёрного окна консоли рядом с программой в Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    bolh_place::run();
}

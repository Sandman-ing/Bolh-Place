//! Главное окно: боковая панель с пространством и деревом файлов, вкладки,
//! содержимое открытого файла и строка состояния.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bolh_core::format::{Content, FileKind};
use bolh_core::settings::{Settings, ThemeChoice};
use bolh_core::space::{Clock, Entry, EntryKind, External, OpenFile, Probe, SaveOutcome, Space};
use bolh_core::trash::TrashEntry;
use bolh_core::update::Release;
use futures::FutureExt as _;
use futures::future::Shared;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::{Notification, NotificationType};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tree::{TreeItem, TreeState, tree};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IndexPath, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, PathPromptOptions, Render, SharedString, Styled, Subscription,
    Task, Window, div, prelude::FluentBuilder as _, px,
};

use crate::actions::{
    self, CloseTab, NewCanvas, NewDocument, NewFolder, NextTab, OpenPalette, PrevTab, Save,
};
use crate::canvas_view::{CanvasEvent, CanvasView};
use crate::i18n::{error_text, set_language, tr, trf};
use crate::palette::{self, Command as PaletteCommand};
use crate::{paths, theme, updater};

/// Через сколько после последней правки файл сохраняется сам.
const AUTOSAVE_DELAY: Duration = Duration::from_millis(800);
/// Как часто смотреть, не изменили ли файлы через облако.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Открытая вкладка.
pub struct OpenTab {
    id: u64,
    /// Файл, каким его видели на диске. Содержимое холста сюда попадает
    /// только перед сохранением: копировать весь холст на каждую правку
    /// незачем.
    pub file: OpenFile,
    /// Поле заглушки редактора — только у документов.
    editor: Option<Entity<TextareaState>>,
    /// Холст — только у холстов.
    canvas: Option<Entity<CanvasView>>,
    /// О чужой правке уже сказали — не повторять каждые две секунды.
    external_noticed: bool,
    /// Сколько правок сделано во вкладке и по какую из них всё на диске.
    /// «Не сохранено» — их неравенство: значок на вкладке проверяется в
    /// каждом кадре, и сравнивать там весь холст с сохранённым дорого.
    edits: u64,
    saved_edits: u64,
    /// Отложенное автосохранение. У каждой вкладки своё: правка в одной
    /// вкладке не должна отменять сохранение другой.
    autosave: Option<Task<()>>,
    /// Сохранение, которое сейчас идёт в фоне. Второе одновременно не
    /// начинается: два писателя одного файла приняли бы запись друг друга
    /// за чужую правку и наплодили копий.
    saving: Option<Saving>,
    /// Пока шло сохранение, подошло время следующего.
    save_again: bool,
    _subscriptions: Vec<Subscription>,
}

/// Сохранение в фоне. Итог кладёт фоновая задача, а забирает тот, кто
/// дождётся первым: обычный ход событий или закрытие окна, которому
/// ждать своей очереди некогда.
struct Saving {
    seq: u64,
    done: Arc<Mutex<Option<SaveDone>>>,
    job: Shared<Task<()>>,
}

/// Чем кончилось сохранение копии файла.
struct SaveDone {
    file: OpenFile,
    /// По какую правку включительно сохранено.
    edits: u64,
    result: bolh_core::Result<SaveOutcome>,
}

/// Сохраняет копию открытого файла — годится для другого потока.
fn save_copy(space: &Space, mut file: OpenFile, edits: u64, conflict_label: &str) -> SaveDone {
    let result = space.save(&mut file, conflict_label, Clock::system());
    SaveDone {
        file,
        edits,
        result,
    }
}

fn conflict_label(cx: &App) -> String {
    crate::i18n::current(cx).t("names.conflict").to_string()
}

impl OpenTab {
    pub fn canvas_view(&self) -> Option<&Entity<CanvasView>> {
        self.canvas.as_ref()
    }

    /// Есть правки, которых ещё нет на диске.
    pub fn unsaved(&self) -> bool {
        self.edits != self.saved_edits
    }

    /// Файл со всеми правками — то, что пора записать, и номер последней
    /// правки в нём.
    fn draft(&mut self, cx: &App) -> (OpenFile, u64) {
        if let Some(view) = &self.canvas {
            self.file.content = Content::Canvas(view.read(cx).canvas());
        }
        (self.file.clone(), self.edits)
    }

    pub fn title(&self) -> String {
        self.file
            .rel
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Состояние проверки обновлений, как его видит человек.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    Available(Release),
    Downloading(Release),
    Failed(SharedString),
}

pub struct Workspace {
    settings: Settings,
    settings_path: Option<PathBuf>,
    space: Option<Space>,
    tree: Vec<Entry>,
    tree_state: Entity<TreeState>,
    /// Папка, в которой создаются новые файлы: выбранная в дереве.
    target_folder: PathBuf,
    tabs: Vec<OpenTab>,
    active: Option<usize>,
    next_tab_id: u64,
    palette: Entity<CommandState>,
    name_input: Entity<InputState>,
    new_space_parent: PathBuf,
    status: SharedString,
    pub update: UpdateStatus,
    focus_handle: FocusHandle,
    update_task: Option<Task<()>>,
    /// Номер последнего начатого сохранения — чтобы итог старого не принять
    /// за итог нового.
    save_seq: u64,
    /// Проверка файлов на чужие правки идёт в фоне — вторую не начинать.
    polling: bool,
    /// Меняется при каждом перечитывании дерева здесь же. Дерево из фоновой
    /// проверки, начатой раньше, устарело: в нём нет только что созданного.
    tree_version: u64,
    _poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Workspace {
    pub fn new(
        settings: Settings,
        settings_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tree_state = cx.new(|cx| TreeState::new(cx));
        let palette = cx.new(|cx| CommandState::new(window, cx));
        let name_input = cx.new(|cx| InputState::new(window, cx));
        let mut subscriptions = Vec::new();
        // «Как в Windows»: тема следует за системой и после запуска.
        subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            if this.settings.theme == ThemeChoice::System {
                theme::apply(ThemeChoice::System, window, cx);
            }
        }));
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            // Правки не теряются при закрытии окна.
            let _ = weak.update(cx, |this, cx| this.save_all(window, cx));
            true
        });

        let mut this = Workspace {
            settings,
            settings_path,
            space: None,
            tree: Vec::new(),
            tree_state,
            target_folder: PathBuf::new(),
            tabs: Vec::new(),
            active: None,
            next_tab_id: 1,
            palette,
            name_input,
            new_space_parent: paths::default_spaces_dir(),
            status: SharedString::default(),
            update: UpdateStatus::Idle,
            focus_handle: cx.focus_handle(),
            update_task: None,
            save_seq: 0,
            polling: false,
            tree_version: 0,
            _poll: None,
            _subscriptions: subscriptions,
        };
        theme::apply(this.settings.theme, window, cx);
        if let Some(root) = this.settings.active.clone() {
            this.open_space(&root, window, cx);
        }
        this.focus_handle.focus(window, cx);
        this._poll = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                if this
                    .update_in(cx, |this, window, cx| this.poll_external(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        this
    }

    // ---- Доступ для тестов и палитры -------------------------------------

    pub fn space(&self) -> Option<&Space> {
        self.space.as_ref()
    }

    pub fn tabs(&self) -> &[OpenTab] {
        &self.tabs
    }

    pub fn active_tab(&self) -> Option<&OpenTab> {
        self.active.and_then(|i| self.tabs.get(i))
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn status(&self) -> &SharedString {
        &self.status
    }

    // ---- Настройки ------------------------------------------------------

    fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = &self.settings_path
            && let Err(e) = self.settings.save(path)
        {
            self.notify_error(&e, window, cx);
        }
    }

    pub fn set_theme(&mut self, choice: ThemeChoice, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.theme = choice;
        theme::apply(choice, window, cx);
        self.save_settings(window, cx);
        cx.notify();
    }

    pub fn set_language(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.language = Some(code.to_string());
        set_language(code, cx);
        window.set_window_title(&tr(cx, "app.title"));
        self.save_settings(window, cx);
        cx.refresh_windows();
    }

    // ---- Сообщения ------------------------------------------------------

    fn notify(
        &mut self,
        kind: NotificationType,
        text: SharedString,
        sticky: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.push_notification(
            Notification::new()
                .message(text)
                .with_type(kind)
                .autohide(!sticky),
            cx,
        );
    }

    fn notify_error(
        &mut self,
        err: &bolh_core::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = error_text(cx, err);
        self.notify(NotificationType::Error, text, true, window, cx);
    }

    // ---- Пространства ---------------------------------------------------

    /// Открывает пространство (папка без `.bolh` становится пространством).
    pub fn open_space(&mut self, root: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if !root.is_dir() {
            let name = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = trf(cx, "space.missing", &[("name", &name)]);
            self.notify(NotificationType::Warning, text, true, window, cx);
            return;
        }
        let space = match Space::create(root) {
            Ok(s) => s,
            Err(e) => return self.notify_error(&e, window, cx),
        };
        self.save_all(window, cx);
        self.tabs.clear();
        self.active = None;
        self.target_folder = PathBuf::new();
        self.space = Some(space);
        self.settings.add_space(root);
        self.save_settings(window, cx);
        self.refresh_tree(cx);
        cx.notify();
    }

    pub fn forget_space(&mut self, root: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let is_current = self.space.as_ref().is_some_and(|s| s.root() == root);
        self.settings.forget_space(root);
        if is_current {
            self.save_all(window, cx);
            self.tabs.clear();
            self.active = None;
            self.space = None;
            self.tree.clear();
            self.refresh_tree(cx);
            if let Some(next) = self.settings.active.clone() {
                self.open_space(&next, window, cx);
            }
        }
        self.save_settings(window, cx);
        cx.notify();
    }

    /// Создаёт пространство `parent/name` и открывает его.
    pub fn create_space(
        &mut self,
        parent: &Path,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let name = name.trim();
        if !bolh_core::fsutil::validate_name(name) {
            let e = bolh_core::Error::InvalidName(name.into());
            self.notify_error(&e, window, cx);
            return false;
        }
        let root = parent.join(name);
        match Space::create(&root) {
            Ok(_) => {
                self.open_space(&root, window, cx);
                true
            }
            Err(e) => {
                self.notify_error(&e, window, cx);
                false
            }
        }
    }

    fn prompt_folder(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, PathBuf, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr(cx, "space.choose_folder")),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await
                && let Some(path) = paths.pop()
            {
                let _ = this.update_in(cx, |this, window, cx| then(this, path, window, cx));
            }
        })
        .detach();
    }

    pub fn open_space_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_folder(window, cx, |this, path, window, cx| {
            this.open_space(&path, window, cx)
        });
    }

    pub fn new_space_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let default = tr(cx, "space.default_name");
        self.name_input
            .update(cx, |s, cx| s.set_value(default, window, cx));
        let weak = cx.entity().downgrade();
        let input = self.name_input.clone();
        let first = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                input.focus_handle(cx).focus(window, cx);
            }
            let parent = weak
                .read_with(cx, |this, _| this.new_space_parent.clone())
                .unwrap_or_default();
            let (w1, w2, i2) = (weak.clone(), weak.clone(), input.clone());
            dialog
                .title(tr(cx, "space.new"))
                .w(px(520.))
                .child(
                    v_flex().gap_3().child(Input::new(&input)).child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(parent.to_string_lossy().into_owned()),
                            )
                            .child(
                                Button::new("new-space-parent")
                                    .small()
                                    .label(tr(cx, "space.choose_parent"))
                                    .on_click(move |_, window, cx| {
                                        let _ = w1.update(cx, |this, cx| {
                                            this.prompt_folder(window, cx, |this, p, _, cx| {
                                                this.new_space_parent = p;
                                                cx.notify();
                                            });
                                        });
                                    }),
                            ),
                    ),
                )
                .footer({
                    h_flex().gap_2().justify_end().children([
                        Button::new("new-space-create")
                            .primary()
                            .label(tr(cx, "common.create"))
                            .on_click(move |_, window, cx| {
                                let name = i2.read(cx).value().to_string();
                                let ok = w2
                                    .update(cx, |this, cx| {
                                        let parent = this.new_space_parent.clone();
                                        this.create_space(&parent, &name, window, cx)
                                    })
                                    .unwrap_or(false);
                                if ok {
                                    window.close_dialog(cx);
                                }
                            })
                            .into_any_element(),
                        Button::new("new-space-cancel")
                            .label(tr(cx, "common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx))
                            .into_any_element(),
                    ])
                })
        });
    }

    // ---- Дерево файлов --------------------------------------------------

    /// Перечитывает дерево с диска сейчас же — после своих правок: новый
    /// файл должен появиться в дереве сразу, а не через две секунды.
    pub fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        self.tree_version += 1;
        let entries = match &self.space {
            Some(s) => s.tree().unwrap_or_default(),
            None => Vec::new(),
        };
        self.set_tree(entries, cx);
    }

    /// Показывает дерево, сохраняя раскрытые папки и выбор.
    fn set_tree(&mut self, entries: Vec<Entry>, cx: &mut Context<Self>) {
        if entries == self.tree && !entries.is_empty() {
            return;
        }
        let state = self.tree_state.read(cx);
        let mut expanded = Vec::new();
        let mut ix = 0;
        while let Some(e) = state.entry(ix) {
            if e.is_folder() && e.is_expanded() {
                expanded.push(e.item().id.clone());
            }
            ix += 1;
        }
        let selected = state.selected_item().map(|i| i.id.clone());
        let items = to_tree_items(&entries, &expanded);
        self.tree = entries;
        self.tree_state.update(cx, |s, cx| {
            s.set_items(items, cx);
            if let Some(id) = selected
                && let Some(ix) = s.index_of(&id)
            {
                s.set_selected_index(Some(ix), cx);
            }
        });
        cx.notify();
    }

    fn on_tree_click(
        &mut self,
        rel: PathBuf,
        is_file: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if is_file {
            self.target_folder = rel.parent().map(Path::to_path_buf).unwrap_or_default();
            self.open_file(&rel, window, cx);
        } else {
            self.target_folder = rel;
        }
    }

    // ---- Файлы и вкладки ------------------------------------------------

    fn clock() -> Clock {
        Clock::system()
    }

    pub fn create_file(&mut self, kind: FileKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = &self.space else { return };
        let name = match kind {
            FileKind::Document => tr(cx, "names.document"),
            FileKind::Canvas => tr(cx, "names.canvas"),
        };
        let folder = if space.root().join(&self.target_folder).is_dir() {
            self.target_folder.clone()
        } else {
            PathBuf::new()
        };
        match space.create_file(&folder, &name, kind, Self::clock()) {
            Ok(rel) => {
                self.refresh_tree(cx);
                self.open_file(&rel, window, cx);
            }
            Err(e) => self.notify_error(&e, window, cx),
        }
    }

    pub fn create_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = &self.space else { return };
        let name = tr(cx, "names.folder");
        let folder = if space.root().join(&self.target_folder).is_dir() {
            self.target_folder.clone()
        } else {
            PathBuf::new()
        };
        match space.create_folder(&folder, &name) {
            Ok(rel) => {
                self.target_folder = rel;
                self.refresh_tree(cx);
            }
            Err(e) => self.notify_error(&e, window, cx),
        }
    }

    /// Открывает файл во вкладке; если он уже открыт — переключается на неё.
    pub fn open_file(&mut self, rel: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.file.rel == rel) {
            self.activate(ix, window, cx);
            return;
        }
        let Some(space) = &self.space else { return };
        let file = match space.open_file(rel) {
            Ok(f) => f,
            Err(e) => return self.notify_error(&e, window, cx),
        };
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut subscriptions = Vec::new();
        let editor = match &file.content {
            Content::Document(doc) => {
                let text = doc.plain_text();
                let placeholder = tr(cx, "document.placeholder");
                let state = cx.new(|cx| {
                    let mut s = TextareaState::new(window, cx).placeholder(placeholder);
                    s.set_value(text, window, cx);
                    s
                });
                subscriptions.push(cx.subscribe_in(
                    &state,
                    window,
                    move |this, state, ev, window, cx| {
                        if matches!(ev, InputEvent::Change) {
                            let text = state.read(cx).value().to_string();
                            this.on_document_edited(id, &text, window, cx);
                        }
                    },
                ));
                Some(state)
            }
            Content::Canvas(_) => None,
        };
        let canvas = match &file.content {
            Content::Canvas(c) => {
                let (c, read_only) = (c.clone(), !file.content.is_writable());
                let view = cx.new(|cx| CanvasView::new(c, read_only, cx));
                subscriptions.push(cx.subscribe_in(
                    &view,
                    window,
                    move |this, _, ev, window, cx| match ev {
                        CanvasEvent::Changed => this.on_canvas_edited(id, window, cx),
                    },
                ));
                Some(view)
            }
            Content::Document(_) => None,
        };
        self.tabs.push(OpenTab {
            id,
            file,
            editor,
            canvas,
            external_noticed: false,
            edits: 0,
            saved_edits: 0,
            autosave: None,
            saving: None,
            save_again: false,
            _subscriptions: subscriptions,
        });
        let ix = self.tabs.len() - 1;
        self.activate(ix, window, cx);
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.active = Some(ix);
        // Фокус — в поле документа, а у холста — на само окно: иначе
        // фокус остался бы на закрытой вкладке, и клавиши бы не работали.
        let tab = &self.tabs[ix];
        match (&tab.editor, &tab.canvas) {
            (Some(editor), _) => editor.focus_handle(cx).focus(window, cx),
            (_, Some(canvas)) => canvas.focus_handle(cx).focus(window, cx),
            _ => self.focus_handle.focus(window, cx),
        }
        cx.notify();
    }

    fn on_document_edited(
        &mut self,
        id: u64,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let Content::Document(doc) = &mut tab.file.content else {
            return;
        };
        if doc.plain_text() == text {
            return;
        }
        doc.set_plain_text(text);
        Self::edited(tab, window, cx);
    }

    fn on_canvas_edited(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            Self::edited(tab, window, cx);
        }
    }

    /// Во вкладке правка: запомнить и сохранить, когда человек остановится.
    fn edited(tab: &mut OpenTab, window: &mut Window, cx: &mut Context<Self>) {
        tab.edits += 1;
        Self::schedule_autosave(tab, window, cx);
        cx.notify();
    }

    fn schedule_autosave(tab: &mut OpenTab, window: &mut Window, cx: &mut Context<Self>) {
        let id = tab.id;
        tab.autosave = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(AUTOSAVE_DELAY).await;
            let _ = this.update_in(cx, |this, window, cx| this.save_tab(id, window, cx));
        }));
    }

    /// Показывает в поле документа или на холсте то, что сейчас в файле
    /// вкладки, — после правки из облака или отката к снимку.
    fn show_file_content(tab: &OpenTab, window: &mut Window, cx: &mut Context<Self>) {
        match (&tab.file.content, &tab.editor, &tab.canvas) {
            (Content::Document(doc), Some(editor), _) => {
                let text = doc.plain_text();
                editor.update(cx, |s, cx| s.set_value(text, window, cx));
            }
            (Content::Canvas(c), _, Some(view)) => {
                let c = c.clone();
                view.update(cx, |v, cx| v.replace_canvas(c, cx));
            }
            _ => {}
        }
    }

    /// Начинает сохранение вкладки в фоне: запись на диск, особенно в
    /// облачную папку, бывает долгой, и окно не должно её ждать. Ядро само
    /// решает, не было ли чужой правки, и при расхождении сохраняет обе
    /// версии.
    pub fn save_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.space.clone() else {
            return;
        };
        let label = conflict_label(cx);
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if !tab.unsaved() || !tab.file.content.is_writable() {
            return;
        }
        if tab.saving.is_some() {
            tab.save_again = true;
            return;
        }
        let (file, edits) = tab.draft(cx);
        let done = Arc::new(Mutex::new(None));
        let slot = done.clone();
        let job = cx
            .background_spawn(async move {
                let saved = save_copy(&space, file, edits, &label);
                *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(saved);
            })
            .shared();
        self.save_seq += 1;
        let seq = self.save_seq;
        let wait = job.clone();
        cx.spawn_in(window, async move |this, cx| {
            wait.await;
            let _ = this.update_in(cx, |this, window, cx| this.finish_save(id, seq, window, cx));
        })
        .detach();
        tab.saving = Some(Saving { seq, done, job });
    }

    /// Фоновое сохранение кончилось. Итог мог уже забрать `flush_tab`.
    fn finish_save(&mut self, id: u64, seq: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if tab.saving.as_ref().map(|s| s.seq) != Some(seq) {
            return;
        }
        let saving = tab.saving.take().expect("проверено выше");
        let again = std::mem::take(&mut tab.save_again);
        let done = saving
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(done) = done {
            self.apply_save(id, done, window, cx);
        }
        if again {
            self.save_tab(id, window, cx);
        }
    }

    /// Сохраняет вкладку сейчас же и ждёт записи — перед закрытием окна или
    /// вкладки, переименованием, удалением, сменой пространства. Если
    /// сохранение уже идёт в фоне, сначала дождаться его: иначе два
    /// писателя одного файла приняли бы запись друг друга за чужую.
    pub fn flush_tab(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        tab.autosave = None;
        tab.save_again = false;
        if let Some(saving) = tab.saving.take() {
            cx.foreground_executor().clone().block_on(saving.job);
            let done = saving
                .done
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(done) = done {
                self.apply_save(id, done, window, cx);
            }
        }
        let Some(space) = self.space.clone() else {
            return;
        };
        let label = conflict_label(cx);
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if !tab.unsaved() || !tab.file.content.is_writable() {
            return;
        }
        let (file, edits) = tab.draft(cx);
        let done = save_copy(&space, file, edits, &label);
        self.apply_save(id, done, window, cx);
    }

    /// Принимает итог сохранения во вкладку.
    fn apply_save(&mut self, id: u64, done: SaveDone, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let name = tab.title();
        let SaveDone {
            file,
            edits,
            result,
        } = done;
        tab.file.adopt_saved(file);
        // У холста id живёт в редакторе: после расхождения у копии он новый,
        // и следующее сохранение не должно вернуть копии чужой id.
        if let Some(view) = &tab.canvas {
            let new_id = tab.file.content.id();
            view.update(cx, |v, _| v.set_canvas_id(new_id));
        }
        if result.is_ok() {
            tab.saved_edits = tab.saved_edits.max(edits);
            tab.external_noticed = false;
        }
        match result {
            Ok(SaveOutcome::Saved) => self.status = tr(cx, "sync.saved"),
            Ok(SaveOutcome::Conflict { copy, .. }) => {
                let copy = copy
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let text = trf(cx, "sync.conflict", &[("name", &name), ("copy", &copy)]);
                self.notify(NotificationType::Warning, text, true, window, cx);
                self.refresh_tree(cx);
            }
            Err(e) => self.notify_error(&e, window, cx),
        }
        cx.notify();
    }

    pub fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active_tab().map(|t| t.id) {
            self.save_tab(id, window, cx);
        }
    }

    /// Сохраняет все вкладки и ждёт записи.
    pub fn save_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.tabs.iter().map(|t| t.id).collect();
        for id in ids {
            self.flush_tab(id, window, cx);
        }
    }

    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.get(ix).map(|t| t.id) else {
            return;
        };
        self.flush_tab(id, window, cx);
        self.tabs.remove(ix);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(a) if a > ix => Some(a - 1),
            Some(a) if a == ix => Some(ix.min(self.tabs.len() - 1)),
            other => other,
        };
        if let Some(a) = self.active {
            self.activate(a, window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    /// Проверяет открытые файлы и дерево на правки, пришедшие снаружи.
    /// Чтение диска — в фоне: в облачной папке с тысячами файлов обход
    /// дерева раз в две секунды заметно подтормаживал окно.
    pub fn poll_external(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.polling {
            return;
        }
        let Some(space) = self.space.clone() else {
            return;
        };
        // Файл, который сейчас сохраняется, проверять бессмысленно: на
        // диске вот-вот будет своя же запись.
        let probes: Vec<(u64, Probe)> = self
            .tabs
            .iter()
            .filter(|t| t.saving.is_none())
            .map(|t| (t.id, t.file.probe()))
            .collect();
        let tree_version = self.tree_version;
        self.polling = true;
        let job = cx.background_spawn(async move {
            let checks: Vec<(u64, Probe, External)> = probes
                .into_iter()
                .map(|(id, probe)| {
                    let found = space.check(&probe);
                    (id, probe, found)
                })
                .collect();
            let tree = space.tree();
            (space, checks, tree)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (space, checks, tree) = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.polling = false;
                this.apply_poll(&space, tree_version, checks, tree, window, cx);
            });
        })
        .detach();
    }

    /// Применяет то, что нашла фоновая проверка, — кроме устаревшего.
    fn apply_poll(
        &mut self,
        polled: &Space,
        tree_version: u64,
        checks: Vec<(u64, Probe, External)>,
        tree: bolh_core::Result<Vec<Entry>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Пока шла проверка, открыли другое пространство.
        let Some(space) = self.space.as_ref().filter(|s| s.root() == polled.root()) else {
            return;
        };
        let mut notes: Vec<(NotificationType, SharedString)> = Vec::new();
        for (id, probe, found) in checks {
            let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
                continue;
            };
            // Пока шла проверка, файл сохранили, перечитали или
            // переименовали: ответ про прежний файл, и своя же запись
            // показалась бы чужой.
            if tab.saving.is_some() || !tab.file.is_current(&probe) {
                continue;
            }
            let name = tab.title();
            match found {
                External::Unchanged | External::Unreadable(_) => {}
                External::Changed(content, bytes) => {
                    if tab.unsaved() {
                        if !tab.external_noticed {
                            tab.external_noticed = true;
                            notes.push((
                                NotificationType::Warning,
                                trf(cx, "sync.changed_dirty", &[("name", &name)]),
                            ));
                        }
                    } else {
                        space.accept_external(&mut tab.file, *content, bytes);
                        Self::show_file_content(tab, window, cx);
                        notes.push((
                            NotificationType::Info,
                            trf(cx, "sync.changed", &[("name", &name)]),
                        ));
                    }
                }
                External::Deleted => {
                    if !tab.external_noticed {
                        tab.external_noticed = true;
                        notes.push((
                            NotificationType::Warning,
                            trf(cx, "sync.deleted", &[("name", &name)]),
                        ));
                    }
                }
            }
        }
        for (kind, text) in notes {
            self.notify(kind, text, false, window, cx);
        }
        if self.tree_version == tree_version {
            self.set_tree(tree.unwrap_or_default(), cx);
        }
    }

    // ---- Переименование и корзина ---------------------------------------

    fn rename_dialog(&mut self, rel: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let current = if FileKind::from_path(&rel).is_some() {
            rel.file_stem()
        } else {
            rel.file_name()
        }
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
        self.name_input
            .update(cx, |s, cx| s.set_value(current, window, cx));
        let weak = cx.entity().downgrade();
        let input = self.name_input.clone();
        let first = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                input.focus_handle(cx).focus(window, cx);
            }
            let (weak, input, rel) = (weak.clone(), input.clone(), rel.clone());
            dialog
                .title(tr(cx, "tree.rename_title"))
                .w(px(420.))
                .child(Input::new(&input))
                .footer({
                    h_flex().gap_2().justify_end().children([
                        Button::new("rename-ok")
                            .primary()
                            .label(tr(cx, "common.rename"))
                            .on_click(move |_, window, cx| {
                                let name = input.read(cx).value().to_string();
                                let ok = weak
                                    .update(cx, |this, cx| {
                                        this.rename(&rel, name.trim(), window, cx)
                                    })
                                    .unwrap_or(false);
                                if ok {
                                    window.close_dialog(cx);
                                }
                            })
                            .into_any_element(),
                        Button::new("rename-cancel")
                            .label(tr(cx, "common.cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx))
                            .into_any_element(),
                    ])
                })
        });
    }

    pub fn rename(
        &mut self,
        rel: &Path,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // Сначала сохранить: переименование не должно потерять правки.
        self.save_all(window, cx);
        let Some(space) = &self.space else {
            return false;
        };
        match space.rename(rel, name) {
            Ok(to) => {
                for tab in &mut self.tabs {
                    if let Ok(rest) = tab.file.rel.strip_prefix(rel) {
                        tab.file.rel = if rest.as_os_str().is_empty() {
                            to.clone()
                        } else {
                            to.join(rest)
                        };
                    }
                }
                self.refresh_tree(cx);
                cx.notify();
                true
            }
            Err(e) => {
                self.notify_error(&e, window, cx);
                false
            }
        }
    }

    pub fn delete(&mut self, rel: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.save_all(window, cx);
        let Some(space) = &self.space else { return };
        match space.delete(rel, Self::clock()) {
            Ok(entry) => {
                let gone: Vec<usize> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.file.rel.starts_with(rel))
                    .map(|(i, _)| i)
                    .collect();
                for ix in gone.into_iter().rev() {
                    self.tabs.remove(ix);
                }
                self.active = if self.tabs.is_empty() {
                    None
                } else {
                    Some(self.active.unwrap_or(0).min(self.tabs.len() - 1))
                };
                if self.target_folder.starts_with(rel) {
                    self.target_folder = PathBuf::new();
                }
                let text = trf(cx, "trash.deleted", &[("name", &entry.name())]);
                self.notify(NotificationType::Info, text, false, window, cx);
                self.refresh_tree(cx);
                cx.notify();
            }
            Err(e) => self.notify_error(&e, window, cx),
        }
    }

    pub fn restore(&mut self, entry: &TrashEntry, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = &self.space else { return };
        match space.restore(entry) {
            Ok(_) => {
                let text = trf(cx, "trash.restored", &[("name", &entry.name())]);
                self.notify(NotificationType::Success, text, false, window, cx);
                self.refresh_tree(cx);
            }
            Err(e) => self.notify_error(&e, window, cx),
        }
    }

    pub fn trash_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.space.is_none() {
            return;
        }
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let entries = weak
                .read_with(cx, |this, _| {
                    this.space
                        .as_ref()
                        .and_then(|s| s.trash().list().ok())
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            let body: AnyElement = if entries.is_empty() {
                empty_note(tr(cx, "trash.empty"), cx)
            } else {
                v_flex()
                    .gap_1()
                    .children(entries.into_iter().map(|e| {
                        let (w1, w2) = (weak.clone(), weak.clone());
                        let (e1, e2) = (e.clone(), e.clone());
                        let when = e
                            .deleted_at
                            .with_timezone(&chrono::Local)
                            .format("%d.%m.%Y %H:%M");
                        h_flex()
                            .gap_2()
                            .items_center()
                            .py_1()
                            .child(Icon::new(kind_icon(&e.original, e.is_dir)).small())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(e.original.to_string_lossy().into_owned()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(when.to_string()),
                            )
                            .child(
                                Button::new(SharedString::from(format!("restore-{}", e.key)))
                                    .small()
                                    .label(tr(cx, "trash.restore"))
                                    .on_click(move |_, window, cx| {
                                        let _ =
                                            w1.update(cx, |this, cx| this.restore(&e1, window, cx));
                                        window.refresh();
                                    }),
                            )
                            .child(
                                Button::new(SharedString::from(format!("purge-{}", e.key)))
                                    .small()
                                    .danger()
                                    .label(tr(cx, "trash.purge"))
                                    .on_click(move |_, window, cx| {
                                        let _ = w2.update(cx, |this, cx| {
                                            if let Some(space) = &this.space
                                                && let Err(err) = space.purge(&e2)
                                            {
                                                this.notify_error(&err, window, cx);
                                            }
                                        });
                                        window.refresh();
                                    }),
                            )
                    }))
                    .into_any_element()
            };
            dialog.title(tr(cx, "trash.title")).w(px(640.)).child(body)
        });
    }

    pub fn history_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_tab().map(|t| t.id) else {
            return;
        };
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let snaps = weak
                .read_with(cx, |this, _| {
                    let tab = this.tabs.iter().find(|t| t.id == tab_id)?;
                    this.space
                        .as_ref()?
                        .history()
                        .list(tab.file.content.id())
                        .ok()
                })
                .ok()
                .flatten()
                .unwrap_or_default();
            let body: AnyElement = if snaps.is_empty() {
                empty_note(tr(cx, "history.empty"), cx)
            } else {
                v_flex()
                    .gap_1()
                    .children(snaps.into_iter().map(|snap| {
                        let weak = weak.clone();
                        let when = snap
                            .at
                            .with_timezone(&chrono::Local)
                            .format("%d.%m.%Y %H:%M:%S")
                            .to_string();
                        let id = SharedString::from(format!("snap-{}", snap.at.timestamp_millis()));
                        h_flex()
                            .gap_2()
                            .items_center()
                            .py_1()
                            .child(div().flex_1().child(when.clone()))
                            .child(
                                Button::new(id)
                                    .small()
                                    .label(tr(cx, "history.restore"))
                                    .on_click(move |_, window, cx| {
                                        let snap = snap.clone();
                                        let when = when.clone();
                                        let _ = weak.update(cx, |this, cx| {
                                            this.restore_snapshot(tab_id, &snap, &when, window, cx)
                                        });
                                        window.close_dialog(cx);
                                    }),
                            )
                    }))
                    .into_any_element()
            };
            dialog
                .title(tr(cx, "history.title"))
                .w(px(520.))
                .child(body)
        });
    }

    fn restore_snapshot(
        &mut self,
        tab_id: u64,
        snap: &bolh_core::history::Snapshot,
        when: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Сначала дописать правки: откат кладёт нынешнее состояние в
        // историю, и оно должно быть полным; и запись не должна столкнуться
        // с фоновым сохранением.
        self.flush_tab(tab_id, window, cx);
        let Some(space) = &self.space else { return };
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let label = conflict_label(cx);
        match space.restore_snapshot(&mut tab.file, snap, &label, Self::clock()) {
            Ok(_) => {
                tab.saved_edits = tab.edits;
                Self::show_file_content(tab, window, cx);
                let text = trf(cx, "history.restored", &[("time", when)]);
                self.notify(NotificationType::Success, text, false, window, cx);
                self.refresh_tree(cx);
            }
            Err(e) => self.notify_error(&e, window, cx),
        }
        cx.notify();
    }

    // ---- Палитра команд -------------------------------------------------

    fn palette_items(&self, cx: &App) -> (Vec<palette::Item>, Vec<palette::Item>) {
        let i18n = crate::i18n::current(cx);
        let ctx = palette::Context {
            i18n,
            has_space: self.space.is_some(),
            has_tab: self.active.is_some(),
            spaces: &self.settings.spaces,
            active_space: self.space.as_ref().and_then(|s| {
                self.settings
                    .spaces
                    .iter()
                    .find(|p| p.as_path() == s.root())
            }),
            tree: &self.tree,
        };
        (palette::commands(&ctx), palette::files(&self.tree))
    }

    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (commands, files) = self.palette_items(cx);
        let groups = Rc::new(vec![files, commands]);
        let state = self.palette.clone();
        state.update(cx, |s, cx| s.set_query("", window, cx));
        let weak = cx.entity().downgrade();
        let first = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                let state = state.clone();
                state.update(cx, |s, cx| s.focus(window, cx));
            }
            let groups = groups.clone();
            let weak = weak.clone();
            let mut palette = Command::new(&state)
                .bordered(false)
                .placeholder(tr(cx, "palette.placeholder"))
                .max_h(px(420.))
                .empty(|_, _, cx| empty_note(tr(cx, "palette.empty"), cx));
            let headings = [tr(cx, "palette.files"), tr(cx, "palette.commands")];
            for (items, heading) in groups.iter().zip(headings) {
                palette = palette.group(CommandGroup::new().label(heading).items(
                    items.iter().map(|it| {
                        CommandItem::new()
                            .label(it.label.clone())
                            .keywords(it.keywords.clone())
                            .icon(command_icon(&it.command))
                    }),
                ));
            }
            let confirm_groups = groups.clone();
            palette = palette.on_confirm(move |ix: IndexPath, window, cx| {
                let cmd = confirm_groups
                    .get(ix.section)
                    .and_then(|g| g.get(ix.row))
                    .map(|i| i.command.clone());
                window.close_dialog(cx);
                if let Some(cmd) = cmd {
                    let _ = weak.update(cx, |this, cx| this.run(cmd, window, cx));
                }
            });
            dialog
                .close_button(false)
                .p_0()
                .w(px(560.))
                .rounded(cx.theme().radius_lg)
                .overflow_hidden()
                .child(palette)
        });
    }

    pub fn run(&mut self, cmd: PaletteCommand, window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            PaletteCommand::NewDocument => self.create_file(FileKind::Document, window, cx),
            PaletteCommand::NewCanvas => self.create_file(FileKind::Canvas, window, cx),
            PaletteCommand::NewFolder => self.create_folder(window, cx),
            PaletteCommand::NewSpace => self.new_space_dialog(window, cx),
            PaletteCommand::OpenSpace => self.open_space_dialog(window, cx),
            PaletteCommand::SwitchSpace(p) => self.open_space(&p, window, cx),
            PaletteCommand::Save => self.save_active(window, cx),
            PaletteCommand::CloseTab => {
                if let Some(a) = self.active {
                    self.close_tab(a, window, cx)
                }
            }
            PaletteCommand::History => self.history_dialog(window, cx),
            PaletteCommand::Trash => self.trash_dialog(window, cx),
            PaletteCommand::Theme(t) => self.set_theme(t, window, cx),
            PaletteCommand::Language(code) => self.set_language(code, window, cx),
            PaletteCommand::CheckUpdate => self.check_update(true, window, cx),
            PaletteCommand::OpenFile(rel) => self.open_file(&rel, window, cx),
        }
    }

    // ---- Обновления -----------------------------------------------------

    /// Спрашивает GitHub о новой версии в фоне. `manual` — человек нажал
    /// сам: тогда сообщаем и «у вас последняя версия», и ошибку сети.
    pub fn check_update(&mut self, manual: bool, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.update,
            UpdateStatus::Checking | UpdateStatus::Downloading(_)
        ) {
            return;
        }
        self.update = UpdateStatus::Checking;
        let task = cx.background_spawn(async move { updater::check(crate::VERSION) });
        self.update_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.update = match result {
                    Ok(Some(release)) => UpdateStatus::Available(release),
                    Ok(None) => {
                        if manual {
                            let text = tr(cx, "update.up_to_date");
                            this.notify(NotificationType::Success, text, false, window, cx);
                        }
                        UpdateStatus::Idle
                    }
                    Err(e) => {
                        if manual {
                            let text = update_error_text(&e, cx);
                            this.notify(NotificationType::Warning, text, false, window, cx);
                        }
                        UpdateStatus::Idle
                    }
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn install_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let UpdateStatus::Available(release) = self.update.clone() else {
            return;
        };
        self.save_all(window, cx);
        self.update = UpdateStatus::Downloading(release.clone());
        let dir = paths::update_dir();
        let task = cx.background_spawn(async move {
            let path = updater::download(&release, &dir)?;
            updater::launch_installer(&path)
        });
        self.update_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                // Установщик запущен: он закроет программу и откроет новую.
                // Правки, сделанные пока шло скачивание, — сохранить.
                Ok(()) => {
                    this.save_all(window, cx);
                    cx.quit();
                }
                Err(e) => {
                    let text = update_error_text(&e, cx);
                    this.update = UpdateStatus::Failed(text.clone());
                    this.notify(NotificationType::Error, text, true, window, cx);
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    // ---- Отрисовка ------------------------------------------------------

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme_menu = self.render_theme_menu(cx);
        let theme = cx.theme().clone();
        let space_name = self
            .space
            .as_ref()
            .map(|s| SharedString::from(s.name()))
            .unwrap_or_else(|| tr(cx, "space.title"));
        let weak = cx.entity().downgrade();
        let spaces = self.settings.spaces.clone();
        let current = self.space.as_ref().map(|s| s.root().to_path_buf());
        let space_menu = Button::new("space-menu")
            .ghost()
            .w_full()
            .justify_start()
            .label(space_name)
            .icon(Icon::new(IconName::ChevronsUpDown).small())
            .dropdown_menu(move |mut menu, _, cx| {
                for p in &spaces {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.to_string_lossy().into_owned());
                    let (w, p2) = (weak.clone(), p.clone());
                    menu = menu.item(
                        PopupMenuItem::new(name)
                            .checked(current.as_deref() == Some(p.as_path()))
                            .on_click(move |_, window, cx| {
                                let _ = w.update(cx, |this, cx| this.open_space(&p2, window, cx));
                            }),
                    );
                }
                let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
                let forget = current.clone();
                menu = menu
                    .separator()
                    .item(
                        PopupMenuItem::new(tr(cx, "space.new")).on_click(move |_, window, cx| {
                            let _ = w1.update(cx, |this, cx| this.new_space_dialog(window, cx));
                        }),
                    )
                    .item(PopupMenuItem::new(tr(cx, "space.open")).on_click(
                        move |_, window, cx| {
                            let _ = w2.update(cx, |this, cx| this.open_space_dialog(window, cx));
                        },
                    ));
                if let Some(root) = forget {
                    menu = menu.item(PopupMenuItem::new(tr(cx, "space.forget")).on_click(
                        move |_, window, cx| {
                            let _ = w3.update(cx, |this, cx| this.forget_space(&root, window, cx));
                        },
                    ));
                }
                menu
            });

        let has_space = self.space.is_some();
        let toolbar = has_space.then(|| {
            h_flex()
                .gap_1()
                .px_2()
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(tr(cx, "tree.title")),
                )
                .child(
                    Button::new("new-document")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::FilePlus))
                        .tooltip(tr(cx, "tree.new_document"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.create_file(FileKind::Document, window, cx)
                        })),
                )
                .child(
                    Button::new("new-canvas")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::Shapes))
                        .tooltip(tr(cx, "tree.new_canvas"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.create_file(FileKind::Canvas, window, cx)
                        })),
                )
                .child(
                    Button::new("new-folder")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::FolderPlus))
                        .tooltip(tr(cx, "tree.new_folder"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.create_folder(window, cx)),
                        ),
                )
        });

        let weak = cx.entity().downgrade();
        let tree_view = if self.tree.is_empty() {
            div()
                .p_4()
                .text_sm()
                .text_color(theme.muted_foreground)
                .when(has_space, |d| d.child(tr(cx, "tree.empty")))
                .into_any_element()
        } else {
            let (w_click, w_menu) = (weak.clone(), weak.clone());
            tree(&self.tree_state, move |_, entry, _selected, _, cx| {
                let item = entry.item();
                let rel = PathBuf::from(item.id.as_ref());
                let is_file = !entry.is_folder();
                let icon = if !is_file {
                    if entry.is_expanded() {
                        IconName::FolderOpen
                    } else {
                        IconName::Folder
                    }
                } else {
                    file_icon(&rel)
                };
                let w = w_click.clone();
                // Имя элемента — путь файла: по нему его находят тесты.
                ListItem::new(SharedString::from(format!("tree:{}", item.id)))
                    .pl(px(12.) + px(14.) * entry.depth() as f32)
                    .py(px(2.))
                    .child(
                        h_flex()
                            .gap_2()
                            .text_sm()
                            .child(
                                Icon::new(icon)
                                    .small()
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child(div().truncate().child(item.label.clone())),
                    )
                    .on_click(move |_, window, cx| {
                        let rel = rel.clone();
                        let _ =
                            w.update(cx, |this, cx| this.on_tree_click(rel, is_file, window, cx));
                    })
            })
            .context_menu(move |_, entry, menu, _, cx| {
                let rel = PathBuf::from(entry.item().id.as_ref());
                let (w1, w2) = (w_menu.clone(), w_menu.clone());
                let r2 = rel.clone();
                menu.item(PopupMenuItem::new(tr(cx, "common.rename")).on_click(
                    move |_, window, cx| {
                        let rel = rel.clone();
                        let _ = w1.update(cx, |this, cx| this.rename_dialog(rel, window, cx));
                    },
                ))
                .item(
                    PopupMenuItem::new(tr(cx, "common.delete")).on_click(move |_, window, cx| {
                        let _ = w2.update(cx, |this, cx| this.delete(&r2, window, cx));
                    }),
                )
            })
            .flex_1()
            .into_any_element()
        };

        let trash_button = has_space.then(|| {
            Button::new("open-trash")
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::Trash))
                .tooltip(tr(cx, "trash.title"))
                .on_click(cx.listener(|this, _, window, cx| this.trash_dialog(window, cx)))
        });
        let footer = h_flex()
            .gap_1()
            .p_2()
            .border_t_1()
            .border_color(theme.border)
            .child(
                Button::new("open-palette")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Search))
                    .tooltip(tr(cx, "palette.placeholder"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx))),
            )
            .children(trash_button)
            .child(div().flex_1())
            .child(theme_menu);

        v_flex()
            .id("sidebar")
            .w(px(272.))
            .h_full()
            .flex_none()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(div().p_2().child(space_menu))
            .children(toolbar)
            .child(div().flex_1().min_h_0().py_1().child(tree_view))
            .child(footer)
            .into_any_element()
    }

    fn render_theme_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let current = self.settings.theme;
        let lang = crate::i18n::current(cx).code();
        Button::new("theme-menu")
            .ghost()
            .xsmall()
            .icon(Icon::new(IconName::Settings))
            .tooltip(tr(cx, "theme.title"))
            .dropdown_menu_with_anchor(gpui_kit::Anchor::BottomRight, move |mut menu, _, cx| {
                menu = menu.label(tr(cx, "theme.title"));
                for (choice, key) in [
                    (ThemeChoice::Dark, "theme.dark"),
                    (ThemeChoice::Light, "theme.light"),
                    (ThemeChoice::System, "theme.system"),
                ] {
                    let w = weak.clone();
                    menu = menu.item(
                        PopupMenuItem::new(tr(cx, key))
                            .checked(current == choice)
                            .on_click(move |_, window, cx| {
                                let _ = w.update(cx, |this, cx| this.set_theme(choice, window, cx));
                            }),
                    );
                }
                menu = menu.separator().label(tr(cx, "settings.language"));
                for (code, _) in bolh_core::i18n::LANGUAGES {
                    let w = weak.clone();
                    let name = bolh_core::i18n::I18n::native_name(code);
                    menu = menu.item(PopupMenuItem::new(name).checked(lang == *code).on_click(
                        move |_, window, cx| {
                            let _ = w.update(cx, |this, cx| this.set_language(code, window, cx));
                        },
                    ));
                }
                let w = weak.clone();
                menu.separator()
                    .item(PopupMenuItem::new(tr(cx, "command.check_update")).on_click(
                        move |_, window, cx| {
                            let _ = w.update(cx, |this, cx| this.check_update(true, window, cx));
                        },
                    ))
            })
            .into_any_element()
    }

    fn render_tabs(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        TabBar::new("tabs")
            .folder()
            .small()
            .selected_index(self.active.unwrap_or(0))
            .on_click(cx.listener(|this, ix: &usize, window, cx| this.activate(*ix, window, cx)))
            .children(self.tabs.iter().enumerate().map(|(ix, t)| {
                let w = weak.clone();
                let dirty = t.unsaved();
                Tab::new()
                    .label(t.title())
                    .prefix(Icon::new(file_icon(&t.file.rel)).small())
                    .suffix(
                        h_flex()
                            .gap_1()
                            .when(dirty, |d| {
                                d.child(
                                    div()
                                        .size(px(6.))
                                        .rounded_full()
                                        .bg(cx.theme().muted_foreground),
                                )
                            })
                            .child(
                                Button::new(SharedString::from(format!("close-tab-{}", t.id)))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(IconName::X))
                                    .on_click(move |_, window, cx| {
                                        let _ =
                                            w.update(cx, |this, cx| this.close_tab(ix, window, cx));
                                    }),
                            ),
                    )
            }))
            .into_any_element()
    }

    fn render_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        if self.space.is_none() {
            return v_flex()
                .id("welcome")
                .size_full()
                .items_center()
                .justify_center()
                .gap_4()
                .p_8()
                .child(
                    Icon::new(IconName::LayoutDashboard)
                        .size(px(48.))
                        .text_color(theme.primary),
                )
                .child(
                    div()
                        .text_2xl()
                        .font_semibold()
                        .child(tr(cx, "space.none_title")),
                )
                .child(
                    div()
                        .max_w(px(520.))
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .child(tr(cx, "space.none_body")),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(
                            Button::new("welcome-new-space")
                                .primary()
                                .label(tr(cx, "space.new"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.new_space_dialog(window, cx)
                                })),
                        )
                        .child(
                            Button::new("welcome-open-space")
                                .label(tr(cx, "space.open"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_space_dialog(window, cx)
                                })),
                        ),
                )
                .into_any_element();
        }
        let Some(tab) = self.active_tab() else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .font_semibold()
                        .child(tr(cx, "tabs.empty_title")),
                )
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(tr(cx, "tabs.empty_body")),
                )
                .into_any_element();
        };
        let writable = tab.file.content.is_writable();
        let stub_note = |key: &'static str, cx: &App| {
            div()
                .px_4()
                .py_2()
                .rounded(px(12.))
                .bg(cx.theme().muted)
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(tr(cx, key))
        };
        // Холст занимает всё место, без полей документа.
        if let Some(canvas) = &tab.canvas {
            return div()
                .id("content")
                .size_full()
                .child(canvas.clone())
                .into_any_element();
        }
        let body = match (&tab.file.content, &tab.editor) {
            (Content::Document(_), Some(editor)) => v_flex()
                .size_full()
                .gap_3()
                .child(stub_note(
                    if writable {
                        "document.stub"
                    } else {
                        "document.read_only"
                    },
                    cx,
                ))
                .child(
                    Textarea::new(editor)
                        .h_full()
                        .bordered(false)
                        .readonly(!writable)
                        .aria_label(tab.title()),
                )
                .into_any_element(),
            _ => div().into_any_element(),
        };
        div()
            .id("content")
            .size_full()
            .px(px(32.))
            .py(px(24.))
            .bg(theme.background)
            .child(div().max_w(px(860.)).h_full().mx_auto().child(body))
            .into_any_element()
    }

    fn render_update_banner(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme();
        let (text, busy) = match &self.update {
            UpdateStatus::Available(r) => (
                trf(
                    cx,
                    "update.available",
                    &[("version", &r.version.to_string())],
                ),
                false,
            ),
            UpdateStatus::Downloading(_) => (tr(cx, "update.downloading"), true),
            _ => return None,
        };
        Some(
            h_flex()
                .id("update-banner")
                .gap_3()
                .px_4()
                .py_2()
                .bg(theme.muted)
                .border_b_1()
                .border_color(theme.border)
                .child(
                    Icon::new(IconName::Download)
                        .small()
                        .text_color(theme.primary),
                )
                .child(div().flex_1().text_sm().child(text))
                .when(!busy, |d| {
                    d.child(
                        Button::new("update-install")
                            .primary()
                            .small()
                            .label(tr(cx, "update.install"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.install_update(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("update-later")
                            .ghost()
                            .small()
                            .label(tr(cx, "update.later"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.update = UpdateStatus::Idle;
                                cx.notify();
                            })),
                    )
                })
                .into_any_element(),
        )
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = self.render_sidebar(cx);
        let tabs = if self.tabs.is_empty() {
            None
        } else {
            Some(self.render_tabs(cx))
        };
        let banner = self.render_update_banner(cx);
        let content = self.render_content(cx);
        let theme = cx.theme();
        let status = self.status.clone();
        h_flex()
            .id("workspace")
            .key_context(actions::CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|this, _: &OpenPalette, window, cx| this.open_palette(window, cx)),
            )
            .on_action(cx.listener(|this, _: &NewDocument, window, cx| {
                this.create_file(FileKind::Document, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewCanvas, window, cx| {
                this.create_file(FileKind::Canvas, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &NewFolder, window, cx| this.create_folder(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Save, window, cx| this.save_active(window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if let Some(a) = this.active {
                    this.close_tab(a, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                if let Some(a) = this.active {
                    let n = this.tabs.len();
                    this.activate((a + 1) % n, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                if let Some(a) = this.active {
                    let n = this.tabs.len();
                    this.activate((a + n - 1) % n, window, cx);
                }
            }))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(sidebar)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .children(tabs)
                    .children(banner)
                    .child(div().flex_1().min_h_0().child(content))
                    .child(
                        h_flex()
                            .id("status-bar")
                            .h(px(24.))
                            .px_3()
                            .border_t_1()
                            .border_color(theme.border)
                            .bg(theme.sidebar)
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(div().flex_1().child(status))
                            .child(SharedString::from(format!("v{}", crate::VERSION))),
                    ),
            )
    }
}

fn to_tree_items(entries: &[Entry], expanded: &[SharedString]) -> Vec<TreeItem> {
    entries
        .iter()
        .map(|e| {
            let id = SharedString::from(e.rel.to_string_lossy().into_owned());
            let item = TreeItem::new(id.clone(), e.name.clone());
            match &e.kind {
                EntryKind::Folder(children) => item
                    .expanded(expanded.contains(&id))
                    .children(to_tree_items(children, expanded)),
                EntryKind::File(_) => item,
            }
        })
        .collect()
}

fn file_icon(rel: &Path) -> IconName {
    match FileKind::from_path(rel) {
        Some(FileKind::Canvas) => IconName::Shapes,
        _ => IconName::FileText,
    }
}

fn kind_icon(rel: &Path, is_dir: bool) -> IconName {
    if is_dir {
        IconName::Folder
    } else {
        file_icon(rel)
    }
}

fn command_icon(cmd: &PaletteCommand) -> IconName {
    match cmd {
        PaletteCommand::OpenFile(rel) => file_icon(rel),
        PaletteCommand::NewDocument => IconName::FilePlus,
        PaletteCommand::NewCanvas => IconName::Shapes,
        PaletteCommand::NewFolder => IconName::FolderPlus,
        PaletteCommand::NewSpace | PaletteCommand::OpenSpace => IconName::FolderInput,
        PaletteCommand::SwitchSpace(_) => IconName::LayoutDashboard,
        PaletteCommand::Save => IconName::Check,
        PaletteCommand::CloseTab => IconName::X,
        PaletteCommand::History => IconName::RotateCcw,
        PaletteCommand::Trash => IconName::Trash,
        PaletteCommand::Theme(ThemeChoice::Dark) => IconName::Moon,
        PaletteCommand::Theme(ThemeChoice::Light) => IconName::Sun,
        PaletteCommand::Theme(ThemeChoice::System) => IconName::Monitor,
        PaletteCommand::Language(_) => IconName::Languages,
        PaletteCommand::CheckUpdate => IconName::Download,
    }
}

fn empty_note(text: SharedString, cx: &App) -> AnyElement {
    div()
        .p_4()
        .text_center()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

fn update_error_text(e: &updater::UpdateError, cx: &App) -> SharedString {
    match e {
        updater::UpdateError::Offline => tr(cx, "update.offline"),
        updater::UpdateError::NotFound => trf(cx, "update.failed", &[("reason", "404")]),
        updater::UpdateError::BadSignature => tr(cx, "update.bad_signature"),
        updater::UpdateError::Other(reason) => trf(cx, "update.failed", &[("reason", reason)]),
    }
}

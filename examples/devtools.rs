//! The devtools inspector over a small two-pane application. Run with
//! `cargo run --example devtools --features devtools`.
//!
//! `alt+i` cycles the inspector: panel → panel with layout outlines → hidden.
//! `tab` moves focus between the list and the composer, `↑`/`↓` move in the
//! list, `g g` jumps to its top (watch the pending sequence), `enter` in the
//! composer adds an item, and `esc`, `ctrl+c`, or `q` in the list quits.
//!
//! Pass `--outlines` to start with the outlines already on, `--hidden` to start
//! with the inspector closed.

use std::io;

use tuika::devtools::{Inspector, InspectorMode};
use tuika::keymap::{Dispatch, Keymap, Layer};
use tuika::prelude::*;
use tuika::routing::Router;
use tuika::ui::{Line, Span};

mod support;

#[derive(Clone, Copy, Debug)]
enum Command {
    Top,
    Bottom,
}

struct App {
    theme: Theme,
    inspector: Inspector,
    focus: FocusRegistry,
    keys: Keymap<Command>,
    items: Vec<String>,
    list: SelectState,
    composer: TextInputState,
}

impl App {
    fn new(theme: Theme, inspector: Inspector) -> Self {
        let mut focus = FocusRegistry::new();
        // The ring never changes in this app, so it is registered once.
        focus.begin_frame();
        focus.register("list");
        focus.register("composer");
        inspector.record_focus(&focus);
        let keys = Keymap::new().layer(
            Layer::new("list")
                .when("focus", "list")
                .bind("g g", Command::Top)
                .bind("G", Command::Bottom),
        );
        let mut keys = keys;
        keys.set_data("focus", "list");
        Self {
            theme,
            inspector,
            focus,
            keys,
            items: [
                "layout.rs",
                "overlay.rs",
                "routing.rs",
                "keymap.rs",
                "focus.rs",
                "devtools.rs",
            ]
            .map(String::from)
            .to_vec(),
            list: SelectState::new(),
            composer: TextInputState::new(),
        }
    }

    fn pane(&self, id: &str, title: &'static str, body: Element) -> Element {
        let focused = self.focus.is_active(id);
        let border = if focused {
            self.theme.border_focused
        } else {
            self.theme.border
        };
        element(
            Boxed::new(body)
                .title(Line::from(Span::styled(title, self.theme.accent_style())))
                .border_color(border)
                .padding(Padding::symmetric(1, 0)),
        )
    }
}

impl Application for App {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        let Signal::Event(event) = signal else {
            return UpdateResult::Clean;
        };
        // `q` is text in the composer, so it quits only from the list.
        if let Event::Key(key) = &event
            && (key.code == KeyCode::Esc
                || (key.ctrl && key.code == KeyCode::Char('c'))
                || (key.plain() && key.code == KeyCode::Char('q') && self.focus.is_active("list")))
        {
            return UpdateResult::Exit;
        }
        if self.focus.handle(&event).consumed() {
            let active = self.focus.active().unwrap_or_default().to_owned();
            self.keys.set_data("focus", active);
            self.inspector.record_focus(&self.focus);
            self.inspector.record_keymap(&self.keys);
            return UpdateResult::Dirty;
        }

        let len = self.items.len();
        let mut command = None;
        let mut submitted = false;
        let delivery = {
            let mut router = Router::new(&self.focus, &event);
            let keys = &mut self.keys;
            let list_active = self.focus.is_active("list");
            router.pre_fn(|event| match event {
                Event::Key(key) if list_active => match keys.dispatch(*key) {
                    Dispatch::Command(found) => {
                        command = Some(found);
                        InputOutcome::Submitted
                    }
                    Dispatch::Pending => InputOutcome::Consumed,
                    Dispatch::Unmatched => InputOutcome::Ignored,
                },
                _ => InputOutcome::Ignored,
            });
            let list = &mut self.list;
            router.target_fn("list", |event| list.handle(event, len));
            let composer = &mut self.composer;
            router.target_fn("composer", |event| {
                let outcome = composer.handle(event);
                submitted = outcome == InputOutcome::Submitted;
                outcome
            });
            router.finish()
        };
        self.inspector.record_route(&delivery);
        self.inspector.record_keymap(&self.keys);

        match command {
            Some(Command::Top) => self.list.select(Some(0)),
            Some(Command::Bottom) => self.list.select(Some(len.saturating_sub(1))),
            None => {}
        }
        if submitted {
            let text = self.composer.text().trim().to_owned();
            if !text.is_empty() {
                self.items.push(text);
                self.list.select(Some(self.items.len() - 1));
            }
            self.composer.clear();
        }
        if delivery.consumed() {
            UpdateResult::Dirty
        } else {
            UpdateResult::Clean
        }
    }

    fn view(&self, _frame: u64) -> ScopedElement<'_> {
        let theme = &self.theme;
        let rows = self
            .items
            .iter()
            .map(|item| Line::from(item.clone()))
            .collect();
        let list = self.inspector.trace(
            "list",
            self.pane(
                "list",
                " files ",
                element(SelectList::new(rows, &self.list)),
            ),
        );
        let selected = self
            .list
            .selected()
            .and_then(|index| self.items.get(index))
            .map_or("nothing selected", String::as_str);
        let detail = self.inspector.trace(
            "detail",
            self.pane(
                "detail",
                " detail ",
                element(Text::new(vec![
                    Line::from(Span::styled(selected.to_owned(), theme.accent_style())),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Every pane is traced, so alt+i twice outlines it.",
                        theme.muted_style(),
                    )),
                ])),
            ),
        );
        let composer = self.inspector.trace(
            "composer",
            self.pane(
                "composer",
                " composer ",
                element(
                    TextInput::new(&self.composer)
                        .placeholder("type, then enter to add", theme.muted_style()),
                ),
            ),
        );
        let hints = self.inspector.trace(
            "hints",
            KeyHints::new([
                ("alt+i", "inspector"),
                ("tab", "focus"),
                ("g g", "top"),
                ("esc", "quit"),
            ]),
        );
        element(
            Flex::column()
                .padding(Padding::all(1))
                .grow(1, element(Flex::row().gap(1).grow(1, list).grow(2, detail)))
                .fixed(3, composer)
                .fixed(1, hints),
        )
    }
}

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let mode = if cli.args.iter().any(|arg| arg == "--hidden") {
        InspectorMode::Hidden
    } else if cli.args.iter().any(|arg| arg == "--outlines") {
        InspectorMode::Outlines
    } else {
        InspectorMode::Panel
    };
    let inspector = Inspector::new().with_mode(mode);
    let runner = Runner::new(RunnerConfig::default()).with_inspector(inspector.clone());
    let mut app = App::new(cli.theme, inspector);
    let theme = app.theme;
    runner.run(&theme, &mut app)
}

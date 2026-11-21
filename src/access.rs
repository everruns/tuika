//! Screen-reader semantics: what a frame *means*, alongside what it paints.
//!
//! A screen reader reads a terminal as a stream of text. tuika paints a cell
//! grid — borders, colour, spinners, cursor-addressed repaints — and none of
//! that survives being read aloud. This module is the other half of a frame:
//! while views paint, they may also record a [`Node`] (a role, a label, a value
//! and some state) into an [`AccessFrame`] the host installed on the
//! [`RenderCtx`]. After the paint, [`AccessFrame::document`] turns the nodes
//! plus the painted cells into a linear, plain-text [`Document`] in reading
//! order.
//!
//! Two properties shape the design:
//!
//! - **It is free when off.** Recording goes through
//!   [`RenderCtx::record_access`], which takes a closure and does nothing when
//!   no frame is installed, so an ordinary paint builds no nodes and formats no
//!   labels.
//! - **Every app gets something.** Any part of the screen no leaf node covers
//!   is read back from its cells, with box-drawing, block, and Braille glyphs
//!   dropped. Labels make the output better; they are not required for it to
//!   exist.
//!
//! Apps label their own views with [`Semantic`], a layout-transparent wrapper
//! in the same shape as [`Probe`](crate::probe::Probe).
//!
//! ```
//! use tuika::access::{Role, Semantic};
//! use tuika::components::Text;
//! use tuika::testing;
//! use tuika::Theme;
//!
//! let send = Semantic::new(Text::raw("[ ⏎ ]")).role(Role::Button).label("Send");
//! let spoken = testing::screen_reader(&send, 10, 1, &Theme::default());
//! assert_eq!(spoken, "Send, button");
//! ```

use std::cell::RefCell;
use std::fmt;
use std::io::{self, Write};

use crate::buffer::Buffer;
use crate::geometry::{Position, Rect, Size};
use crate::surface::Surface;
use crate::view::{Element, RenderCtx, View};

/// What a [`Node`] is, in the vocabulary screen readers speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Role {
    /// Plain text. Spoken as its label alone.
    Text,
    /// A heading; see [`Node::level`].
    Heading,
    /// A pressable control.
    Button,
    /// A control that is checked or not; see [`Node::checked`].
    Checkbox,
    /// One choice among several.
    Radio,
    /// A hyperlink.
    Link,
    /// A picture; the label is its alternative text.
    Image,
    /// An editable text field; the value is its current text.
    TextInput,
    /// A determinate or indeterminate progress indicator.
    ProgressBar,
    /// A live status line whose changes should be spoken.
    Status,
    /// An urgent message, spoken as soon as it appears.
    Alert,
    /// A list of items. Container.
    List,
    /// One item of a [`List`](Role::List).
    ListItem,
    /// A row of tabs. Container.
    TabList,
    /// One tab of a [`TabList`](Role::TabList).
    Tab,
    /// A table. Container.
    Table,
    /// One row of a [`Table`](Role::Table).
    Row,
    /// A tree. Container.
    Tree,
    /// One item of a [`Tree`](Role::Tree).
    TreeItem,
    /// A dialog. Container.
    Dialog,
    /// A labelled region with no role of its own. Container.
    Group,
}

impl Role {
    /// Whether a node of this role groups other content rather than replacing
    /// it.
    ///
    /// A container's label is spoken as a header and the content painted
    /// inside its rect — child nodes and unlabelled text alike — follows it.
    /// A leaf node *replaces* the cells it covers: a button labelled "Send"
    /// is read as "Send, button", not as the glyphs it drew.
    pub const fn is_container(self) -> bool {
        matches!(
            self,
            Self::List | Self::TabList | Self::Table | Self::Tree | Self::Dialog | Self::Group
        )
    }

    /// The word a screen reader appends to the label, if any.
    ///
    /// Roles whose meaning is carried by the text itself (plain text, status,
    /// list and tree items, groups) have none, so they are not announced on
    /// every line.
    pub const fn name(self) -> Option<&'static str> {
        Some(match self {
            Self::Text | Self::Status | Self::ListItem | Self::TreeItem | Self::Group => {
                return None;
            }
            Self::Heading => "heading",
            Self::Button => "button",
            Self::Checkbox => "checkbox",
            Self::Radio => "radio button",
            Self::Link => "link",
            Self::Image => "image",
            Self::TextInput => "text input",
            Self::ProgressBar => "progress bar",
            Self::Alert => "alert",
            Self::List => "list",
            Self::TabList => "tab list",
            Self::Tab => "tab",
            Self::Table => "table",
            Self::Row => "row",
            Self::Tree => "tree",
            Self::Dialog => "dialog",
        })
    }
}

/// One meaningful element of a frame: where it is, what it is, and its state.
///
/// Build one with [`Node::new`] and the builder methods, then record it with
/// [`RenderCtx::record_access`]. The fields are public to read; the type is
/// `#[non_exhaustive]` so new states can be added without a breaking change.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Node {
    /// The screen cells the node stands for, already clipped to what was
    /// visible. A node with an empty area is dropped.
    pub area: Rect,
    /// What the node is.
    pub role: Role,
    /// Its accessible name.
    pub label: String,
    /// Its current value: an input's text, a progress percentage, a selection.
    pub value: Option<String>,
    /// Heading level, `1` for the most important; for a tree item, its depth
    /// from `1` at the root.
    pub level: Option<u8>,
    /// 1-based position within its set, and the size of the set.
    pub position: Option<(usize, usize)>,
    /// Whether it holds keyboard focus.
    pub focused: bool,
    /// Whether it is the selected item of its set.
    pub selected: bool,
    /// Checked state, for checkboxes and radio buttons.
    pub checked: Option<bool>,
    /// Expanded state, for tree items and disclosures.
    pub expanded: Option<bool>,
    /// Whether it ignores input.
    pub disabled: bool,
    /// Whether it is working on something.
    pub busy: bool,
}

impl Node {
    /// A node of `role` named `label`, covering no cells yet.
    pub fn new(role: Role, label: impl Into<String>) -> Self {
        Self {
            area: Rect::ZERO,
            role,
            label: label.into(),
            value: None,
            level: None,
            position: None,
            focused: false,
            selected: false,
            checked: None,
            expanded: None,
            disabled: false,
            busy: false,
        }
    }

    /// Set the cells the node stands for.
    pub fn area(mut self, area: Rect) -> Self {
        self.area = area;
        self
    }

    /// Set the current value.
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Set the heading level.
    pub fn level(mut self, level: u8) -> Self {
        self.level = Some(level);
        self
    }

    /// Set the 1-based `index` within a set of `count`.
    pub fn position(mut self, index: usize, count: usize) -> Self {
        self.position = Some((index, count));
        self
    }

    /// Mark whether the node holds keyboard focus.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Mark whether the node is the selected item of its set.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Set the checked state.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// Set the expanded state.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// Mark whether the node ignores input.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Mark whether the node is working on something.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// The node as one spoken line: label, role, value, then state.
    ///
    /// `Sonnet, 2 of 5, selected`, `Remember me, checkbox, checked`,
    /// `Build, progress bar, 40%`. Focus is not part of the line: a host
    /// announces a focus *change* on its own.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let label = collapse_whitespace(&self.label);
        if !label.is_empty() {
            parts.push(label);
        }
        match (self.role, self.level) {
            (Role::Heading, Some(level)) => parts.push(format!("heading level {level}")),
            (role, _) => parts.extend(role.name().map(str::to_owned)),
        }
        if let (Role::TreeItem, Some(level)) = (self.role, self.level) {
            parts.push(format!("level {level}"));
        }
        if let Some(value) = &self.value {
            let value = collapse_whitespace(value);
            if value.is_empty() && self.role == Role::TextInput {
                parts.push("blank".to_owned());
            } else if !value.is_empty() {
                parts.push(value);
            }
        }
        if let Some((index, count)) = self.position {
            parts.push(format!("{index} of {count}"));
        }
        if self.selected {
            parts.push("selected".to_owned());
        }
        match self.checked {
            Some(true) => parts.push("checked".to_owned()),
            Some(false) => parts.push("not checked".to_owned()),
            None => {}
        }
        match self.expanded {
            Some(true) => parts.push("expanded".to_owned()),
            Some(false) => parts.push("collapsed".to_owned()),
            None => {}
        }
        if self.disabled {
            parts.push("unavailable".to_owned());
        }
        if self.busy {
            parts.push("busy".to_owned());
        }
        parts.join(", ")
    }
}

/// The per-frame collector views record [`Node`]s into.
///
/// A host creates one, installs it with [`RenderCtx::with_access`], paints,
/// then reads the frame back with [`document`](Self::document). Clear it (or
/// create a new one) before the next paint. Views never need to name this type:
/// they record through [`RenderCtx::record_access`].
#[derive(Debug, Default)]
pub struct AccessFrame {
    nodes: RefCell<Vec<Node>>,
    hidden: RefCell<Vec<Rect>>,
    /// While a modal overlay is open, only these rects are readable: the
    /// modal itself and anything painted over it afterwards.
    visible: RefCell<Option<Vec<Rect>>>,
}

impl AccessFrame {
    /// An empty frame.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything recorded, ready for the next paint.
    pub fn clear(&self) {
        self.nodes.borrow_mut().clear();
        self.hidden.borrow_mut().clear();
        *self.visible.borrow_mut() = None;
    }

    /// Record `node`. Nodes with an empty area are ignored.
    pub fn record(&self, node: Node) {
        if !node.area.is_empty() {
            self.nodes.borrow_mut().push(node);
        }
    }

    /// Mark `area` as decoration: its cells are never read as text.
    pub fn hide(&self, area: Rect) {
        if !area.is_empty() {
            self.hidden.borrow_mut().push(area);
        }
    }

    /// The nodes recorded so far, in paint order.
    pub fn nodes(&self) -> Vec<Node> {
        self.nodes.borrow().clone()
    }

    /// An overlay is about to paint over `area`.
    ///
    /// Nodes it fully covers are no longer on screen and are dropped. A
    /// `modal` overlay makes everything outside it inert: earlier nodes go,
    /// and cell text is read only inside the modal (and anything painted
    /// over it later).
    pub(crate) fn begin_overlay(&self, area: Rect, modal: bool) {
        let mut nodes = self.nodes.borrow_mut();
        let mut visible = self.visible.borrow_mut();
        if modal {
            nodes.clear();
            *visible = Some(vec![area]);
        } else {
            nodes.retain(|node| node.area.intersection(area) != node.area);
            if let Some(rects) = visible.as_mut() {
                rects.push(area);
            }
        }
    }

    /// Read the painted `buffer` back as a linear document.
    ///
    /// Reading order is top to bottom, then left to right, except that
    /// everything inside a container node (a list, a dialog, a labelled group)
    /// is read together, right after the container's own line. Cells covered by
    /// a leaf node, a hidden area, or an inert region are skipped; the rest are
    /// read as text, one line per screen row, with decorative glyphs dropped
    /// and blank runs collapsed.
    pub fn document(&self, buffer: &Buffer) -> Document {
        let nodes: Vec<Node> = self
            .nodes
            .borrow()
            .iter()
            .filter_map(|node| {
                let area = node.area.intersection(buffer.area);
                if area.is_empty() {
                    return None;
                }
                let mut node = Node {
                    area,
                    ..node.clone()
                };
                // A leaf with no label of its own is named by what it painted,
                // so `Semantic::new(view).role(Role::Button)` still says
                // something.
                if !node.role.is_container()
                    && node.value.is_none()
                    && collapse_whitespace(&node.label).is_empty()
                {
                    node.label = cells_text(buffer, area);
                }
                Some(node)
            })
            .collect();
        let hidden = self.hidden.borrow();
        let visible = self.visible.borrow();

        let mut items: Vec<Item> = Vec::new();
        for (order, node) in nodes.iter().enumerate() {
            items.push(Item {
                anchor: node.area.as_position(),
                order,
                content: Content::Node(order),
            });
        }
        let containers: Vec<usize> = (0..nodes.len())
            .filter(|&index| nodes[index].role.is_container())
            .collect();
        let masks = Masks::build(
            buffer.area,
            &nodes,
            &containers,
            &hidden,
            visible.as_deref(),
        );
        let order_base = nodes.len();
        for y in buffer.area.top()..buffer.area.bottom() {
            let mut segment = Segment::default();
            let mut region = None;
            // The columns a wide cluster covers after its first: some writers
            // leave an empty placeholder there, others a space.
            let mut covered_until = 0u16;
            for x in buffer.area.left()..buffer.area.right() {
                let here = masks.container(x, y);
                if here != region {
                    segment.flush(y, order_base, &mut items);
                    region = here;
                }
                if !masks.readable(x, y) {
                    segment.flush(y, order_base, &mut items);
                    continue;
                }
                let cell = &buffer[(x, y)];
                if x < covered_until || cell.raw_symbol().is_empty() {
                    continue;
                }
                let symbol = if cell.is_skipped() {
                    " "
                } else {
                    cell.symbol()
                };
                covered_until = x.saturating_add(cell.cell_width().max(1));
                if is_decorative(symbol) {
                    segment.push(x, " ");
                } else {
                    segment.push(x, symbol);
                }
            }
            segment.flush(y, order_base, &mut items);
        }

        // Each item belongs to the innermost container that holds it.
        let parent_of = |item: &Item| -> Option<usize> {
            let own = match item.content {
                Content::Node(index) => Some(index),
                Content::Text(_) => None,
            };
            containers
                .iter()
                .copied()
                .filter(|&candidate| Some(candidate) != own)
                .filter(|&candidate| {
                    let rect = nodes[candidate].area;
                    match own {
                        // A container nests in another that encloses it; on a
                        // tie, the one painted first is the outer.
                        Some(index) => {
                            let inner = nodes[index].area;
                            rect.intersection(inner) == inner
                                && (rect.area() > inner.area() || candidate < index)
                        }
                        None => rect.contains(item.anchor),
                    }
                })
                .min_by_key(|&candidate| (nodes[candidate].area.area(), usize::MAX - candidate))
        };
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len() + 1];
        for (index, item) in items.iter().enumerate() {
            let slot = parent_of(item).unwrap_or(nodes.len());
            children[slot].push(index);
        }
        for list in &mut children {
            list.sort_by_key(|&index| {
                let item = &items[index];
                (item.anchor.y, item.anchor.x, item.order)
            });
        }

        let mut lines = Vec::new();
        // (item, label of the container it sits in)
        let mut stack: Vec<(usize, Option<&str>)> = children[nodes.len()]
            .iter()
            .rev()
            .map(|&index| (index, None))
            .collect();
        while let Some((index, enclosing)) = stack.pop() {
            match &items[index].content {
                Content::Text(text) => lines.push(Line {
                    text: text.clone(),
                    node: None,
                }),
                Content::Node(node_index) => {
                    let node = &nodes[*node_index];
                    let text = node.describe();
                    // A titled panel inside a dialog of the same name would
                    // otherwise announce the title twice.
                    let repeats = node.role == Role::Group
                        && enclosing.is_some_and(|outer| {
                            collapse_whitespace(outer) == collapse_whitespace(&node.label)
                        });
                    if !text.is_empty() && !repeats {
                        lines.push(Line {
                            text,
                            node: Some(node.clone()),
                        });
                    }
                    if node.role.is_container() {
                        let label = Some(node.label.as_str());
                        stack.extend(
                            children[*node_index]
                                .iter()
                                .rev()
                                .map(|&child| (child, label)),
                        );
                    }
                }
            }
        }
        Document { lines }
    }
}

/// Per-cell lookups for one [`AccessFrame::document`] pass, painted once from
/// the rects so the scan is linear in cells rather than cells × nodes.
struct Masks {
    area: Rect,
    readable: Vec<bool>,
    /// Innermost container under each cell. A text run stops at a container
    /// edge so side-by-side panels are not read as one line.
    container: Vec<Option<usize>>,
}

impl Masks {
    fn build(
        area: Rect,
        nodes: &[Node],
        containers: &[usize],
        hidden: &[Rect],
        visible: Option<&[Rect]>,
    ) -> Self {
        let cells = usize::from(area.width) * usize::from(area.height);
        let mut masks = Self {
            area,
            readable: vec![visible.is_none(); cells],
            container: vec![None; cells],
        };
        for rect in visible.unwrap_or_default() {
            masks.paint(*rect, |readable, _| *readable = true);
        }
        let blocked = hidden.iter().copied().chain(
            nodes
                .iter()
                .filter(|node| !node.role.is_container())
                .map(|node| node.area),
        );
        for rect in blocked {
            masks.paint(rect, |readable, _| *readable = false);
        }
        // Larger first so the smallest wins; on a tie the later-painted one.
        let mut ordered = containers.to_vec();
        ordered.sort_by_key(|&index| (std::cmp::Reverse(nodes[index].area.area()), index));
        for index in ordered {
            masks.paint(nodes[index].area, |_, container| *container = Some(index));
        }
        masks
    }

    fn paint(&mut self, rect: Rect, mut apply: impl FnMut(&mut bool, &mut Option<usize>)) {
        let rect = rect.intersection(self.area);
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                let index = self.index(x, y);
                apply(&mut self.readable[index], &mut self.container[index]);
            }
        }
    }

    fn index(&self, x: u16, y: u16) -> usize {
        usize::from(y - self.area.y) * usize::from(self.area.width) + usize::from(x - self.area.x)
    }

    fn readable(&self, x: u16, y: u16) -> bool {
        self.readable[self.index(x, y)]
    }

    fn container(&self, x: u16, y: u16) -> Option<usize> {
        self.container[self.index(x, y)]
    }
}

struct Item {
    anchor: Position,
    order: usize,
    content: Content,
}

enum Content {
    Node(usize),
    Text(String),
}

/// A run of readable cells on one row.
#[derive(Default)]
struct Segment {
    start: Option<u16>,
    text: String,
}

impl Segment {
    fn push(&mut self, x: u16, symbol: &str) {
        if self.start.is_none() && !symbol.trim().is_empty() {
            self.start = Some(x);
        }
        if self.start.is_some() {
            self.text.push_str(symbol);
        }
    }

    fn flush(&mut self, y: u16, order_base: usize, items: &mut Vec<Item>) {
        if let Some(x) = self.start.take() {
            let text = collapse_whitespace(&self.text);
            if !text.is_empty() {
                items.push(Item {
                    anchor: Position::new(x, y),
                    order: order_base + items.len(),
                    content: Content::Text(text),
                });
            }
        }
        self.text.clear();
    }
}

/// Glyphs that draw rather than say: box drawing, block elements, Braille
/// patterns, and the legacy-computing sextants and octants.
fn is_decorative(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    chars.next().is_none()
        && matches!(
            first,
            '\u{2500}'..='\u{259F}' | '\u{2800}'..='\u{28FF}' | '\u{1FB00}'..='\u{1FBFF}'
        )
}

/// The text painted in `area`, row by row, with decoration dropped.
fn cells_text(buffer: &Buffer, area: Rect) -> String {
    let mut text = String::new();
    for y in area.top()..area.bottom() {
        let mut covered_until = 0u16;
        for x in area.left()..area.right() {
            let cell = &buffer[(x, y)];
            if x < covered_until || cell.raw_symbol().is_empty() {
                continue;
            }
            covered_until = x.saturating_add(cell.cell_width().max(1));
            let symbol = cell.symbol();
            if cell.is_skipped() || is_decorative(symbol) {
                text.push(' ');
            } else {
                text.push_str(symbol);
            }
        }
        text.push(' ');
    }
    collapse_whitespace(&text)
}

/// The plain text of a styled line, for a node label.
pub(crate) fn line_text(line: &crate::text::Line) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        // Control characters never reach the output: a label is data, and the
        // document may be written straight to a terminal.
        out.extend(word.chars().filter(|c| !c.is_control()));
    }
    out
}

/// A frame read back as plain text, one [`Line`] per spoken unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    lines: Vec<Line>,
}

impl Document {
    /// The lines in reading order.
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// The node that holds keyboard focus, if any recorded one does.
    pub fn focused(&self) -> Option<&Node> {
        self.lines
            .iter()
            .filter_map(|line| line.node.as_ref())
            .find(|node| node.focused)
    }
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 {
                f.write_str("\n")?;
            }
            f.write_str(&line.text)?;
        }
        Ok(())
    }
}

/// One line of a [`Document`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Line {
    /// The text to speak. Never contains a control character.
    pub text: String,
    /// The node the line describes, or `None` for text read from cells.
    pub node: Option<Node>,
}

/// The environment variable that turns screen-reader output on for any tuika
/// app whose runner is left at [`ScreenReader::Auto`].
pub const SCREEN_READER_ENV: &str = "TUIKA_SCREEN_READER";

/// Whether a runner speaks to a screen reader instead of painting cells.
///
/// In screen-reader mode a [`Runner`](crate::Runner) or
/// [`AsyncRunner`](crate::AsyncRunner) stays on the main screen with the cursor
/// visible and no mouse capture, paints each frame off screen, and writes only
/// what changed as plain lines through a [`Narrator`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScreenReader {
    /// On when [`SCREEN_READER_ENV`] is set to anything but empty, `0`,
    /// `false`, `no`, or `off`. The default, so a user can turn the mode on
    /// for any tuika app without the app doing anything.
    #[default]
    Auto,
    /// Always on.
    On,
    /// Always off, even when the environment asks for it.
    Off,
}

impl ScreenReader {
    /// Whether the mode is on for this process.
    pub fn is_active(self) -> bool {
        match self {
            Self::On => true,
            Self::Off => false,
            Self::Auto => {
                std::env::var(SCREEN_READER_ENV).is_ok_and(|value| Self::env_value_enables(&value))
            }
        }
    }

    fn env_value_enables(value: &str) -> bool {
        let value = value.trim();
        !value.is_empty()
            && !["0", "false", "no", "off"]
                .iter()
                .any(|off| value.eq_ignore_ascii_case(off))
    }
}

/// Turns successive [`Document`]s into the lines a screen reader should speak.
///
/// A screen reader speaks text as it arrives, so repainting a frame would make
/// it read the whole screen on every keystroke. A narrator remembers the
/// previous document and writes only:
///
/// 1. new alerts, first;
/// 2. the focused node, when focus moved to a different one;
/// 3. every other line that is new or changed, in reading order.
///
/// A focused [`Role::TextInput`] is the exception: its line is rewritten in
/// place (carriage return, clear line) as the user types, and committed with a
/// newline when focus leaves it. Lines end in `\r\n` so the output is right
/// in raw mode too.
///
/// ```
/// use tuika::access::{Narrator, Role, Semantic};
/// use tuika::components::Text;
/// use tuika::Theme;
///
/// let theme = Theme::default();
/// let mut narrator = Narrator::new();
/// let mut out = Vec::new();
/// let status = |text| Semantic::new(Text::raw(text)).role(Role::Status);
/// let first = tuika::access::read(&status("Building"), 20, 1, &theme);
/// narrator.narrate(&first, &mut out).unwrap();
/// let same = tuika::access::read(&status("Building"), 20, 1, &theme);
/// narrator.narrate(&same, &mut out).unwrap();
/// let done = tuika::access::read(&status("Done"), 20, 1, &theme);
/// narrator.narrate(&done, &mut out).unwrap();
/// assert_eq!(String::from_utf8(out).unwrap(), "Building\r\nDone\r\n");
/// ```
#[derive(Debug, Default)]
pub struct Narrator {
    previous: Vec<String>,
    /// Role and label of the node that held focus.
    focus: Option<(Role, String)>,
    /// The text input line currently shown without a newline.
    editing: Option<String>,
}

impl Narrator {
    /// A narrator that has spoken nothing yet: the first document is written
    /// whole.
    pub fn new() -> Self {
        Self::default()
    }

    /// Write what changed between the previous document and `document`.
    pub fn narrate(
        &mut self,
        document: &Document,
        out: &mut (impl Write + ?Sized),
    ) -> io::Result<()> {
        let focused = document.focused();
        let focus = focused.map(|node| (node.role, node.label.clone()));
        let focus_moved = focus.is_some() && focus != self.focus;
        let editing_index = document.lines.iter().position(|line| {
            line.node
                .as_ref()
                .is_some_and(|node| node.focused && node.role == Role::TextInput)
        });
        let editing = editing_index.map(|index| document.lines[index].text.clone());

        // Lines not in the previous document, counted as a multiset so a
        // repeated line is new only as often as it is added.
        let mut remaining: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for line in &self.previous {
            *remaining.entry(line.as_str()).or_default() += 1;
        }
        let mut fresh: Vec<usize> = Vec::new();
        for (index, line) in document.lines.iter().enumerate() {
            match remaining.get_mut(line.text.as_str()) {
                Some(count) if *count > 0 => *count -= 1,
                _ if Some(index) == editing_index => {}
                _ => fresh.push(index),
            }
        }

        let mut spoken: Vec<&str> = Vec::new();
        let is_alert = |index: &usize| {
            document.lines[*index]
                .node
                .as_ref()
                .is_some_and(|node| node.role == Role::Alert)
        };
        spoken.extend(
            fresh
                .iter()
                .filter(|i| is_alert(i))
                .map(|&i| document.lines[i].text.as_str()),
        );
        if focus_moved
            && editing_index.is_none()
            && let Some(index) = document
                .lines
                .iter()
                .position(|line| line.node.as_ref().is_some_and(|node| node.focused))
            && !is_alert(&index)
        {
            spoken.push(document.lines[index].text.as_str());
            fresh.retain(|&i| i != index);
        }
        spoken.extend(
            fresh
                .iter()
                .filter(|i| !is_alert(i))
                .map(|&i| document.lines[i].text.as_str()),
        );

        let editing_changed = editing != self.editing;
        if self.editing.is_some() && (!spoken.is_empty() || editing_changed) {
            if editing.is_some() {
                // Still typing: take the line back, speak, then redraw it.
                out.write_all(b"\r\x1b[2K")?;
            } else {
                // Focus left the input: its last state stays on screen.
                out.write_all(b"\r\n")?;
            }
        }
        for line in &spoken {
            out.write_all(line.as_bytes())?;
            out.write_all(b"\r\n")?;
        }
        if let Some(text) = &editing
            && (editing_changed || !spoken.is_empty() || self.editing.is_none())
        {
            out.write_all(text.as_bytes())?;
        }
        out.flush()?;

        self.previous = document
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect();
        self.focus = focus.or(self.focus.take().filter(|_| focused.is_some()));
        self.editing = editing;
        Ok(())
    }

    /// Write every line of `document`, unconditionally: output that is
    /// published once, such as a block committed to scrollback, rather than a
    /// frame that is repainted. An open text input line is taken back first
    /// and redrawn after, so the announcement never lands mid-line.
    pub fn announce(
        &mut self,
        document: &Document,
        out: &mut (impl Write + ?Sized),
    ) -> io::Result<()> {
        if document.lines.is_empty() {
            return Ok(());
        }
        if self.editing.is_some() {
            out.write_all(b"\r\x1b[2K")?;
        }
        for line in &document.lines {
            out.write_all(line.text.as_bytes())?;
            out.write_all(b"\r\n")?;
        }
        if let Some(text) = &self.editing {
            out.write_all(text.as_bytes())?;
        }
        out.flush()
    }

    /// End the session: commit a text input line left open, so the shell
    /// prompt starts on a line of its own.
    pub fn finish(&mut self, out: &mut (impl Write + ?Sized)) -> io::Result<()> {
        if self.editing.take().is_some() {
            out.write_all(b"\r\n")?;
            out.flush()?;
        }
        Ok(())
    }
}

/// Paint `view` at `width × height` and read it back as a [`Document`].
///
/// The one-call form of installing an [`AccessFrame`], painting, and calling
/// [`AccessFrame::document`]. [`testing::screen_reader`](crate::testing::screen_reader)
/// is the same, as text.
pub fn read(view: &dyn View, width: u16, height: u16, theme: &crate::Theme) -> Document {
    let frame = AccessFrame::new();
    let ctx = RenderCtx::new(theme).with_access(&frame);
    let buffer = crate::testing::render_with_context(view, width, height, &ctx);
    frame.document(&buffer)
}

/// Give a view screen-reader semantics without changing how it looks.
///
/// Layout-transparent: it measures and paints exactly as the wrapped view. With
/// a [`role`](Self::role) it records a [`Node`] over the area it was painted
/// into; a leaf role replaces the cells' text with the node's line, a container
/// role reads them after it. [`hidden`](Self::hidden) marks the view as
/// decoration instead: nothing inside it is read, including nodes its
/// children record.
pub struct Semantic<V: View = Element> {
    inner: V,
    node: Option<Node>,
    hidden: bool,
}

impl<V: View> Semantic<V> {
    /// Wrap `inner` as plain [`Role::Text`] with no label of its own. Set at
    /// least a [`role`](Self::role) or a [`label`](Self::label).
    pub fn new(inner: V) -> Self {
        Self {
            inner,
            node: Some(Node::new(Role::Text, "")),
            hidden: false,
        }
    }

    /// Wrap `inner` as decoration: painted, never read.
    pub fn hidden(inner: V) -> Self {
        Self {
            inner,
            node: None,
            hidden: true,
        }
    }

    /// Set the role.
    pub fn role(mut self, role: Role) -> Self {
        if let Some(node) = self.node.as_mut() {
            node.role = role;
        }
        self
    }

    /// Set the accessible name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        if let Some(node) = self.node.as_mut() {
            node.label = label.into();
        }
        self
    }

    /// Replace the whole node; its area is ignored and filled in at paint.
    pub fn node(mut self, node: Node) -> Self {
        if !self.hidden {
            self.node = Some(node);
        }
        self
    }
}

impl<V: View> View for Semantic<V> {
    fn measure(&self, available: Size, ctx: &RenderCtx) -> Size {
        self.inner.measure(available, ctx)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        let visible = area.intersection(surface.area());
        if self.hidden {
            if let Some(frame) = ctx.access() {
                frame.hide(visible);
            }
            self.inner.render(area, surface, &ctx.without_access());
            return;
        }
        if let Some(node) = &self.node {
            // Recorded before the children, so a container precedes what it
            // contains when two anchors tie.
            ctx.record_access(|| Node {
                area: visible,
                ..node.clone()
            });
        }
        self.inner.render(area, surface, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Boxed, Text};
    use crate::style::Theme;
    use crate::testing;
    use crate::view::view_fn;

    /// A view painting one string per row.
    fn rows(lines: &'static [&'static str]) -> impl View {
        view_fn(
            move |_, _| Size::new(10, lines.len() as u16),
            move |area, surface, _| {
                for (offset, line) in lines.iter().enumerate() {
                    let style = crate::style::Style::default();
                    surface.set_string(area.x, area.y + offset as u16, line, style);
                }
            },
        )
    }

    fn read(view: &dyn View, width: u16, height: u16) -> String {
        testing::screen_reader(view, width, height, &Theme::default())
    }

    #[test]
    fn unlabelled_views_are_read_from_their_cells_without_decoration() {
        let boxed = Boxed::new(Text::raw("hello   world")).title("Greeting");
        let spoken = read(&boxed, 20, 3);
        assert_eq!(spoken, "Greeting\nhello world");
    }

    #[test]
    fn a_leaf_node_replaces_the_cells_it_covers() {
        let button = Semantic::new(Text::raw("[ ⏎ ]"))
            .role(Role::Button)
            .label("Send");
        assert_eq!(read(&button, 10, 1), "Send, button");
    }

    #[test]
    fn a_container_reads_its_contents_after_its_own_line() {
        let list = Semantic::new(rows(&["one", "two"]))
            .role(Role::List)
            .label("Numbers");
        assert_eq!(read(&list, 10, 2), "Numbers, list\none\ntwo");
    }

    #[test]
    fn hidden_views_and_their_nodes_are_never_read() {
        let spinner = Semantic::hidden(Semantic::new(Text::raw("⠋ working")).label("Spinner"));
        assert_eq!(read(&spinner, 12, 1), "");
    }

    #[test]
    fn describe_orders_label_role_value_then_state() {
        let node = Node::new(Role::ListItem, "Sonnet")
            .position(2, 5)
            .selected(true);
        assert_eq!(node.describe(), "Sonnet, 2 of 5, selected");
        let node = Node::new(Role::Checkbox, "Remember me").checked(false);
        assert_eq!(node.describe(), "Remember me, checkbox, not checked");
        let node = Node::new(Role::Heading, "Usage").level(2);
        assert_eq!(node.describe(), "Usage, heading level 2");
        let node = Node::new(Role::TextInput, "Prompt").value("");
        assert_eq!(node.describe(), "Prompt, text input, blank");
        let node = Node::new(Role::ProgressBar, "Build")
            .value("40%")
            .busy(true)
            .disabled(true)
            .expanded(false);
        assert_eq!(
            node.describe(),
            "Build, progress bar, 40%, collapsed, unavailable, busy"
        );
    }

    #[test]
    fn labels_never_carry_control_characters() {
        let node = Node::new(Role::Button, "Se\u{1b}[2Jnd\u{7}");
        assert_eq!(node.describe(), "Se[2Jnd, button");
    }

    #[test]
    fn focus_is_reported_by_the_document() {
        let theme = Theme::default();
        let frame = AccessFrame::new();
        let view = Semantic::new(Text::raw("x")).node(Node::new(Role::Button, "OK").focused(true));
        let ctx = RenderCtx::new(&theme).with_access(&frame);
        let buffer = testing::render_with_context(&view, 4, 1, &ctx);
        let document = frame.document(&buffer);
        assert_eq!(
            document.focused().map(|node| node.label.as_str()),
            Some("OK")
        );
    }

    #[test]
    fn side_by_side_containers_are_read_one_after_the_other() {
        let column = |label: &'static str, body: &'static [&'static str]| {
            Semantic::new(rows(body)).role(Role::Group).label(label)
        };
        let left = column("Left", &["a1", "a2"]);
        let right = column("Right", &["b1", "b2"]);
        let view = view_fn(
            |_, _| Size::new(10, 2),
            move |area, surface, ctx| {
                let half = area.width / 2;
                left.render(Rect::new(area.x, area.y, half, area.height), surface, ctx);
                right.render(
                    Rect::new(area.x + half, area.y, area.width - half, area.height),
                    surface,
                    ctx,
                );
            },
        );
        assert_eq!(read(&view, 10, 2), "Left\na1\na2\nRight\nb1\nb2");
    }

    #[test]
    fn a_modal_overlay_makes_the_base_inert() {
        let frame = AccessFrame::new();
        frame.record(Node::new(Role::Button, "Behind").area(Rect::new(0, 0, 4, 1)));
        frame.begin_overlay(Rect::new(0, 1, 4, 1), true);
        frame.record(Node::new(Role::Button, "Front").area(Rect::new(0, 1, 4, 1)));
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 2));
        buffer.set_string(5, 0, "base", crate::style::Style::default());
        assert_eq!(frame.document(&buffer).to_string(), "Front, button");
    }

    #[test]
    fn a_dimmed_scene_overlay_is_read_alone() {
        use crate::overlay::OverlaySpec;
        use crate::scene::{Backdrop, Scene, SceneOverlay};
        let base = Semantic::new(Text::raw("Quit")).role(Role::Button);
        let dialog = Semantic::new(Text::raw("Sure?"))
            .role(Role::Dialog)
            .label("Confirm");
        let scene = Scene::new(crate::element(base)).overlay(
            SceneOverlay::new(crate::element(dialog), OverlaySpec::centered(50, 34))
                .backdrop(Backdrop::Dim),
        );
        assert_eq!(read(&scene, 20, 3), "Confirm, dialog\nSure?");
    }

    #[test]
    fn an_overlay_drops_the_nodes_it_covers() {
        let frame = AccessFrame::new();
        frame.record(Node::new(Role::Button, "Covered").area(Rect::new(0, 0, 4, 1)));
        frame.record(Node::new(Role::Button, "Beside").area(Rect::new(4, 0, 4, 1)));
        frame.begin_overlay(Rect::new(0, 0, 4, 1), false);
        let buffer = Buffer::empty(Rect::new(0, 0, 8, 1));
        assert_eq!(frame.document(&buffer).to_string(), "Beside, button");
    }

    fn narrate(narrator: &mut Narrator, view: &dyn View) -> String {
        let document = super::read(view, 30, 4, &Theme::default());
        let mut out = Vec::new();
        narrator.narrate(&document, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn list(selected: usize) -> impl View {
        let items = ["Haiku", "Sonnet", "Opus"];
        view_fn(
            |_, _| Size::new(10, 3),
            move |area, surface, ctx| {
                for (index, item) in items.iter().enumerate() {
                    let row = Rect::new(area.x, area.y + index as u16, area.width, 1);
                    surface.set_string(row.x, row.y, item, crate::style::Style::default());
                    ctx.record_access(|| {
                        Node::new(Role::ListItem, *item)
                            .area(row)
                            .position(index + 1, items.len())
                            .selected(index == selected)
                            .focused(index == selected)
                    });
                }
            },
        )
    }

    #[test]
    fn a_narrator_speaks_the_first_frame_whole_then_only_changes() {
        let mut narrator = Narrator::new();
        assert_eq!(
            narrate(&mut narrator, &list(0)),
            "Haiku, 1 of 3, selected\r\nSonnet, 2 of 3\r\nOpus, 3 of 3\r\n"
        );
        assert_eq!(narrate(&mut narrator, &list(0)), "");
        // Moving the selection announces the newly focused item first, then
        // the item that lost its selection.
        assert_eq!(
            narrate(&mut narrator, &list(1)),
            "Sonnet, 2 of 3, selected\r\nHaiku, 1 of 3\r\n"
        );
    }

    #[test]
    fn a_narrator_speaks_alerts_before_everything_else() {
        let mut narrator = Narrator::new();
        narrate(&mut narrator, &rows(&["status: ok"]));
        let alert = view_fn(
            |_, _| Size::new(10, 2),
            |area, surface, ctx| {
                surface.set_string(
                    area.x,
                    area.y,
                    "status: bad",
                    crate::style::Style::default(),
                );
                ctx.record_access(|| {
                    Node::new(Role::Alert, "Disk full").area(Rect::new(area.x, area.y + 1, 9, 1))
                });
            },
        );
        assert_eq!(
            narrate(&mut narrator, &alert),
            "Disk full, alert\r\nstatus: bad\r\n"
        );
    }

    #[test]
    fn a_focused_text_input_is_rewritten_in_place_then_committed() {
        let input = |value: &'static str, focused: bool| {
            view_fn(
                |_, _| Size::new(10, 1),
                move |area, _, ctx| {
                    ctx.record_access(|| {
                        Node::new(Role::TextInput, "Prompt")
                            .value(value)
                            .area(area)
                            .focused(focused)
                    });
                },
            )
        };
        let mut narrator = Narrator::new();
        assert_eq!(
            narrate(&mut narrator, &input("", true)),
            "Prompt, text input, blank"
        );
        assert_eq!(
            narrate(&mut narrator, &input("h", true)),
            "\r\x1b[2KPrompt, text input, h"
        );
        assert_eq!(narrate(&mut narrator, &input("h", true)), "");
        // Focus leaves: the last state is committed and nothing repeats.
        assert_eq!(narrate(&mut narrator, &input("h", false)), "\r\n");
        let mut out = Vec::new();
        narrator.finish(&mut out).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn finishing_commits_an_open_input_line() {
        let input = view_fn(
            |_, _| Size::new(10, 1),
            |area, _, ctx| {
                ctx.record_access(|| Node::new(Role::TextInput, "").area(area).focused(true));
            },
        );
        let mut narrator = Narrator::new();
        narrate(&mut narrator, &input);
        let mut out = Vec::new();
        narrator.finish(&mut out).unwrap();
        assert_eq!(out, b"\r\n");
    }

    #[test]
    fn the_environment_switch_reads_common_spellings() {
        for on in ["1", "true", "yes", "ON"] {
            assert!(ScreenReader::env_value_enables(on), "{on}");
        }
        for off in ["", " ", "0", "false", "No", "off"] {
            assert!(!ScreenReader::env_value_enables(off), "{off:?}");
        }
        assert!(ScreenReader::On.is_active());
        assert!(!ScreenReader::Off.is_active());
    }

    #[test]
    fn wide_glyphs_read_without_a_gap() {
        assert_eq!(read(&Text::raw("漢字 ok"), 10, 1), "漢字 ok");
    }

    #[test]
    fn degenerate_sizes_read_nothing_and_do_not_panic() {
        let view = Semantic::new(Boxed::new(Text::raw("x")).title("t"))
            .role(Role::Dialog)
            .label("D");
        for (width, height) in [(0, 0), (0, 3), (3, 0), (1, 1)] {
            let _ = read(&view, width, height);
        }
        assert_eq!(read(&view, 0, 0), "");
    }
}

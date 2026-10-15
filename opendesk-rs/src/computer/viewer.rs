//! A window's text as plain lines, the way a screen reader's virtual viewer
//! shows it (ported from Okur's Virtualize Window, modelled on JAWS):
//!
//! - **Classic windows** (Win32 dialogs, WinForms, File Explorer) by screen
//!   rows: the title, then each row's text left to right, with labels beside
//!   their values and buttons on their rows.
//! - **Modern apps** (UWP/WinUI/WPF, e.g. Calculator) one element per line in
//!   tree order, with role and state: "Memory recall Button  unavailable".
//!
//! Far cheaper to read than a screenshot, and closer to what is on screen
//! than an accessibility tree dump. The layout half is pure (items in, lines
//! out); the walker below turns an xa11y element tree into items.

use anyhow::{Result, anyhow};
use std::time::Duration;
use xa11y::{App, AppExt, Element, Role, Toggled};

use super::local::LocalComputer;

/// How a window's elements become lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerLayout {
    /// As drawn on screen, by rows (classic windows).
    Rows,
    /// One element per line in tree order, with role and state (modern apps).
    Elements,
}

/// What an item is, as far as the viewer cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Text,
    Button,
    MenuButton,
    CheckBox,
    RadioButton,
    ComboBox,
    Edit,
    Tab,
    Group,
    Link,
    MenuItem,
    Slider,
    ProgressBar,
    List,
    ListItem,
    Tree,
    TreeItem,
    Other,
}

/// An element with text, where it is on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenItem {
    pub kind: ItemKind,
    /// Its name, or its value for a field.
    pub text: String,
    /// The text as the control draws it, one entry per line, when known.
    pub lines: Vec<String>,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    /// A group box: only its caption, at its top, is text.
    pub caption: bool,
    pub unavailable: bool,
    /// For a check box or radio button.
    pub checked: Option<bool>,
}

impl ScreenItem {
    pub fn new(kind: ItemKind, text: impl Into<String>, rect: (i32, i32, i32, i32)) -> Self {
        let (left, top, right, bottom) = rect;
        Self {
            kind,
            text: text.into(),
            lines: Vec::new(),
            left,
            top,
            right,
            bottom,
            caption: false,
            unavailable: false,
            checked: None,
        }
    }

    fn height(&self) -> i32 {
        (self.bottom - self.top).max(1)
    }
}

/// One line of text at its place: what a row is made of.
#[derive(Debug, Clone)]
struct Run {
    left: i32,
    top: i32,
    bottom: i32,
    text: String,
}

/// `text` without invisible direction/join marks (Explorer wraps dates in
/// them), which would show up as stray characters.
fn visible(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
            )
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The height of one line of text in the window: the median height of its
/// one-line items.
fn line_height(items: &[ScreenItem]) -> i32 {
    let mut heights: Vec<i32> = items
        .iter()
        .filter(|i| i.lines.len() <= 1 && !i.caption)
        .map(ScreenItem::height)
        .filter(|h| *h >= 8)
        .collect();
    heights.sort_unstable();
    heights.get(heights.len() / 2).copied().unwrap_or(20)
}

/// The window's text by screen rows: each item's lines placed where they are
/// drawn, rows top to bottom, each row left to right. A row's band is its
/// first line's, so a tall item (a group box, wrapped text) never pulls the
/// rows beside it into one.
pub fn rows(items: &[ScreenItem]) -> Vec<String> {
    let line = line_height(items);
    let mut runs: Vec<Run> = Vec::new();
    for item in items {
        let text = visible(&item.text);
        if text.is_empty() {
            continue;
        }
        if item.caption {
            runs.push(Run {
                left: item.left,
                top: item.top,
                bottom: item.top + line,
                text,
            });
            continue;
        }
        let lines: Vec<String> = if item.lines.len() > 1 {
            item.lines.iter().map(|l| visible(l)).collect()
        } else {
            text.lines()
                .map(visible)
                .filter(|l| !l.is_empty())
                .collect()
        };
        let count = i32::try_from(lines.len()).unwrap_or(1).max(1);
        let each = if lines.len() > 1 {
            item.height() / count
        } else if item.kind == ItemKind::Text {
            // A label draws its line at its top, however tall it is.
            item.height().min(line)
        } else {
            item.height().min(line * 2)
        };
        for (i, part) in lines.into_iter().enumerate() {
            let top = item.top + each * i32::try_from(i).unwrap_or(0);
            runs.push(Run {
                left: item.left,
                top,
                bottom: top + each,
                text: part,
            });
        }
    }
    runs.sort_by_key(|r| (r.top, r.left));
    let mut grouped: Vec<(i32, i32, Vec<Run>)> = Vec::new();
    for run in runs {
        let middle = run.top + (run.bottom - run.top) / 2;
        match grouped
            .iter_mut()
            .find(|(top, bottom, _)| (*top..*bottom).contains(&middle))
        {
            Some((_, _, row)) => row.push(run),
            None => grouped.push((run.top, run.bottom.max(run.top + 1), vec![run])),
        }
    }
    grouped
        .into_iter()
        .map(|(_, _, mut row)| {
            row.sort_by_key(|r| r.left);
            row.iter()
                .map(|r| r.text.as_str())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|r| !r.is_empty())
        .collect()
}

/// The word an element list gives a role ("Button", "group"); text has none.
fn role_word(kind: ItemKind) -> Option<&'static str> {
    Some(match kind {
        ItemKind::Button => "Button",
        ItemKind::MenuButton => "button menu",
        ItemKind::CheckBox => "check box",
        ItemKind::RadioButton => "radio button",
        ItemKind::ComboBox => "Combo box",
        ItemKind::Edit => "Edit",
        ItemKind::Tab => "Tab",
        ItemKind::Group => "group",
        ItemKind::Link => "link",
        ItemKind::MenuItem => "menu item",
        ItemKind::Slider => "slider",
        ItemKind::ProgressBar => "progress bar",
        ItemKind::List => "list",
        ItemKind::ListItem => "list item",
        ItemKind::Tree => "tree view",
        ItemKind::TreeItem => "tree item",
        ItemKind::Text | ItemKind::Other => return None,
    })
}

/// The window's elements one per line in tree order: name, role and state
/// ("Memory recall Button  unavailable"). Unnamed items are left out.
pub fn elements(items: &[ScreenItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| {
            let mut line = visible(&item.text);
            if line.is_empty() {
                return None;
            }
            if let Some(word) = role_word(item.kind) {
                line.push(' ');
                line.push_str(word);
            }
            match item.checked {
                Some(true) => line.push_str("  checked"),
                Some(false) if item.kind == ItemKind::CheckBox => line.push_str("  not checked"),
                _ => {}
            }
            if item.unavailable {
                line.push_str("  unavailable");
            }
            Some(line)
        })
        .collect()
}

/// The lines for a window titled `title`. An app that draws its own title
/// bar shows its title again at the top; it is listed once.
pub fn lines(title: Option<&str>, layout: ViewerLayout, items: &[ScreenItem]) -> Vec<String> {
    let title = title.map(visible).filter(|t| !t.is_empty());
    let mut body = match layout {
        ViewerLayout::Rows => rows(items),
        ViewerLayout::Elements => elements(items),
    };
    if title.is_some() && body.first() == title.as_ref() {
        body.remove(0);
    }
    title.into_iter().chain(body).collect()
}

// ---------------------------------------------------------------------------
// Walking the live accessibility tree
// ---------------------------------------------------------------------------

/// Upper bound on elements visited per read.
const MAX_ELEMENTS: usize = 2000;

// UIA control type ids (UIAutomationClient.h) that xa11y folds into broader
// roles but the viewer treats specially.
const UIA_TITLE_BAR: i64 = 50037;
const UIA_DOCUMENT: i64 = 50030;
const UIA_HEADER_ITEM: i64 = 50035;
const UIA_SPLIT_BUTTON: i64 = 50031;
const UIA_TAB_ITEM: i64 = 50019;
const UIA_DATA_ITEM: i64 = 50029;

fn raw_i64(el: &Element, key: &str) -> Option<i64> {
    el.raw.get(key).and_then(|v| v.as_i64())
}

fn class_name(el: &Element) -> &str {
    el.raw
        .get("class_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
}

fn kind_of(el: &Element) -> ItemKind {
    match raw_i64(el, "control_type_id") {
        Some(UIA_SPLIT_BUTTON) => return ItemKind::MenuButton,
        Some(UIA_TAB_ITEM) => return ItemKind::Tab,
        _ => {}
    }
    match el.role {
        Role::StaticText | Role::Heading => ItemKind::Text,
        Role::Button => ItemKind::Button,
        Role::CheckBox | Role::Switch => ItemKind::CheckBox,
        Role::RadioButton => ItemKind::RadioButton,
        Role::ComboBox => ItemKind::ComboBox,
        Role::TextField | Role::TextArea | Role::SpinButton => ItemKind::Edit,
        Role::Tab => ItemKind::Tab,
        Role::Group => ItemKind::Group,
        Role::Link => ItemKind::Link,
        Role::MenuItem => ItemKind::MenuItem,
        Role::Slider => ItemKind::Slider,
        Role::ProgressBar => ItemKind::ProgressBar,
        Role::List => ItemKind::List,
        Role::ListItem | Role::TableRow => ItemKind::ListItem,
        Role::TreeItem => ItemKind::TreeItem,
        _ => ItemKind::Other,
    }
}

fn rect(el: &Element) -> (i32, i32, i32, i32) {
    el.bounds
        .map(|b| (b.x, b.y, b.x + b.width as i32, b.y + b.height as i32))
        .unwrap_or_default()
}

fn item(el: &Element, kind: ItemKind, text: String) -> ScreenItem {
    let mut it = ScreenItem::new(kind, text, rect(el));
    it.unavailable = !el.states.enabled;
    it.checked = match el.states.checked {
        Some(Toggled::On) => Some(true),
        Some(Toggled::Off) => Some(false),
        _ if kind == ItemKind::RadioButton => Some(el.states.selected),
        _ => None,
    };
    it
}

fn name(el: &Element) -> String {
    el.name.clone().unwrap_or_default()
}

/// Classic (Rows) or modern (Elements) layout, from the window class.
fn layout_of(root: &Element) -> ViewerLayout {
    let class = class_name(root);
    if matches!(
        class,
        "ApplicationFrameWindow" | "Windows.UI.Core.CoreWindow" | "WinUIDesktopWin32WindowClass"
    ) || class.starts_with("HwndWrapper")
    {
        ViewerLayout::Elements
    } else {
        ViewerLayout::Rows
    }
}

fn skipped(el: &Element) -> bool {
    matches!(
        el.role,
        Role::ScrollBar | Role::ScrollThumb | Role::Tooltip | Role::Separator | Role::Image
    ) || raw_i64(el, "control_type_id") == Some(UIA_TITLE_BAR)
        || class_name(el) == "ApplicationFrameTitleBarWindow"
        // Offscreen elements and everything under them are not on screen.
        || !el.states.visible
}

struct Walk {
    budget: usize,
    items: Vec<ScreenItem>,
}

impl Walk {
    fn children(&mut self, el: &Element) -> Vec<Element> {
        if self.budget == 0 {
            return Vec::new();
        }
        el.children().unwrap_or_default()
    }

    /// Rows mode: what each element contributes, per Okur's rules.
    fn rows(&mut self, el: &Element) {
        if self.budget == 0 || skipped(el) {
            return;
        }
        self.budget -= 1;
        let kind = kind_of(el);
        let ct = raw_i64(el, "control_type_id");
        if ct == Some(UIA_DOCUMENT) {
            return; // a document's text is not "the window"
        }
        match kind {
            // Fields show their value, not their parts.
            ItemKind::Edit | ItemKind::ComboBox => {
                let v = el.value.clone().unwrap_or_default();
                self.items.push(item(el, kind, v));
            }
            ItemKind::Group => {
                // A Win32 group box (class Button): its caption is a line.
                if class_name(el) == "Button" && !name(el).is_empty() {
                    let mut it = item(el, kind, name(el));
                    it.caption = true;
                    self.items.push(it);
                }
                for c in self.children(el) {
                    self.rows(&c);
                }
            }
            ItemKind::Text
            | ItemKind::Button
            | ItemKind::MenuButton
            | ItemKind::CheckBox
            | ItemKind::RadioButton
            | ItemKind::Link
            | ItemKind::MenuItem
            | ItemKind::Tab => {
                // A named control's parts are never read again.
                if name(el).is_empty() {
                    for c in self.children(el) {
                        self.rows(&c);
                    }
                } else {
                    self.items.push(item(el, kind, name(el)));
                }
            }
            _ if ct == Some(UIA_HEADER_ITEM) => {
                self.items.push(item(el, ItemKind::Text, name(el)));
            }
            ItemKind::ListItem | ItemKind::TreeItem => {
                // Cells first; the row's own name only if it has none.
                let before = self.items.len();
                for c in self.children(el) {
                    self.rows(&c);
                }
                if self.items.len() == before {
                    self.items.push(item(el, kind, name(el)));
                }
            }
            _ if ct == Some(UIA_DATA_ITEM) || el.role == Role::Status => {
                let before = self.items.len();
                for c in self.children(el) {
                    self.rows(&c);
                }
                if self.items.len() == before {
                    self.items.push(item(el, kind, name(el)));
                }
            }
            _ => {
                for c in self.children(el) {
                    self.rows(&c);
                }
            }
        }
    }

    /// Elements mode: interactive and text elements give their name and stop
    /// descending; groups give theirs and are always walked.
    fn elements(&mut self, el: &Element) {
        if self.budget == 0 || skipped(el) {
            return;
        }
        self.budget -= 1;
        let kind = kind_of(el);
        match kind {
            ItemKind::Group => {
                self.items.push(item(el, kind, name(el)));
                for c in self.children(el) {
                    self.elements(&c);
                }
            }
            ItemKind::Other | ItemKind::List | ItemKind::Tree => {
                for c in self.children(el) {
                    self.elements(&c);
                }
            }
            _ => self.items.push(item(el, kind, name(el))),
        }
    }
}

impl LocalComputer {
    /// The text of an app's front window (default: the foreground app), as
    /// viewer lines: title first.
    pub fn window_text(&self, app_name: Option<&str>) -> Result<Vec<String>> {
        let app = match app_name {
            Some(n) => Self::find_app(n)?,
            None => App::foreground(Duration::from_secs(2))
                .map_err(|e| anyhow!("no foreground app: {e}"))?,
        };
        let windows = app.windows().map_err(|e| anyhow!("no windows: {e}"))?;
        let root = windows
            .iter()
            .find(|w| w.states.active)
            .or_else(|| windows.first())
            .ok_or_else(|| anyhow!("'{}' has no window", app.name))?
            .clone();
        let layout = layout_of(&root);
        let mut walk = Walk {
            budget: MAX_ELEMENTS,
            items: Vec::new(),
        };
        for c in root.children().unwrap_or_default() {
            match layout {
                ViewerLayout::Rows => walk.rows(&c),
                ViewerLayout::Elements => walk.elements(&c),
            }
        }
        Ok(lines(root.name.as_deref(), layout, &walk.items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str, left: i32, top: i32, right: i32, bottom: i32) -> ScreenItem {
        ScreenItem::new(ItemKind::Text, t, (left, top, right, bottom))
    }

    /// Local Disk (C:) Properties, as UI Automation places its controls.
    #[test]
    fn a_property_page_reads_by_rows() {
        let tab = |t: &str, left: i32, top: i32, right: i32| {
            ScreenItem::new(ItemKind::Tab, t, (left, top, right, top + 24))
        };
        let button = |t: &str, left: i32, top: i32, right: i32, bottom: i32| {
            ScreenItem::new(ItemKind::Button, t, (left, top, right, bottom))
        };
        let mut allow = ScreenItem::new(
            ItemKind::CheckBox,
            "Allow files on this drive to have contents indexed in addition to file properties",
            (1459, 901, 1875, 941),
        );
        allow.lines = vec![
            "Allow files on this drive to have contents indexed in addition to file".to_owned(),
            "properties".to_owned(),
        ];
        let items = vec![
            tab("Security", 1445, 423, 1520),
            tab("Previous Versions", 1520, 423, 1660),
            tab("Quota", 1660, 423, 1720),
            tab("General", 1445, 447, 1520),
            tab("Tools", 1520, 447, 1580),
            tab("Hardware", 1580, 447, 1660),
            tab("Sharing", 1660, 447, 1730),
            text("Type:", 1459, 567, 1543, 585),
            text("Local Disk", 1557, 567, 1885, 595),
            text("File system:", 1459, 593, 1543, 611),
            text("NTFS", 1557, 593, 1885, 621),
            text("Used space:", 1487, 629, 1579, 647),
            text("440,534,675,456 bytes", 1583, 629, 1791, 647),
            text("410 GB", 1805, 629, 1875, 647),
            text("Drive C:", 1499, 811, 1845, 851),
            button("Details", 1777, 799, 1885, 827),
            ScreenItem::new(
                ItemKind::CheckBox,
                "Compress this drive to save disk space",
                (1459, 873, 1875, 893),
            ),
            allow,
            button("OK", 1587, 971, 1687, 999),
            button("Cancel", 1695, 971, 1795, 999),
            button("Apply", 1803, 971, 1903, 999),
        ];
        assert_eq!(
            lines(
                Some("Local Disk (C:) Properties"),
                ViewerLayout::Rows,
                &items
            ),
            vec![
                "Local Disk (C:) Properties",
                "Security Previous Versions Quota",
                "General Tools Hardware Sharing",
                "Type: Local Disk",
                "File system: NTFS",
                "Used space: 440,534,675,456 bytes 410 GB",
                "Drive C: Details",
                "Compress this drive to save disk space",
                "Allow files on this drive to have contents indexed in addition to file",
                "properties",
                "OK Cancel Apply",
            ]
        );
    }

    #[test]
    fn a_group_box_does_not_join_the_rows_inside_it() {
        let mut plan = ScreenItem::new(ItemKind::Group, "Plan", (12, 144, 312, 200));
        plan.caption = true;
        let items = vec![
            text("Name:", 12, 36, 102, 56),
            plan,
            ScreenItem::new(ItemKind::RadioButton, "Free", (24, 166, 104, 190)),
            ScreenItem::new(ItemKind::RadioButton, "Pro", (112, 166, 192, 190)),
            ScreenItem::new(ItemKind::RadioButton, "Team", (200, 166, 280, 190)),
        ];
        assert_eq!(rows(&items), vec!["Name:", "Plan", "Free Pro Team"]);
    }

    #[test]
    fn a_multi_line_field_reads_line_by_line() {
        let items = vec![
            text("Notes:", 12, 100, 90, 120),
            ScreenItem::new(
                ItemKind::Edit,
                "First line\r\nSecond line",
                (100, 100, 400, 140),
            ),
            text("Country:", 12, 150, 90, 170),
        ];
        assert_eq!(
            rows(&items),
            vec!["Notes: First line", "Second line", "Country:"]
        );
    }

    #[test]
    fn direction_marks_are_not_shown() {
        let items = vec![
            text("notes.txt", 10, 10, 200, 30),
            text(
                "\u{200F}\u{200E}15.10.2026 \u{200F}\u{200E}14:02",
                210,
                10,
                400,
                30,
            ),
        ];
        assert_eq!(rows(&items), vec!["notes.txt 15.10.2026 14:02"]);
    }

    /// Calculator on Windows 11: one element per line.
    #[test]
    fn a_modern_app_lists_its_elements() {
        let mut recall = ScreenItem::new(ItemKind::Button, "Memory recall", (0, 0, 10, 10));
        recall.unavailable = true;
        let items = vec![
            ScreenItem::new(ItemKind::Text, "Calculator", (0, 0, 10, 10)),
            ScreenItem::new(ItemKind::Button, "Open Navigation", (0, 0, 10, 10)),
            ScreenItem::new(ItemKind::Group, "", (0, 0, 10, 10)),
            ScreenItem::new(ItemKind::Text, "Display is 0", (0, 0, 10, 10)),
            ScreenItem::new(ItemKind::Group, "Memory controls", (0, 0, 10, 10)),
            recall,
        ];
        assert_eq!(
            lines(Some("Calculator"), ViewerLayout::Elements, &items),
            vec![
                "Calculator",
                "Open Navigation Button",
                "Display is 0",
                "Memory controls group",
                "Memory recall Button  unavailable",
            ]
        );
    }

    #[test]
    fn check_states_are_shown_in_element_lists() {
        let mut on = ScreenItem::new(ItemKind::CheckBox, "Wrap", (0, 0, 1, 1));
        on.checked = Some(true);
        let mut off = ScreenItem::new(ItemKind::CheckBox, "Bold", (0, 0, 1, 1));
        off.checked = Some(false);
        assert_eq!(
            elements(&[on, off]),
            vec!["Wrap check box  checked", "Bold check box  not checked"]
        );
    }
}

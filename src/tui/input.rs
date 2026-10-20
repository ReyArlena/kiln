//! Translate raw terminal events into app actions, and map mouse clicks
//! back onto whatever was drawn.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

/// Everything a screen can be asked to do. Navigation actions come from the
/// reserved keys; the rest are letters, Space, Enter and Esc.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    /// Tab: move focus to the next pane.
    NextPane,
    /// Shift-Tab: previous pane.
    PrevPane,
    Toggle,
    Confirm,
    Back,
    Quit,
    Help,
    /// A plain letter not covered above, e.g. `a` for select all.
    Char(char),
    /// Mouse: left click at a cell.
    Click {
        x: u16,
        y: u16,
    },
    /// Mouse wheel over a cell.
    Scroll {
        x: u16,
        y: u16,
        up: bool,
    },
    /// Terminal was resized.
    Resize,
    /// Raw key, for text editing mode.
    Key(KeyEvent),
}

pub fn map_event(event: Event) -> Option<Action> {
    match event {
        Event::Key(key) => Some(map_key(key)),
        Event::Mouse(mouse) => map_mouse(mouse),
        Event::Resize(_, _) => Some(Action::Resize),
        _ => None,
    }
}

fn map_key(key: KeyEvent) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Action::Quit;
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => Action::Up,
        KeyCode::Down | KeyCode::Char('j') => Action::Down,
        KeyCode::Left | KeyCode::Char('h') => Action::Left,
        KeyCode::Right | KeyCode::Char('l') => Action::Right,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::Home => Action::Home,
        KeyCode::End => Action::End,
        KeyCode::Tab => Action::NextPane,
        KeyCode::BackTab => Action::PrevPane,
        KeyCode::Char(' ') => Action::Toggle,
        KeyCode::Enter => Action::Confirm,
        KeyCode::Esc => Action::Back,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => Action::Help,
        KeyCode::Char(c) => Action::Char(c),
        _ => Action::Key(key),
    }
}

fn map_mouse(mouse: MouseEvent) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => Some(Action::Click {
            x: mouse.column,
            y: mouse.row,
        }),
        MouseEventKind::ScrollUp => Some(Action::Scroll {
            x: mouse.column,
            y: mouse.row,
            up: true,
        }),
        MouseEventKind::ScrollDown => Some(Action::Scroll {
            x: mouse.column,
            y: mouse.row,
            up: false,
        }),
        _ => None,
    }
}

/// Clickable regions registered while drawing. Rebuilt on every frame.
#[derive(Debug, Default, Clone)]
pub struct HitMap {
    regions: Vec<(Rect, Hit)>,
}

/// What a click on a region means. Screens interpret these.
#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    /// Row `index` of list/pane `pane`.
    Row { pane: usize, index: usize },
    /// A button or key hint labelled with a character (same as pressing it).
    Button(char),
    /// Somewhere inside pane `pane` (for focus and wheel scrolling).
    Pane(usize),
}

impl HitMap {
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    pub fn add(&mut self, area: Rect, hit: Hit) {
        self.regions.push((area, hit));
    }

    /// Most specific (last registered, smallest) hit under a cell.
    pub fn find(&self, x: u16, y: u16) -> Option<&Hit> {
        self.regions
            .iter()
            .rev()
            .filter(|(r, _)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            .min_by_key(|(r, _)| r.width as u32 * r.height as u32)
            .map(|(_, h)| h)
    }

    /// The pane under a cell, ignoring rows and buttons.
    pub fn pane_at(&self, x: u16, y: u16) -> Option<usize> {
        self.regions
            .iter()
            .rev()
            .filter(|(r, h)| {
                matches!(h, Hit::Pane(_))
                    && x >= r.x
                    && x < r.x + r.width
                    && y >= r.y
                    && y < r.y + r.height
            })
            .min_by_key(|(r, _)| r.width as u32 * r.height as u32)
            .and_then(|(_, h)| if let Hit::Pane(p) = h { Some(*p) } else { None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_and_arrow_keys_navigate_and_letters_pass_through() {
        let key = |c: KeyCode| map_key(KeyEvent::new(c, KeyModifiers::NONE));
        assert_eq!(key(KeyCode::Char('j')), Action::Down);
        assert_eq!(key(KeyCode::Up), Action::Up);
        assert_eq!(key(KeyCode::Char('a')), Action::Char('a'));
        assert_eq!(key(KeyCode::Char(' ')), Action::Toggle);
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    fn hit_map_prefers_the_smallest_region() {
        let mut map = HitMap::default();
        map.add(Rect::new(0, 0, 40, 20), Hit::Pane(0));
        map.add(Rect::new(2, 3, 30, 1), Hit::Row { pane: 0, index: 2 });
        assert_eq!(map.find(5, 3), Some(&Hit::Row { pane: 0, index: 2 }));
        assert_eq!(map.find(5, 10), Some(&Hit::Pane(0)));
        assert_eq!(map.find(50, 50), None);
        assert_eq!(map.pane_at(5, 3), Some(0));
    }
}

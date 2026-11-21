use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Open,
    Parent,
    Home,
    ToggleFocus,
    PlayPause,
    Next,
    Prev,
    VolumeUp,
    VolumeDown,
    SeekForward,
    SeekBackward,
    ToggleShuffle,
    CycleRepeat,
    StartFilter,
    ClearFilter,
    Remove,
    MoveDown,
    MoveUp,
    ToggleMode,
    Login,
    Quit,
}

/// Map a key press in normal mode to an action.
pub fn map_key(key: KeyEvent) -> Option<Action> {
    use Action::*;
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Some(Quit),
            KeyCode::Char('d') => Some(PageDown),
            KeyCode::Char('u') => Some(PageUp),
            _ => None,
        };
    }
    Some(match key.code {
        KeyCode::Char('j') | KeyCode::Down => Down,
        KeyCode::Char('k') | KeyCode::Up => Up,
        KeyCode::PageDown => PageDown,
        KeyCode::PageUp => PageUp,
        KeyCode::Char('g') | KeyCode::Home => Top,
        KeyCode::Char('G') | KeyCode::End => Bottom,
        KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right => Open,
        KeyCode::Char('h') | KeyCode::Backspace | KeyCode::Left => Parent,
        KeyCode::Char('~') => Home,
        KeyCode::Tab | KeyCode::BackTab => ToggleFocus,
        KeyCode::Char(' ') => PlayPause,
        KeyCode::Char('n') => Next,
        KeyCode::Char('p') => Prev,
        KeyCode::Char('+') | KeyCode::Char('=') => VolumeUp,
        KeyCode::Char('-') => VolumeDown,
        KeyCode::Char('>') | KeyCode::Char('.') => SeekForward,
        KeyCode::Char('<') | KeyCode::Char(',') => SeekBackward,
        KeyCode::Char('s') => ToggleShuffle,
        KeyCode::Char('r') => CycleRepeat,
        KeyCode::Char('/') => StartFilter,
        KeyCode::Esc => ClearFilter,
        KeyCode::Char('d') | KeyCode::Delete => Remove,
        KeyCode::Char('J') => MoveDown,
        KeyCode::Char('K') => MoveUp,
        KeyCode::Char('m') => ToggleMode,
        KeyCode::Char('L') => Login,
        KeyCode::Char('q') => Quit,
        _ => return None,
    })
}

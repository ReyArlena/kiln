//! Render every screen into an in-memory terminal. These catch panics in
//! layout code (for example on very small terminals) without a real TTY.

use kiln::app::AppState;
use kiln::tui::App;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn sandbox() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("temp dir");
    let state = AppState::sandboxed(dir.path()).expect("sandboxed state");
    (dir, state)
}

#[test]
fn picker_renders_at_common_and_tiny_sizes() {
    let (_dir, state) = sandbox();
    let mut app = App::new(state);
    for (w, h) in [(140, 40), (80, 24), (20, 5), (1, 1)] {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| app.draw_for_test(f)).unwrap();
    }
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|f| app.draw_for_test(f)).unwrap();
    let text = buffer_text(terminal.backend());
    assert!(text.contains("CPU Stress & Verify"));
    assert!(text.contains("Details"));
}

#[test]
fn setup_results_history_and_settings_render() {
    let (_dir, state) = sandbox();
    let mut app = App::new(state);
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();

    app.open_setup_for_test(&["cpu-stress".into(), "btrfs-harness".into()]);
    terminal.draw(|f| app.draw_for_test(f)).unwrap();
    let text = buffer_text(terminal.backend());
    assert!(text.contains("Temperature cap"));
    assert!(
        text.contains("(required)"),
        "device parameter should show as required"
    );

    for screen in ["history", "settings", "results", "dashboard"] {
        app.show_screen_for_test(screen);
        terminal.draw(|f| app.draw_for_test(f)).unwrap();
    }
}

fn buffer_text(backend: &TestBackend) -> String {
    let buffer = backend.buffer();
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

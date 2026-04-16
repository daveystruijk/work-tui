use ratatui::{backend::TestBackend, Terminal};

pub fn render_to_string<F>(width: u16, height: u16, render: F) -> String
where
    F: FnOnce(&mut ratatui::Frame),
{
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");

    terminal.draw(|frame| render(frame)).expect("draw frame");

    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    buffer
        .content
        .chunks(area.width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

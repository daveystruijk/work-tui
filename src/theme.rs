use ratatui::style::Color;

pub enum Theme {}

#[allow(non_upper_case_globals)]
impl Theme {
    pub const Text: Color = Color::White;
    pub const Muted: Color = Color::DarkGray;
    pub const Accent: Color = Color::Blue;
    pub const AccentSoft: Color = Color::Cyan;
    pub const Surface: Color = Color::Reset;
    pub const SurfaceAlt: Color = Color::DarkGray;
    pub const Panel: Color = Color::Reset;
    pub const SidebarBg: Color = Color::Rgb(42, 42, 55);
    pub const Success: Color = Color::Green;
    pub const Warning: Color = Color::Yellow;
    pub const Error: Color = Color::Red;
    pub const Info: Color = Color::LightBlue;
}

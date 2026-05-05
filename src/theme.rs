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
    pub const Selection: Color = Color::Rgb(50, 50, 65);
    pub const Success: Color = Color::Green;
    pub const Warning: Color = Color::Yellow;
    pub const Error: Color = Color::Red;
    pub const Info: Color = Color::LightBlue;

    /// Quartic rolloff color between White and DarkGray based on elapsed seconds.
    /// Stays bright for recent activity, drops off quickly approaching the 1-day cutoff.
    pub fn recency_color(elapsed_secs: u64) -> Color {
        const CUTOFF: f64 = 86_400.0;
        const HIGH: f64 = 255.0;
        const LOW: f64 = 128.0;

        let t = (elapsed_secs as f64 / CUTOFF).min(1.0);
        let brightness = (1.0 - t).powi(4); // quartic: holds bright, then rapid falloff
        let value = (LOW + brightness * (HIGH - LOW)).round() as u8;
        Color::Rgb(value, value, value)
    }
}

//! Colors. Mint brand accent, tuned separately for dark and light terminals, with 256-color and
//! NO_COLOR fallbacks.
use ratatui::style::{Color, Modifier, Style};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Brand mint: logo, selection, success.
    pub accent: Color,
    /// End of the logo gradient.
    pub accent_deep: Color,
    pub dim: Color,
    pub faint: Color,
    pub warn: Color,
    pub danger: Color,
    pub info: Color,
    pub sel_bg: Color,
    truecolor: bool,
    plain: bool,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Theme {
    /// Picks the palette from the environment. Call before entering raw mode / the alternate screen.
    pub fn detect() -> Self {
        let plain = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let truecolor =
            std::env::var("COLORTERM").is_ok_and(|v| matches!(v.as_str(), "truecolor" | "24bit"));
        let mode = match std::env::var("CMW_THEME").as_deref() {
            Ok("light") => Mode::Light,
            Ok("dark") => Mode::Dark,
            _ if plain => Mode::Dark,
            _ => {
                let mut opts = terminal_colorsaurus::QueryOptions::default();
                opts.timeout = Duration::from_millis(150);
                match terminal_colorsaurus::theme_mode(opts) {
                    Ok(terminal_colorsaurus::ThemeMode::Light) => Mode::Light,
                    _ => Mode::Dark,
                }
            }
        };
        Self::new(mode, truecolor, plain)
    }

    pub fn new(mode: Mode, truecolor: bool, plain: bool) -> Self {
        use Color::Indexed as I;
        let pick = |tc: u32, idx: u8| if truecolor { rgb(tc) } else { I(idx) };
        let t = match mode {
            Mode::Dark => Theme {
                accent: pick(0x4EF2A0, 85),
                accent_deep: pick(0x1FB8A6, 37),
                dim: pick(0x9AA5A0, 247),
                faint: pick(0x5C6661, 241),
                warn: pick(0xF2C14E, 221),
                danger: pick(0xFF8F7A, 210),
                info: pick(0x7AB8FF, 111),
                sel_bg: pick(0x1B2B24, 236),
                truecolor,
                plain,
            },
            Mode::Light => Theme {
                accent: pick(0x0C9A63, 29),
                accent_deep: pick(0x0B6F78, 24),
                dim: pick(0x5E6A64, 243),
                faint: pick(0xA3ACA8, 249),
                warn: pick(0xA86B00, 130),
                danger: pick(0xC93B3B, 160),
                info: pick(0x1F5FD1, 26),
                sel_bg: pick(0xDDF5E9, 194),
                truecolor,
                plain,
            },
        };
        if plain {
            Theme {
                accent: Color::Reset,
                accent_deep: Color::Reset,
                dim: Color::Reset,
                faint: Color::Reset,
                warn: Color::Reset,
                danger: Color::Reset,
                info: Color::Reset,
                sel_bg: Color::Reset,
                ..t
            }
        } else {
            t
        }
    }

    pub fn fg(&self, c: Color) -> Style {
        Style::new().fg(c)
    }

    pub fn accent_bold(&self) -> Style {
        Style::new().fg(self.accent).add_modifier(Modifier::BOLD)
    }

    /// Highlighted row. Without colors, reverse video keeps it visible.
    pub fn selected(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new().bg(self.sel_bg)
        }
    }

    /// Logo row color: a vertical mint → teal gradient on truecolor terminals.
    pub fn gradient(&self, row: usize, rows: usize) -> Color {
        match (self.truecolor && !self.plain, self.accent, self.accent_deep) {
            (true, Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
                let t = row as f32 / (rows.max(2) - 1) as f32;
                let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
                Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
            }
            _ if row + 2 >= rows => self.accent_deep,
            _ => self.accent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_runs_from_accent_to_deep() {
        let t = Theme::new(Mode::Dark, true, false);
        assert_eq!(t.gradient(0, 6), t.accent);
        assert_eq!(t.gradient(5, 6), t.accent_deep);
    }

    #[test]
    fn no_color_is_plain() {
        let t = Theme::new(Mode::Light, true, true);
        assert_eq!(t.accent, Color::Reset);
        assert!(t.selected().add_modifier.contains(Modifier::REVERSED));
    }
}

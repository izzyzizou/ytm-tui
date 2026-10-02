//! Design tokens (see docs/design-system.md) → ratatui colors, resolved once per color tier.

use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    TrueColor,
    Ansi256,
    Ansi16,
    Mono,
}

impl Tier {
    pub fn detect() -> Self {
        let env = |k| std::env::var(k).unwrap_or_default();
        if std::env::var_os("NO_COLOR").is_some() {
            Tier::Mono
        } else if matches!(env("COLORTERM").as_str(), "truecolor" | "24bit") {
            Tier::TrueColor
        } else if env("TERM").contains("256color") {
            Tier::Ansi256
        } else {
            Tier::Ansi16
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub tier: Tier,
    pub bg_raised: Color,
    pub bg_select: Color,
    pub border: Color,
    pub ink: Color,
    pub ink_muted: Color,
    pub ink_faint: Color,
    /// Now playing, progress, liked.
    pub ember: Color,
    /// Focus.
    pub lagoon: Color,
    /// Buffering, rate limit, cached.
    pub amber: Color,
    /// Online / healthy.
    pub moss: Color,
    pub error: Color,
}

type Tok = (u32, u8, Color);

impl Theme {
    pub fn from_name(name: &str) -> Self {
        let tier = Tier::detect();
        if name.eq_ignore_ascii_case("light") {
            Self::light(tier)
        } else {
            Self::dark(tier)
        }
    }

    fn build(tier: Tier, t: [Tok; 11]) -> Self {
        let c = |(rgb, idx, ansi16): Tok| match tier {
            Tier::TrueColor => Color::from_u32(rgb),
            Tier::Ansi256 => Color::Indexed(idx),
            Tier::Ansi16 => ansi16,
            Tier::Mono => Color::Reset,
        };
        Self {
            tier,
            bg_raised: c(t[0]),
            bg_select: c(t[1]),
            border: c(t[2]),
            ink: c(t[3]),
            ink_muted: c(t[4]),
            ink_faint: c(t[5]),
            ember: c(t[6]),
            lagoon: c(t[7]),
            amber: c(t[8]),
            moss: c(t[9]),
            error: c(t[10]),
        }
    }

    pub fn dark(tier: Tier) -> Self {
        Self::build(
            tier,
            [
                (0x1c1f26, 234, Color::Reset),
                (0x2a2f3a, 236, Color::Reset),
                (0x3d4452, 238, Color::DarkGray),
                (0xe6e1d6, 253, Color::Reset),
                (0xa39d90, 247, Color::Gray),
                (0x6b675f, 241, Color::DarkGray),
                (0xf4845f, 209, Color::LightRed),
                (0x5fc4b8, 79, Color::Cyan),
                (0xe8b75c, 179, Color::Yellow),
                (0x93c47d, 114, Color::Green),
                (0xff6b78, 204, Color::Red),
            ],
        )
    }

    pub fn light(tier: Tier) -> Self {
        Self::build(
            tier,
            [
                (0xece7dc, 254, Color::Reset),
                (0xe4ddcf, 253, Color::Reset),
                (0xa8a090, 247, Color::DarkGray),
                (0x24211c, 234, Color::Reset),
                (0x5e584e, 240, Color::DarkGray),
                (0x857d70, 244, Color::Gray),
                (0xa33c17, 130, Color::Red),
                (0x12645e, 23, Color::Cyan),
                (0x77520a, 94, Color::Yellow),
                (0x375f20, 22, Color::Green),
                (0xb3261e, 124, Color::Red),
            ],
        )
    }

    /// In 16-color and monochrome tiers the cursor row uses reverse video instead of a fill.
    pub fn select_uses_reverse(&self) -> bool {
        matches!(self.tier, Tier::Ansi16 | Tier::Mono)
    }
}

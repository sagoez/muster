use ratatui::style::{Color, Style};
use terminal_colorsaurus::{QueryOptions, background_color};

use super::widget::theme;

/// An 8-bit RGB triple.
type Rgb = (u8, u8, u8);

/// Fraction the host background is mixed toward the contrast target to form
/// the selection background (herdr's value).
const SELECTION_MIX: f32 = 0.28;
/// Relative-luminance threshold separating dark from light colors.
const DARK_LUMINANCE_THRESHOLD: f32 = 0.5;
/// The mix target over a dark background.
const WHITE: Rgb = (u8::MAX, u8::MAX, u8::MAX);
/// The mix target over a light background.
const BLACK: Rgb = (u8::MIN, u8::MIN, u8::MIN);

/// Derives the pane selection style from the host terminal's reported
/// background color, so highlighted text looks like the terminal's own
/// selection (herdr's approach). Falls back to reverse video when the
/// terminal does not answer.
///
/// Must run before the alternate screen and the input reader thread take over
/// the terminal, because the color query reads the reply from stdin.
pub fn detect_selection_style() -> Style {
    match background_color(QueryOptions::default()) {
        Ok(background) => selection_style_for(background.scale_to_8bit()),
        Err(_) => theme::selection_style(),
    }
}

/// The selection style for a given host background: the background nudged
/// toward white (dark themes) or black (light themes), under a black or white
/// foreground chosen for contrast. Ported from herdr.
fn selection_style_for(background: Rgb) -> Style {
    let target = if relative_luminance(background) < DARK_LUMINANCE_THRESHOLD {
        WHITE
    } else {
        BLACK
    };
    let selected = mix_rgb(background, target, SELECTION_MIX);
    let foreground = if relative_luminance(selected) < DARK_LUMINANCE_THRESHOLD {
        Color::White
    } else {
        Color::Black
    };
    Style::default()
        .fg(foreground)
        .bg(Color::Rgb(selected.0, selected.1, selected.2))
}

/// Mixes `base` toward `target` by `amount` per channel.
fn mix_rgb(base: Rgb, target: Rgb, amount: f32) -> Rgb {
    fn channel(base: u8, target: u8, amount: f32) -> u8 {
        (f32::from(base) + (f32::from(target) - f32::from(base)) * amount).round() as u8
    }
    (
        channel(base.0, target.0, amount),
        channel(base.1, target.1, amount),
        channel(base.2, target.2, amount),
    )
}

/// WCAG relative luminance of an 8-bit color.
fn relative_luminance(color: Rgb) -> f32 {
    fn channel(value: u8) -> f32 {
        let value = f32::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.0) + 0.7152 * channel(color.1) + 0.0722 * channel(color.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dark background lightens toward white and takes a white foreground.
    #[test]
    fn dark_background_yields_lightened_selection() {
        let style = selection_style_for((0, 0, 0));
        let gray = mix_rgb(BLACK, WHITE, SELECTION_MIX).0;
        assert_eq!(style.bg, Some(Color::Rgb(gray, gray, gray)));
        assert_eq!(style.fg, Some(Color::White));
    }

    /// A light background darkens toward black; the resulting mid-gray sits
    /// just under the WCAG threshold, so the foreground stays white (herdr
    /// behaves the same way).
    #[test]
    fn light_background_yields_darkened_selection() {
        let style = selection_style_for(WHITE);
        let gray = mix_rgb(WHITE, BLACK, SELECTION_MIX).0;
        assert_eq!(style.bg, Some(Color::Rgb(gray, gray, gray)));
        assert_eq!(style.fg, Some(Color::White));
    }

    /// The user's steel-navy terminal produces a lighter steel selection.
    #[test]
    fn dark_navy_background_mixes_toward_white() {
        let style = selection_style_for((26, 29, 41));
        assert_eq!(
            style.bg,
            Some(Color::Rgb(
                mix_rgb((26, 29, 41), WHITE, SELECTION_MIX).0,
                mix_rgb((26, 29, 41), WHITE, SELECTION_MIX).1,
                mix_rgb((26, 29, 41), WHITE, SELECTION_MIX).2
            ))
        );
        assert_eq!(style.fg, Some(Color::White));
    }
}

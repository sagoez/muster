//! The color and attributes a log line's characters are drawn in, kept as the
//! state SGR sequences leave rather than the sequences themselves, so a pen
//! is a small `Copy` value, every cell carries its own without sharing or
//! allocating, and the sequence written back out never grows past the few
//! attributes and three colors a terminal tracks.

use std::{fmt, fmt::Write, ops::RangeInclusive};

use alacritty_terminal::vte::Params;

use super::{
    param_separator::Separator,
    pen_color::{PenColor, RGB_MODE},
};

/// The SGR code that resets every attribute.
const RESET: u16 = 0;
/// The SGR code that underlines, its style as an optional sub-parameter.
const UNDERLINE: u16 = 4;
/// The SGR code that underlines twice.
const DOUBLE_UNDERLINE: u16 = 21;
/// The SGR code that ends any underline.
const NO_UNDERLINE: u16 = 24;
/// The underline style of a plain underline.
const SINGLE_UNDERLINE_STYLE: u8 = 1;
/// The underline style of a double underline.
const DOUBLE_UNDERLINE_STYLE: u8 = 2;
/// The highest underline style a terminal knows (dashed).
const MAX_UNDERLINE_STYLE: u16 = 5;
/// Separates an SGR code from its sub-parameters.
const SUBPARAM_SEPARATOR: char = ':';
/// The attributes SGR switches on, by code, in the order they are written
/// back; bit `n` of a pen's attributes stands for the code at index `n`.
const ATTRIBUTES: [u16; 9] = [1, 2, 3, 5, 6, 7, 8, 9, 53];
/// The attribute written back right after the underline, so codes come out
/// in ascending order.
const AFTER_UNDERLINE: u16 = 5;
/// The codes that switch attributes off, each with the codes it switches off.
const ATTRIBUTE_RESETS: [(u16, &[u16]); 7] = [
    (22, &[1, 2]),
    (23, &[3]),
    (25, &[5, 6]),
    (27, &[7]),
    (28, &[8]),
    (29, &[9]),
    (55, &[53]),
];
/// The SGR code that sets an extended foreground color.
const FOREGROUND: u16 = 38;
/// The SGR code that returns to the default foreground.
const DEFAULT_FOREGROUND: u16 = 39;
/// The SGR code that sets an extended background color.
const BACKGROUND: u16 = 48;
/// The SGR code that returns to the default background.
const DEFAULT_BACKGROUND: u16 = 49;
/// The SGR code that sets an extended underline color.
const UNDERLINE_COLOR: u16 = 58;
/// The SGR code that returns to the default underline color.
const DEFAULT_UNDERLINE_COLOR: u16 = 59;
/// The SGR codes of the named foreground colors, normal and bright.
const NAMED_FOREGROUNDS: [RangeInclusive<u16>; 2] = [30..=37, 90..=97];
/// The SGR codes of the named background colors, normal and bright.
const NAMED_BACKGROUNDS: [RangeInclusive<u16>; 2] = [40..=47, 100..=107];
/// Arguments an RGB color takes: red, green and blue.
const RGB_ARGS: usize = 3;

/// The attributes and colors in effect, as SGR left them; the default pen
/// draws plain text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pen {
    /// One bit per entry of [`ATTRIBUTES`] that is on.
    attributes: u16,
    /// The underline style, zero for none.
    underline: u8,
    /// The foreground color, none for the default.
    foreground: Option<PenColor>,
    /// The background color, none for the default.
    background: Option<PenColor>,
    /// The underline color, none for the default.
    underline_color: Option<PenColor>,
}

impl Pen {
    /// Whether this is the default pen, with no color or attribute set.
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// The SGR parameters that draw in this pen from a reset, without the CSI
    /// or the final `m`; empty for the default pen.
    pub fn params(&self) -> String {
        let mut text = String::new();
        let _ = self.write_params(&mut text);
        text
    }

    /// Applies the SGR sequence carrying `params`. A code a log reader's
    /// terminal would not show, or a malformed one, changes nothing.
    pub fn apply(&mut self, params: &Params) {
        if params.is_empty() {
            *self = Self::default();
            return;
        }
        let mut groups = params.iter();
        while let Some(group) = groups.next() {
            let [code, subparams @ ..] = group else {
                continue;
            };
            match *code {
                RESET => *self = Self::default(),
                UNDERLINE => {
                    self.underline = match subparams.first() {
                        None => SINGLE_UNDERLINE_STYLE,
                        Some(style) => u8::try_from(*style.min(&MAX_UNDERLINE_STYLE))
                            .unwrap_or(SINGLE_UNDERLINE_STYLE),
                    };
                },
                DOUBLE_UNDERLINE => self.underline = DOUBLE_UNDERLINE_STYLE,
                NO_UNDERLINE => self.underline = 0,
                FOREGROUND | BACKGROUND | UNDERLINE_COLOR => {
                    let color = if subparams.is_empty() {
                        extended_from(&mut groups)
                    } else {
                        extended_inline(subparams)
                    };
                    if let Some(color) = color {
                        *self.slot(*code) = Some(color);
                    }
                },
                DEFAULT_FOREGROUND => self.foreground = None,
                DEFAULT_BACKGROUND => self.background = None,
                DEFAULT_UNDERLINE_COLOR => self.underline_color = None,
                code => self.apply_simple(code),
            }
        }
    }

    /// Applies one SGR code that takes no arguments: a named color, or an
    /// attribute switched on or off.
    fn apply_simple(&mut self, code: u16) {
        let named = u8::try_from(code).ok().map(PenColor::Named);
        if NAMED_FOREGROUNDS.iter().any(|range| range.contains(&code)) {
            self.foreground = named;
        } else if NAMED_BACKGROUNDS.iter().any(|range| range.contains(&code)) {
            self.background = named;
        } else if let Some(bit) = ATTRIBUTES.iter().position(|on| *on == code) {
            self.attributes |= 1 << bit;
        } else if let Some((_, off)) = ATTRIBUTE_RESETS.iter().find(|(reset, _)| *reset == code) {
            for (bit, attribute) in ATTRIBUTES.iter().enumerate() {
                if off.contains(attribute) {
                    self.attributes &= !(1 << bit);
                }
            }
        }
    }

    /// The color an extended color `code` (38, 48 or 58) sets.
    fn slot(&mut self, code: u16) -> &mut Option<PenColor> {
        match code {
            FOREGROUND => &mut self.foreground,
            BACKGROUND => &mut self.background,
            _ => &mut self.underline_color,
        }
    }

    /// Writes the SGR parameters of this pen into `out`: the attributes in
    /// ascending order, then the foreground, background and underline color.
    ///
    /// # Errors
    /// Returns a [`fmt::Error`] when `out` cannot be written.
    fn write_params(&self, out: &mut impl Write) -> fmt::Result {
        let mut separator = Separator::default();
        for (bit, code) in ATTRIBUTES.iter().enumerate() {
            if *code == AFTER_UNDERLINE && self.underline != 0 {
                separator.write(out)?;
                match self.underline {
                    SINGLE_UNDERLINE_STYLE => write!(out, "{UNDERLINE}")?,
                    style => write!(out, "{UNDERLINE}{SUBPARAM_SEPARATOR}{style}")?,
                }
            }
            if self.attributes & (1 << bit) != 0 {
                separator.write(out)?;
                write!(out, "{code}")?;
            }
        }
        for (extended, color) in [
            (FOREGROUND, self.foreground),
            (BACKGROUND, self.background),
            (UNDERLINE_COLOR, self.underline_color),
        ] {
            if let Some(color) = color {
                separator.write(out)?;
                color.write_params(extended, out)?;
            }
        }
        Ok(())
    }
}

/// The extended color whose mode and arguments follow as parameters of their
/// own (`38;5;n`, `38;2;r;g;b`), taken from `groups`.
fn extended_from<'a>(groups: &mut impl Iterator<Item = &'a [u16]>) -> Option<PenColor> {
    let mode = *groups.next()?.first()?;
    let wanted = if mode == RGB_MODE { RGB_ARGS } else { 1 };
    let mut arguments = [0; RGB_ARGS];
    for argument in arguments.iter_mut().take(wanted) {
        *argument = *groups.next()?.first()?;
    }
    PenColor::extended(mode, &arguments[..wanted])
}

/// The extended color given as sub-parameters (`38:5:n`, `38:2:r:g:b`, or
/// `38:2:space:r:g:b` with a color space first).
fn extended_inline(subparams: &[u16]) -> Option<PenColor> {
    let (mode, arguments) = subparams.split_first()?;
    let arguments = if *mode == RGB_MODE && arguments.len() > RGB_ARGS {
        &arguments[arguments.len() - RGB_ARGS..]
    } else {
        arguments
    };
    PenColor::extended(*mode, arguments)
}

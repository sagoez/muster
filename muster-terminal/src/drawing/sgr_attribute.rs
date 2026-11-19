//! The attributes SGR switches on and off, each with the code that sets it
//! and the code that ends it.

use strum::{EnumIter, EnumProperty, IntoEnumIterator};

/// The property holding the SGR code that switches an attribute on.
const ON: &str = "on";
/// The property holding the SGR code that switches an attribute off.
const OFF: &str = "off";

/// An attribute a pen tracks, declared in ascending order of the code that
/// sets it, so a pen writes them back in that order. Its discriminant is its
/// bit in a pen's attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, EnumProperty)]
#[repr(u16)]
pub(super) enum SgrAttribute {
    /// Bold, or increased intensity.
    #[strum(props(on = 1, off = 22))]
    Bold,
    /// Faint, or decreased intensity.
    #[strum(props(on = 2, off = 22))]
    Faint,
    /// Italic.
    #[strum(props(on = 3, off = 23))]
    Italic,
    /// Slow blink.
    #[strum(props(on = 5, off = 25))]
    SlowBlink,
    /// Rapid blink.
    #[strum(props(on = 6, off = 25))]
    RapidBlink,
    /// Foreground and background swapped.
    #[strum(props(on = 7, off = 27))]
    Inverse,
    /// Hidden.
    #[strum(props(on = 8, off = 28))]
    Hidden,
    /// Crossed out.
    #[strum(props(on = 9, off = 29))]
    Strike,
    /// Overlined.
    #[strum(props(on = 53, off = 55))]
    Overline,
}

impl SgrAttribute {
    /// The SGR code that switches this attribute on.
    pub(super) fn on(self) -> u16 {
        self.code(ON)
    }

    /// This attribute's bit in a pen's attributes.
    pub(super) fn bit(self) -> u16 {
        1 << self as u16
    }

    /// The attribute SGR `code` switches on, if it switches one on.
    pub(super) fn switched_on_by(code: u16) -> Option<Self> {
        Self::iter().find(|attribute| attribute.on() == code)
    }

    /// The bits of the attributes SGR `code` switches off; zero when it
    /// switches none off.
    pub(super) fn bits_switched_off_by(code: u16) -> u16 {
        Self::iter()
            .filter(|attribute| attribute.code(OFF) == code)
            .fold(0, |bits, attribute| bits | attribute.bit())
    }

    /// The code held in `property`.
    fn code(self, property: &str) -> u16 {
        self.get_int(property)
            .and_then(|code| u16::try_from(code).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The attributes are declared in ascending order of their codes, each
    /// code sets one attribute, and each reset code clears the attributes it
    /// names.
    #[test]
    fn codes_ascend_and_each_reset_clears_its_attributes() {
        let on: Vec<u16> = SgrAttribute::iter().map(SgrAttribute::on).collect();
        assert_eq!(on, [1, 2, 3, 5, 6, 7, 8, 9, 53]);
        let off: Vec<u16> = SgrAttribute::iter()
            .map(|attribute| attribute.code(OFF))
            .collect();
        assert_eq!(off, [22, 22, 23, 25, 25, 27, 28, 29, 55]);
        assert_eq!(
            SgrAttribute::switched_on_by(53),
            Some(SgrAttribute::Overline)
        );
        assert_eq!(SgrAttribute::switched_on_by(4), None);
        assert_eq!(
            SgrAttribute::bits_switched_off_by(22),
            SgrAttribute::Bold.bit() | SgrAttribute::Faint.bit()
        );
        assert_eq!(
            SgrAttribute::bits_switched_off_by(55),
            SgrAttribute::Overline.bit()
        );
        assert_eq!(SgrAttribute::bits_switched_off_by(24), 0);
    }
}

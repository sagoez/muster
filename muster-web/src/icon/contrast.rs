//! Every mark must stand out on the face disc it is drawn on.

use muster_domain::process::ProcessIcon;
use strum::IntoEnumIterator;

use super::AgentMark;

/// The face disc's color.
const FACE: &str = "#2a2b2f";
/// The least WCAG contrast ratio a mark has against the face.
const MIN_CONTRAST: f64 = 3.0;
/// Digits of a `#rrggbb` color's channel, in hex.
const HEX_RADIX: u32 = 16;
/// The largest value of an 8-bit channel.
const CHANNEL_MAX: f64 = 255.0;
/// Where the sRGB curve turns from linear to a power.
const SRGB_KNEE: f64 = 0.039_28;
/// The slope of the sRGB curve's linear part.
const SRGB_SLOPE: f64 = 12.92;
/// The offset and scale of the sRGB curve's power part.
const SRGB_OFFSET: f64 = 0.055;
/// The exponent of the sRGB curve's power part.
const SRGB_EXPONENT: f64 = 2.4;
/// How much each channel counts toward relative luminance.
const LUMINANCE_WEIGHTS: [f64; 3] = [0.2126, 0.7152, 0.0722];
/// What WCAG adds to both luminances before dividing them.
const FLARE: f64 = 0.05;

/// The relative luminance of a `#rrggbb` color.
fn luminance(color: &str) -> f64 {
    let channel = |index: usize| {
        let start = 1 + index * 2;
        let byte = u32::from_str_radix(&color[start..start + 2], HEX_RADIX).unwrap();
        let value = f64::from(byte) / CHANNEL_MAX;
        if value <= SRGB_KNEE {
            value / SRGB_SLOPE
        } else {
            ((value + SRGB_OFFSET) / (1.0 + SRGB_OFFSET)).powf(SRGB_EXPONENT)
        }
    };
    LUMINANCE_WEIGHTS
        .iter()
        .enumerate()
        .map(|(index, weight)| weight * channel(index))
        .sum()
}

/// The WCAG contrast ratio of `color` against the face disc.
fn contrast_on_face(color: &str) -> f64 {
    let (mark, face) = (luminance(color), luminance(FACE));
    (mark.max(face) + FLARE) / (mark.min(face) + FLARE)
}

/// Every process icon and agent mark is drawn at a contrast of at least 3.0.
#[test]
fn every_mark_stands_out_on_the_face_disc() {
    for icon in ProcessIcon::iter() {
        let ratio = contrast_on_face(icon.color());
        assert!(ratio >= MIN_CONTRAST, "{icon} {} is {ratio}", icon.color());
    }
    for mark in AgentMark::iter() {
        let ratio = contrast_on_face(mark.color());
        assert!(ratio >= MIN_CONTRAST, "{mark} {} is {ratio}", mark.color());
    }
    assert!(contrast_on_face("#2a2b2f") < MIN_CONTRAST);
}

//! The colour and attributes a log line's characters are drawn in, kept as
//! the SGR parameters that set them since the last reset.

use std::sync::Arc;

use alacritty_terminal::vte::Params;

/// The SGR parameter that resets every attribute.
const RESET: u16 = 0;
/// SGR parameters that pick an extended colour (foreground, background,
/// underline); their arguments follow as parameters of their own.
const EXTENDED_COLOURS: [u16; 3] = [38, 48, 58];
/// Extended-colour mode that picks one of 256 colours by index.
const PALETTE_MODE: u16 = 5;
/// Arguments after [`PALETTE_MODE`]: the index.
const PALETTE_ARGS: usize = 1;
/// Extended-colour mode that gives red, green and blue.
const RGB_MODE: u16 = 2;
/// Arguments after [`RGB_MODE`]: red, green and blue.
const RGB_ARGS: usize = 3;
/// An extended colour's own parameter and its mode, before the arguments.
const EXTENDED_HEAD: usize = 2;
/// Longest the kept parameters grow; past it only those of the newest SGR
/// are kept, so a stream that never resets cannot grow a pen without bound.
pub const MAX_PEN_BYTES: usize = 256;
/// Separates SGR parameters.
const PARAM_SEPARATOR: &str = ";";
/// Separates the sub-parameters of one SGR parameter.
const SUBPARAM_SEPARATOR: &str = ":";

/// The SGR parameters in effect, rebuilt from parsed numbers so nothing but
/// digits and separators is ever written back out; empty for the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pen(Arc<str>);

impl Pen {
    /// Whether this is the default pen, with no colour or attribute set.
    pub fn is_plain(&self) -> bool {
        self.0.is_empty()
    }

    /// The SGR parameters that draw in this pen, without the CSI or the
    /// final `m`.
    pub fn params(&self) -> &str {
        &self.0
    }

    /// Applies the SGR sequence carrying `params`: a reset clears what came
    /// before it, anything else is added to what is in effect.
    pub fn apply(&mut self, params: &Params) {
        let groups: Vec<&[u16]> = params.iter().collect();
        let mut fresh: Vec<String> = Vec::new();
        let mut reset = groups.is_empty();
        let mut index = 0;
        while index < groups.len() {
            let taken = match groups[index] {
                [RESET] => {
                    fresh.clear();
                    reset = true;
                    index += 1;
                    continue;
                },
                [code] if EXTENDED_COLOURS.contains(code) => {
                    let arguments = match groups.get(index + 1) {
                        Some([PALETTE_MODE]) => PALETTE_ARGS,
                        Some([RGB_MODE]) => RGB_ARGS,
                        _ => 0,
                    };
                    (EXTENDED_HEAD + arguments).min(groups.len() - index)
                },
                _ => 1,
            };
            fresh.push(
                groups[index..index + taken]
                    .iter()
                    .map(|group| joined(group, SUBPARAM_SEPARATOR))
                    .collect::<Vec<_>>()
                    .join(PARAM_SEPARATOR),
            );
            index += taken;
        }
        let fresh = fresh.join(PARAM_SEPARATOR);
        let combined = if reset || self.0.is_empty() {
            fresh.clone()
        } else if fresh.is_empty() {
            return;
        } else {
            format!("{}{PARAM_SEPARATOR}{fresh}", self.0)
        };
        self.0 = if combined.len() > MAX_PEN_BYTES {
            fresh.into()
        } else {
            combined.into()
        };
    }
}

/// `values` written out joined by `separator`.
fn joined(values: &[u16], separator: &str) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(separator)
}

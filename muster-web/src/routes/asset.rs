use std::str::FromStr;

use strum::{EnumProperty, EnumString};

/// Where this version's assets are served from. The version in the path
/// means a browser never runs a cached asset of another muster.
pub const ASSET_ROOT: &str = concat!("/assets/v", env!("CARGO_PKG_VERSION"));
/// The property naming an asset kind's media type.
const MEDIA_TYPE: &str = "media_type";
/// What separates a file name from its extension.
const EXTENSION_MARK: char = '.';

/// Every embedded asset: its path under [`ASSET_ROOT`] and its bytes.
const ASSETS: [(&str, &[u8]); 1] = [("js/app.js", include_bytes!("../../assets/js/app.js"))];

/// A kind of asset, by its file extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub enum AssetKind {
    #[strum(props(media_type = "text/javascript; charset=utf-8"))]
    Js,
    #[strum(props(media_type = "text/css; charset=utf-8"))]
    Css,
    #[strum(props(media_type = "image/svg+xml"))]
    Svg,
    #[strum(props(media_type = "font/woff2"))]
    Woff2,
}

impl AssetKind {
    /// The media type this kind is served as.
    pub fn media_type(self) -> &'static str {
        self.get_str(MEDIA_TYPE).unwrap_or_default()
    }
}

/// The embedded asset at `path` under [`ASSET_ROOT`], with its kind.
pub fn asset(path: &str) -> Option<(AssetKind, &'static [u8])> {
    let (_, body) = ASSETS.iter().find(|(known, _)| *known == path)?;
    let (_, extension) = path.rsplit_once(EXTENSION_MARK)?;
    Some((AssetKind::from_str(extension).ok()?, *body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every embedded asset is found with a kind, and every kind has a
    /// media type; a path not embedded is not found.
    #[test]
    fn every_asset_is_served_with_a_media_type() {
        for (path, body) in ASSETS {
            let (_, served) = asset(path).unwrap_or_else(|| panic!("{path} has no kind"));
            assert_eq!(served, body);
        }
        for extension in ["js", "css", "svg", "woff2"] {
            let kind = AssetKind::from_str(extension).unwrap();
            assert!(!kind.media_type().is_empty(), "{extension}");
        }
        assert_eq!(asset("js/missing.js"), None);
    }
}

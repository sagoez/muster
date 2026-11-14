use muster_domain::process::ProcessIcon;

/// The folder of brand marks under the asset root.
const BRAND_DIR: &str = "icons/brands";
/// The extension of a brand mark's file.
const BRAND_EXTENSION: &str = "svg";

/// The path under the asset root of `icon`'s mark, named by its slug.
#[must_use]
pub fn brand_asset(icon: ProcessIcon) -> String {
    format!("{BRAND_DIR}/{icon}.{BRAND_EXTENSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mark is named by its slug under the brand folder.
    #[test]
    fn a_mark_is_named_by_its_slug() {
        assert_eq!(
            brand_asset(ProcessIcon::Nodedotjs),
            "icons/brands/nodedotjs.svg"
        );
        assert_eq!(
            brand_asset(ProcessIcon::Terminal),
            "icons/brands/terminal.svg"
        );
    }
}

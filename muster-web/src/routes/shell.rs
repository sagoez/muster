use super::asset::ASSET_ROOT;

/// The media type the shell is served as.
pub const SHELL_MEDIA_TYPE: &str = "text/html; charset=utf-8";
/// The page's shell, naming its assets under [`ASSET_ROOT_MARK`].
const SHELL_TEMPLATE: &str = include_str!("../../assets/shell.html");
/// What the template writes where this version's asset root goes.
const ASSET_ROOT_MARK: &str = "{asset-root}";

/// The page's shell, its assets under this version's root.
pub fn shell() -> String {
    SHELL_TEMPLATE.replace(ASSET_ROOT_MARK, ASSET_ROOT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shell names the entry module under this version's root, relative
    /// so it resolves below the token's path, and no mark is left.
    #[test]
    fn the_shell_names_its_assets_under_this_version() {
        let shell = shell();

        assert!(
            shell.contains(&format!(
                "src=\"assets/v{}/js/app.js\"",
                env!("CARGO_PKG_VERSION")
            )),
            "{shell}"
        );
        assert!(!shell.contains(ASSET_ROOT_MARK));
    }
}

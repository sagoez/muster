use std::str::FromStr;

use strum::{EnumIter, EnumProperty, EnumString};

/// Where this version's assets are served from, relative to the shell, so
/// the page works under its token's path. The version in the path means a
/// browser never runs a cached asset of another muster.
pub const ASSET_ROOT: &str = concat!("assets/v", env!("CARGO_PKG_VERSION"));
/// The property naming an asset kind's media type.
const MEDIA_TYPE: &str = "media_type";
/// What separates a file name from its extension.
const EXTENSION_MARK: char = '.';

/// Embeds each file of `assets/` named by its path below it, as the pair of
/// that path and its bytes.
macro_rules! embed {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!("../../assets/", $path)) as &[u8])),*]
    };
}

/// Every embedded asset: its path under [`ASSET_ROOT`] and its bytes. The
/// shell is not one; it is a template the shell route fills.
const ASSETS: &[(&str, &[u8])] = embed![
    "css/view.css",
    "fonts/LICENSE.txt",
    "fonts/inter-latin-wght-normal.woff2",
    "icons/brands/LICENSE.txt",
    "icons/brands/astro.svg",
    "icons/brands/bun.svg",
    "icons/brands/claude.svg",
    "icons/brands/deno.svg",
    "icons/brands/django.svg",
    "icons/brands/docker.svg",
    "icons/brands/dotnet.svg",
    "icons/brands/elixir.svg",
    "icons/brands/go.svg",
    "icons/brands/mongodb.svg",
    "icons/brands/mysql.svg",
    "icons/brands/nextdotjs.svg",
    "icons/brands/nginx.svg",
    "icons/brands/nodedotjs.svg",
    "icons/brands/nuxt.svg",
    "icons/brands/php.svg",
    "icons/brands/postgresql.svg",
    "icons/brands/python.svg",
    "icons/brands/rabbitmq.svg",
    "icons/brands/redis.svg",
    "icons/brands/ruby.svg",
    "icons/brands/rubyonrails.svg",
    "icons/brands/rust.svg",
    "icons/brands/svelte.svg",
    "icons/brands/terminal.svg",
    "icons/brands/vite.svg",
    "icons/brands/vuedotjs.svg",
    "icons/lucide/LICENSE.txt",
    "icons/lucide/bot.svg",
    "icons/lucide/check.svg",
    "icons/lucide/circle-check.svg",
    "icons/lucide/circle-pause.svg",
    "icons/lucide/circle-x.svg",
    "icons/lucide/eye-off.svg",
    "icons/lucide/folder-open.svg",
    "icons/lucide/loader-circle.svg",
    "icons/lucide/pause.svg",
    "icons/lucide/refresh-cw.svg",
    "icons/lucide/square-terminal.svg",
    "icons/lucide/sticky-note.svg",
    "icons/lucide/triangle-alert.svg",
    "icons/lucide/user.svg",
    "icons/lucide/x.svg",
    "js/app.js",
    "js/card.js",
    "js/dock.js",
    "js/floor.js",
    "js/floors.js",
    "js/format.js",
    "js/marks.js",
    "js/notes.js",
    "js/pin.js",
    "js/stream.js",
    "js/theme.js",
    "js/walk.js",
    "js/vendor/htm/LICENSE.txt",
    "js/vendor/htm/htm.mjs",
    "js/vendor/preact/LICENSE.txt",
    "js/vendor/preact/hooks.mjs",
    "js/vendor/preact/preact.mjs",
    "js/vendor/signals-core/LICENSE.txt",
    "js/vendor/signals-core/signals-core.mjs",
    "js/vendor/signals/LICENSE.txt",
    "js/vendor/signals/signals.mjs",
];

/// A kind of asset, by its file extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, EnumString, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub enum AssetKind {
    #[strum(props(media_type = "text/javascript; charset=utf-8"))]
    Js,
    #[strum(props(media_type = "text/javascript; charset=utf-8"))]
    Mjs,
    #[strum(props(media_type = "text/css; charset=utf-8"))]
    Css,
    #[strum(props(media_type = "image/svg+xml"))]
    Svg,
    #[strum(props(media_type = "font/woff2"))]
    Woff2,
    #[strum(props(media_type = "text/plain; charset=utf-8"))]
    Txt,
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
    use std::{
        collections::BTreeSet,
        fs,
        path::{Path, PathBuf},
        str,
    };

    use muster_domain::process::ProcessIcon;
    use strum::IntoEnumIterator;

    use super::*;
    use crate::icon::{AgentMark, brand_asset};

    /// The folder the assets are embedded from.
    const ASSETS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets");
    /// The shell template, embedded by its own route.
    const SHELL_FILE: &str = "shell.html";
    /// The file every vendored source keeps beside it.
    const LICENSE_FILE: &str = "LICENSE.txt";
    /// The folders of vendored sources, which each keep a licence.
    const VENDORED_ROOTS: [&str; 3] = ["js/vendor/", "fonts/", "icons/"];
    /// The folder of vendored ES modules.
    const MODULE_ROOT: &str = "js/vendor/";
    /// What introduces the module an ES module imports, in minified form.
    const IMPORT_MARKS: [&str; 2] = ["from\"", "import\""];
    /// What a vendored module must not carry: a source map reference.
    const SOURCE_MAP_MARK: &str = "sourceMappingURL";
    /// The folder of the page's own modules.
    const PAGE_ROOT: &str = "js/";
    /// What introduces the module a page module imports, as it is written.
    const PAGE_IMPORT_MARK: &str = "from \"";
    /// The page module holding the icon and author colour tables.
    const MARKS_MODULE: &str = "js/marks.js";
    /// How a page module names a line icon: `line("<name>")`.
    const LINE_ICON_MARK: &str = "line(\"";
    /// How a page module would hand the icon helper a name it never marked.
    const UNMARKED_ICON: &str = "<${Icon} name=\"";
    /// The folder of the bundled line icons, and their extension.
    const LINE_ICON_DIR: &str = "icons/lucide/";
    const LINE_ICON_EXTENSION: &str = ".svg";

    /// The path of every file below `dir`, relative to `base` and joined with `/`.
    fn files_below(base: &Path, dir: &Path, found: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files_below(base, &path, found);
            } else {
                let relative = path.strip_prefix(base).unwrap();
                let parts: Vec<_> = relative
                    .components()
                    .map(|part| part.as_os_str().to_str().unwrap())
                    .collect();
                found.insert(parts.join("/"));
            }
        }
    }

    /// The module path an import `specifier` names from the module at `from`.
    fn imported_path(from: &str, specifier: &str) -> String {
        let mut parts: Vec<&str> = from.split('/').collect();
        parts.pop();
        for part in specifier.split('/') {
            match part {
                "." => {},
                ".." => {
                    parts.pop();
                },
                other => parts.push(other),
            }
        }
        parts.join("/")
    }

    /// Every embedded asset is found with a kind, and every kind has a
    /// media type; a path not embedded is not found.
    #[test]
    fn every_asset_is_served_with_a_media_type() {
        for (path, body) in ASSETS {
            let (_, served) = asset(path).unwrap_or_else(|| panic!("{path} has no kind"));
            assert_eq!(served, *body);
        }
        for kind in AssetKind::iter() {
            assert!(!kind.media_type().is_empty(), "{kind:?}");
        }
        assert_eq!(asset("js/missing.js"), None);
    }

    /// Every file under `assets/` is embedded and nothing else is, apart
    /// from the shell the shell route embeds.
    #[test]
    fn every_file_under_assets_is_embedded_and_nothing_else() {
        let base = PathBuf::from(ASSETS_DIR);
        let mut on_disk = BTreeSet::new();
        files_below(&base, &base, &mut on_disk);
        on_disk.remove(SHELL_FILE);
        let embedded: BTreeSet<String> =
            ASSETS.iter().map(|(path, _)| (*path).to_string()).collect();
        assert_eq!(on_disk, embedded);
        assert_eq!(embedded.len(), ASSETS.len(), "an asset is listed twice");
    }

    /// Every process icon and every agent mark has its file bundled, and the
    /// brand file of the terminal is the Lucide one.
    #[test]
    fn every_icon_and_agent_mark_has_its_file_bundled() {
        for icon in ProcessIcon::iter() {
            let path = brand_asset(icon);
            assert!(asset(&path).is_some(), "{icon} has no file at {path}");
        }
        for mark in AgentMark::iter() {
            assert!(asset(mark.asset()).is_some(), "{mark} has no file");
        }
        assert_eq!(
            asset("icons/brands/terminal.svg"),
            asset("icons/lucide/square-terminal.svg")
        );
    }

    /// A vendored module imports relative paths that are embedded, and
    /// carries no source map reference.
    #[test]
    fn vendored_modules_import_only_embedded_relative_paths() {
        let mut checked = 0;
        for (path, body) in ASSETS
            .iter()
            .filter(|(path, _)| path.starts_with(MODULE_ROOT) && path.ends_with(".mjs"))
        {
            let source = str::from_utf8(body).unwrap();
            assert!(
                !source.contains(SOURCE_MAP_MARK),
                "{path} keeps a source map"
            );
            for mark in IMPORT_MARKS {
                for piece in source.split(mark).skip(1) {
                    let specifier = piece.split('"').next().unwrap();
                    assert!(
                        specifier.starts_with("./") || specifier.starts_with("../"),
                        "{path} imports {specifier}"
                    );
                    let target = imported_path(path, specifier);
                    assert!(asset(&target).is_some(), "{path} imports {target}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
    }

    /// Each vendored source keeps its licence beside it.
    #[test]
    fn each_vendored_source_keeps_its_licence() {
        for (path, _) in ASSETS {
            if VENDORED_ROOTS.iter().any(|root| path.starts_with(root)) {
                let (folder, _) = path.rsplit_once('/').unwrap();
                let licence = format!("{folder}/{LICENSE_FILE}");
                assert!(asset(&licence).is_some(), "{path} has no {licence}");
            }
        }
    }

    /// The page's own modules, outside the vendored folder, with their text.
    fn page_modules() -> Vec<(&'static str, &'static str)> {
        ASSETS
            .iter()
            .filter(|(path, _)| path.starts_with(PAGE_ROOT) && !path.starts_with(MODULE_ROOT))
            .map(|(path, body)| (*path, str::from_utf8(body).unwrap()))
            .collect()
    }

    /// Each page module imports only relative paths that are embedded.
    #[test]
    fn page_modules_import_only_embedded_relative_paths() {
        let mut checked = 0;
        for (path, source) in page_modules() {
            for piece in source.split(PAGE_IMPORT_MARK).skip(1) {
                let specifier = piece.split('"').next().unwrap();
                assert!(specifier.starts_with("./"), "{path} imports {specifier}");
                let target = imported_path(path, specifier);
                assert!(asset(&target).is_some(), "{path} imports {target}");
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    /// The page draws every process icon and author in the colour the Rust
    /// side gives it, from the file the Rust side names.
    #[test]
    fn the_page_mirrors_every_icon_and_author_colour() {
        let (_, marks) = asset(MARKS_MODULE).unwrap();
        let marks = str::from_utf8(marks).unwrap();
        for icon in ProcessIcon::iter() {
            let entry = format!("  {icon}: \"{}\",\n", icon.color());
            assert!(marks.contains(&entry), "{MARKS_MODULE} lacks {entry:?}");
        }
        for mark in AgentMark::iter() {
            let entry = format!(
                "  {mark}: {{ asset: \"{}\", color: \"{}\", title: \"{}\" }},\n",
                mark.asset(),
                mark.color(),
                mark.title()
            );
            assert!(marks.contains(&entry), "{MARKS_MODULE} lacks {entry:?}");
        }
    }

    /// Every line icon the page names through `line("...")` is bundled, and
    /// no page hands the icon helper a name it did not mark that way.
    #[test]
    fn every_line_icon_the_page_names_is_bundled() {
        let mut named = BTreeSet::new();
        for (path, source) in page_modules() {
            assert!(
                !source.contains(UNMARKED_ICON),
                "{path} names a line icon without line()"
            );
            for piece in source.split(LINE_ICON_MARK).skip(1) {
                named.insert(piece.split('"').next().unwrap());
            }
        }
        assert!(!named.is_empty());
        for name in named {
            let path = format!("{LINE_ICON_DIR}{name}{LINE_ICON_EXTENSION}");
            assert!(asset(&path).is_some(), "{name} is not bundled at {path}");
        }
    }
}

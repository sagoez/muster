use std::path::Path;

use muster_domain::process::ProcessIcon;
use serde_json::Value;

use crate::ManifestReader;

/// The manifests that name a tool by their presence, in the order they are
/// looked for.
const MANIFESTS: &[(&str, ProcessIcon)] = &[
    ("Cargo.toml", ProcessIcon::Rust),
    ("go.mod", ProcessIcon::Go),
    ("pyproject.toml", ProcessIcon::Python),
    ("requirements.txt", ProcessIcon::Python),
    ("compose.yaml", ProcessIcon::Docker),
    ("compose.yml", ProcessIcon::Docker),
    ("docker-compose.yaml", ProcessIcon::Docker),
    ("docker-compose.yml", ProcessIcon::Docker),
];
/// The Node manifest, whose dependencies refine the icon.
const PACKAGE_JSON: &str = "package.json";
/// The most of a `package.json` that is read.
const PACKAGE_JSON_LIMIT: u64 = 256 * 1024;
/// The `package.json` sections naming dependencies.
const DEPENDENCY_SECTIONS: [&str; 2] = ["dependencies", "devDependencies"];
/// The dependency that makes a Node project a Next.js one.
const NEXT_PACKAGE: &str = "next";
/// The dependency that makes a Node project a Vite one.
const VITE_PACKAGE: &str = "vite";

/// The icon a `package.json` body names: Next.js over Vite over plain Node,
/// and plain Node for a body that is missing or not a JSON object.
fn node_icon(body: Option<&str>) -> ProcessIcon {
    let Some(Value::Object(package)) = body.and_then(|text| serde_json::from_str(text).ok()) else {
        return ProcessIcon::Nodedotjs;
    };
    let depends_on = |name: &str| {
        DEPENDENCY_SECTIONS.iter().any(|section| {
            package
                .get(*section)
                .and_then(Value::as_object)
                .is_some_and(|dependencies| dependencies.contains_key(name))
        })
    };
    if depends_on(NEXT_PACKAGE) {
        ProcessIcon::Nextdotjs
    } else if depends_on(VITE_PACKAGE) {
        ProcessIcon::Vite
    } else {
        ProcessIcon::Nodedotjs
    }
}

/// The icon of the first manifest found in `dir`, none when it holds none.
pub(super) fn manifest_icon(dir: &Path, manifests: &dyn ManifestReader) -> Option<ProcessIcon> {
    if let Some((_, icon)) = MANIFESTS
        .iter()
        .find(|(name, _)| manifests.stat(&dir.join(name)).is_some())
    {
        return Some(*icon);
    }
    let package = dir.join(PACKAGE_JSON);
    manifests
        .stat(&package)
        .is_some()
        .then(|| node_icon(manifests.read_text(&package, PACKAGE_JSON_LIMIT).as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon::fake_manifests::FakeManifests;

    /// The folder the manifests under test are in.
    const DIR: &str = "/home/me/shop";

    /// The icon of `files` in the test folder.
    fn icon_of(files: &FakeManifests) -> Option<ProcessIcon> {
        manifest_icon(Path::new(DIR), files)
    }

    /// Each manifest names its tool, and an empty folder names none.
    #[test]
    fn a_manifest_names_its_tool() {
        let cases = [
            ("Cargo.toml", ProcessIcon::Rust),
            ("go.mod", ProcessIcon::Go),
            ("pyproject.toml", ProcessIcon::Python),
            ("requirements.txt", ProcessIcon::Python),
            ("compose.yaml", ProcessIcon::Docker),
            ("compose.yml", ProcessIcon::Docker),
            ("docker-compose.yaml", ProcessIcon::Docker),
            ("docker-compose.yml", ProcessIcon::Docker),
        ];
        for (name, icon) in cases {
            let files = FakeManifests::default().with(&format!("{DIR}/{name}"), "");
            assert_eq!(icon_of(&files), Some(icon), "{name}");
        }
        assert_eq!(icon_of(&FakeManifests::default()), None);
    }

    /// A manifest in another folder names nothing here.
    #[test]
    fn only_the_folder_itself_is_looked_in() {
        let files = FakeManifests::default().with("/home/me/other/Cargo.toml", "");
        assert_eq!(icon_of(&files), None);
    }

    /// `package.json` is plain Node, refined by a dependency: Vite, or Next.js
    /// over Vite, in either section.
    #[test]
    fn package_json_is_refined_by_its_dependencies() {
        let cases = [
            ("{}", ProcessIcon::Nodedotjs),
            (r#"{"devDependencies":{"vite":"^5"}}"#, ProcessIcon::Vite),
            (r#"{"dependencies":{"vite":"^5"}}"#, ProcessIcon::Vite),
            (
                r#"{"dependencies":{"next":"14","react":"18"},"devDependencies":{"vite":"^5"}}"#,
                ProcessIcon::Nextdotjs,
            ),
            (r#"{"dependencies":["vite"]}"#, ProcessIcon::Nodedotjs),
            ("not json", ProcessIcon::Nodedotjs),
            ("[\"vite\"]", ProcessIcon::Nodedotjs),
        ];
        for (body, icon) in cases {
            let files = FakeManifests::default().with(&format!("{DIR}/package.json"), body);
            assert_eq!(icon_of(&files), Some(icon), "{body}");
        }
    }

    /// A `package.json` larger than the read limit stays plain Node, however
    /// it ends.
    #[test]
    fn an_oversized_package_json_is_plain_node() {
        let padding = " ".repeat(usize::try_from(PACKAGE_JSON_LIMIT).unwrap());
        let body = format!(r#"{{"devDependencies":{{"vite":"1"}}}}{padding}"#);
        let files = FakeManifests::default().with(&format!("{DIR}/package.json"), &body);
        assert_eq!(icon_of(&files), Some(ProcessIcon::Nodedotjs));
    }

    /// A tool's manifest wins over `package.json`.
    #[test]
    fn a_tool_manifest_comes_before_package_json() {
        let files = FakeManifests::default()
            .with(&format!("{DIR}/package.json"), "{}")
            .with(&format!("{DIR}/go.mod"), "");
        assert_eq!(icon_of(&files), Some(ProcessIcon::Go));
    }
}

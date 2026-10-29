use std::path::{Path, PathBuf};

use rusqlite::{Connection, Params, Row, params};

use super::sqlite_store::{SqliteStore, corrupt, db_error, now_epoch_secs};
use crate::domain::{
    config::{ConfigError, Registration},
    port::ProjectStore,
    project::Project,
    value::{ProjectName, ProjectRoot},
};

/// Selects a project's row id, root and name, in that order.
const SELECT_PROJECT: &str = "SELECT id, root, name FROM projects";

/// A project with the row id other tables key it by.
pub(super) type ProjectRow = (i64, Project);

/// The text a project root is stored as.
///
/// # Errors
/// Returns [`ConfigError::RootNotUtf8`] for a root that is not UTF-8, which
/// `path::project_root` never builds.
pub(super) fn root_text(root: &ProjectRoot) -> Result<&str, ConfigError> {
    root.as_ref()
        .to_str()
        .ok_or_else(|| ConfigError::RootNotUtf8 {
            path: root.as_ref().clone(),
        })
}

/// Reads a `SELECT_PROJECT` row.
fn project_from_row(row: &Row) -> Result<ProjectRow, ConfigError> {
    let id: i64 = row.get(0).map_err(db_error)?;
    let root: String = row.get(1).map_err(db_error)?;
    let name: String = row.get(2).map_err(db_error)?;
    let project = Project::builder()
        .root(
            ProjectRoot::try_new(PathBuf::from(root))
                .map_err(|error| corrupt("project root", error))?,
        )
        .name(ProjectName::try_new(name).map_err(|error| corrupt("project name", error))?)
        .build();
    Ok((id, project))
}

/// Every project row `sql` selects with `values`.
fn projects_where(
    conn: &Connection,
    sql: &str,
    values: impl Params,
) -> Result<Vec<ProjectRow>, ConfigError> {
    let mut statement = conn.prepare(sql).map_err(db_error)?;
    let mut rows = statement.query(values).map_err(db_error)?;
    let mut projects = Vec::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        projects.push(project_from_row(row)?);
    }
    Ok(projects)
}

/// The project registered at `root`, if any.
pub(super) fn find_project(
    conn: &Connection,
    root: &ProjectRoot,
) -> Result<Option<ProjectRow>, ConfigError> {
    Ok(
        projects_where(conn, &format!("{SELECT_PROJECT} WHERE root = ?1"), params![
            root_text(root)?
        ])?
        .pop(),
    )
}

/// The project registered at `root`.
///
/// # Errors
/// Returns [`ConfigError::UnknownProject`] when `root` is not registered.
pub(super) fn require_project(
    conn: &Connection,
    root: &ProjectRoot,
) -> Result<ProjectRow, ConfigError> {
    find_project(conn, root)?.ok_or_else(|| ConfigError::UnknownProject {
        root: root.as_ref().clone(),
    })
}

/// The project at `root`, registering it first under the first free name
/// [`ProjectName::candidates`] yields. Runs inside the caller's write
/// transaction, so the name stays free until it commits.
pub(super) fn register(
    conn: &Connection,
    root: &ProjectRoot,
) -> Result<(i64, Registration), ConfigError> {
    if let Some((id, project)) = find_project(conn, root)? {
        return Ok((
            id,
            Registration::builder()
                .project(project)
                .newly_registered(false)
                .build(),
        ));
    }
    let mut taken = conn
        .prepare("SELECT 1 FROM projects WHERE name = ?1")
        .map_err(db_error)?;
    let mut name = None;
    for candidate in ProjectName::candidates(root) {
        if !taken
            .exists(params![candidate.as_ref()])
            .map_err(db_error)?
        {
            name = Some(candidate);
            break;
        }
    }
    let name = name.ok_or_else(|| corrupt("project names", "every candidate is taken"))?;
    let id = conn
        .query_row(
            "INSERT INTO projects (root, name, created_at) VALUES (?1, ?2, ?3) RETURNING id",
            params![root_text(root)?, name.as_ref(), now_epoch_secs()],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    let project = Project::builder().root(root.clone()).name(name).build();
    Ok((
        id,
        Registration::builder()
            .project(project)
            .newly_registered(true)
            .build(),
    ))
}

impl ProjectStore for SqliteStore {
    fn projects(&self) -> Result<Vec<Project>, ConfigError> {
        self.read(|conn| {
            Ok(
                projects_where(conn, &format!("{SELECT_PROJECT} ORDER BY name"), [])?
                    .into_iter()
                    .map(|(_, project)| project)
                    .collect(),
            )
        })
    }

    fn project_at(&self, root: &ProjectRoot) -> Result<Option<Project>, ConfigError> {
        self.read(|conn| Ok(find_project(conn, root)?.map(|(_, project)| project)))
    }

    fn project_named(&self, name: &ProjectName) -> Result<Option<Project>, ConfigError> {
        self.read(|conn| {
            Ok(
                projects_where(conn, &format!("{SELECT_PROJECT} WHERE name = ?1"), params![
                    name.as_ref()
                ])?
                .pop()
                .map(|(_, project)| project),
            )
        })
    }

    fn project_containing(&self, dir: &Path) -> Result<Option<Project>, ConfigError> {
        self.read(|conn| {
            Ok(projects_where(conn, SELECT_PROJECT, [])?
                .into_iter()
                .map(|(_, project)| project)
                .filter(|project| project.root().contains(dir))
                .max_by_key(|project| project.root().as_ref().components().count()))
        })
    }

    fn remove_project(&self, root: &ProjectRoot) -> Result<bool, ConfigError> {
        self.write(|conn| {
            let removed = conn
                .execute("DELETE FROM projects WHERE root = ?1", params![root_text(
                    root
                )?])
                .map_err(db_error)?;
            Ok(removed > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::store::test_support::{fresh_store, spec},
        domain::{
            config::Environment,
            coordination::{Author, KvKey},
            port::{CoordinationStore, StackStore},
            value::{EnvName, EnvValue},
        },
    };

    /// A real folder under `scratch`, as a project root.
    fn folder(scratch: &TempDir, relative: &str) -> ProjectRoot {
        let dir = scratch.path().join(relative);
        fs::create_dir_all(&dir).unwrap();
        ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap()
    }

    /// Rows of `table` left in the database.
    fn rows_in(store: &SqliteStore, table: &str) -> i64 {
        store
            .read(|conn| {
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .map_err(db_error)
            })
            .unwrap()
    }

    #[test]
    fn the_first_add_registers_the_folder_under_its_name() {
        let (store, scratch) = fresh_store();
        let root = folder(&scratch, "shop");

        let first = store
            .add_process(&root, &spec("web", "npm run dev"))
            .unwrap();
        let second = store.add_process(&root, &spec("api", "cargo run")).unwrap();

        assert!(*first.newly_registered());
        assert!(!*second.newly_registered());
        assert_eq!(first.project().name().as_ref(), "shop");
        assert_eq!(
            store.project_at(&root).unwrap().as_ref(),
            Some(first.project())
        );
        assert_eq!(
            store
                .project_named(first.project().name())
                .unwrap()
                .as_ref(),
            Some(first.project())
        );
    }

    /// Two folders with one name get distinct project names.
    #[test]
    fn a_colliding_folder_name_is_suffixed() {
        let (store, scratch) = fresh_store();
        let names: Vec<String> = ["a/shop", "b/shop", "c/shop"]
            .into_iter()
            .map(|relative| {
                store
                    .add_process(&folder(&scratch, relative), &spec("web", "true"))
                    .unwrap()
                    .project()
                    .name()
                    .to_string()
            })
            .collect();

        assert_eq!(names, ["shop", "shop-2", "shop-3"]);
        let listed: Vec<String> = store
            .projects()
            .unwrap()
            .iter()
            .map(|project| project.name().to_string())
            .collect();
        assert_eq!(listed, ["shop", "shop-2", "shop-3"], "listed by name");
    }

    /// The deepest registered ancestor wins, and containment is by whole
    /// components.
    #[test]
    fn project_containing_picks_the_deepest_root_by_components() {
        let (store, scratch) = fresh_store();
        let outer = folder(&scratch, "a");
        let inner = folder(&scratch, "a/b");
        store.add_process(&outer, &spec("web", "true")).unwrap();
        store.add_process(&inner, &spec("web", "true")).unwrap();
        let inside = |relative: &str| scratch.path().canonicalize().unwrap().join(relative);

        let found = |dir: PathBuf| {
            store
                .project_containing(&dir)
                .unwrap()
                .map(|project| project.root().clone())
        };
        assert_eq!(found(inside("a/b/src")), Some(inner.clone()));
        assert_eq!(found(inside("a/b")), Some(inner));
        assert_eq!(
            found(inside("a/bc")),
            Some(outer.clone()),
            "a/bc is not inside a/b"
        );
        assert_eq!(found(inside("a")), Some(outer));
        assert_eq!(found(inside("z")), None);
    }

    /// Removing a project takes its processes, environment and coordination
    /// with it, and leaves other projects alone.
    #[test]
    fn remove_project_cascades() {
        let (store, scratch) = fresh_store();
        let root = folder(&scratch, "shop");
        let other = folder(&scratch, "other");
        let with_port = spec("web", "true").with_environment(Environment::empty().with_var(
            EnvName::try_new("PORT").unwrap(),
            EnvValue::try_new("3000").unwrap(),
        ));
        store.add_process(&root, &with_port).unwrap();
        store.add_process(&other, &with_port).unwrap();
        store
            .set_value(
                &root,
                &Author::human(),
                &KvKey::try_new("port").unwrap(),
                "3000",
            )
            .unwrap();
        store
            .set_value(
                &other,
                &Author::human(),
                &KvKey::try_new("port").unwrap(),
                "4000",
            )
            .unwrap();

        assert!(store.remove_project(&root).unwrap());
        assert!(!store.remove_project(&root).unwrap());

        assert_eq!(store.project_at(&root).unwrap(), None);
        assert_eq!(store.stack(&root).unwrap(), None);
        assert_eq!(rows_in(&store, "processes"), 1);
        assert_eq!(rows_in(&store, "process_env"), 1);
        assert_eq!(rows_in(&store, "kv"), 1);
        assert_eq!(store.values(&other).unwrap().len(), 1);
    }
}

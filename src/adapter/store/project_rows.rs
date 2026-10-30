use std::path::{Path, PathBuf};

use rusqlite::{Connection, Params, Row, params};

use super::sqlite_store::{SqliteStore, corrupt, db_error, epoch_param};
use crate::{
    adapter::{clock::epoch_now, sanitize::printable},
    domain::{
        config::{ConfigError, Registration, UnreadableProject},
        port::ProjectStore,
        project::Project,
        value::{ProjectName, ProjectNameError, ProjectRoot, ProjectRootError},
    },
};

/// Selects a project's row id, root and name, in that order.
const SELECT_PROJECT: &str = "SELECT id, root, name FROM projects";
/// What an unreadable project's refusal calls it, before its folder.
const PROJECT_AT: &str = "project at ";
/// Why a stored folder that is not absolute is refused.
const FOLDER_NOT_ABSOLUTE: &str = "its folder is not an absolute path";
/// Why a blank stored name is refused.
const NAME_EMPTY: &str = "its name is empty";
/// Why a stored name that could drive a terminal is refused.
const NAME_UNSAFE: &str = "its name holds a control or format character";

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

/// Reads a `SELECT_PROJECT` row: the project, or, when the domain refuses
/// its root or name, the folder it is stored at and why.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails.
fn project_from_row(row: &Row) -> Result<Result<ProjectRow, UnreadableProject>, ConfigError> {
    let id: i64 = row.get("id").map_err(db_error)?;
    let root: String = row.get("root").map_err(db_error)?;
    let name: String = row.get("name").map_err(db_error)?;
    let unreadable = |reason: &str| {
        UnreadableProject::builder()
            .root(PathBuf::from(&root))
            .reason(reason.to_string())
            .build()
    };
    let project_root = match ProjectRoot::try_new(PathBuf::from(&root)) {
        Ok(project_root) => project_root,
        Err(ProjectRootError::PredicateViolated) => {
            return Ok(Err(unreadable(FOLDER_NOT_ABSOLUTE)));
        },
    };
    let name = match ProjectName::try_new(name) {
        Ok(name) => name,
        Err(ProjectNameError::NotEmptyViolated) => return Ok(Err(unreadable(NAME_EMPTY))),
        Err(ProjectNameError::PredicateViolated) => return Ok(Err(unreadable(NAME_UNSAFE))),
    };
    Ok(Ok((
        id,
        Project::builder().root(project_root).name(name).build(),
    )))
}

/// Every project row `sql` selects with `values`, each read or refused.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails.
fn stored_where(
    conn: &Connection,
    sql: &str,
    values: impl Params,
) -> Result<Vec<Result<ProjectRow, UnreadableProject>>, ConfigError> {
    let mut statement = conn.prepare(sql).map_err(db_error)?;
    let mut rows = statement.query(values).map_err(db_error)?;
    let mut projects = Vec::new();
    while let Some(row) = rows.next().map_err(db_error)? {
        projects.push(project_from_row(row)?);
    }
    Ok(projects)
}

/// Every project row `sql` selects with `values` that reads, the others
/// left out.
///
/// # Errors
/// Returns a [`ConfigError`] when SQLite fails.
fn readable_where(
    conn: &Connection,
    sql: &str,
    values: impl Params,
) -> Result<Vec<ProjectRow>, ConfigError> {
    Ok(stored_where(conn, sql, values)?
        .into_iter()
        .filter_map(Result::ok)
        .collect())
}

/// The one project row `sql` selects with `values`, if any.
///
/// # Errors
/// Returns [`ConfigError::CorruptRow`] naming the project's folder when its
/// row does not read, or a [`ConfigError`] when SQLite fails.
fn one_where(
    conn: &Connection,
    sql: &str,
    values: impl Params,
) -> Result<Option<ProjectRow>, ConfigError> {
    match stored_where(conn, sql, values)?.pop() {
        None => Ok(None),
        Some(Ok(project)) => Ok(Some(project)),
        Some(Err(unreadable)) => Err(unreadable_row(&unreadable)),
    }
}

/// The refusal of the project `unreadable`, naming its folder.
fn unreadable_row(unreadable: &UnreadableProject) -> ConfigError {
    corrupt(
        &format!(
            "{PROJECT_AT}{}",
            printable(&unreadable.root().to_string_lossy())
        ),
        unreadable.reason(),
    )
}

/// The project registered at `root`, if any.
pub(super) fn find_project(
    conn: &Connection,
    root: &ProjectRoot,
) -> Result<Option<ProjectRow>, ConfigError> {
    one_where(conn, &format!("{SELECT_PROJECT} WHERE root = ?1"), params![
        root_text(root)?
    ])
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
/// transaction, so the name stays free, and no project is registered inside
/// `root`, until it commits: of two folders registered at once, one inside
/// the other, the outer one is refused whichever commits first.
///
/// # Errors
/// Returns [`ConfigError::HoldsProject`] when a registered project lies
/// inside `root`, or a store failure.
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
    if let Some((_, held)) = readable_where(conn, SELECT_PROJECT, [])?
        .into_iter()
        .find(|(_, other)| other.root().as_ref().starts_with(root.as_ref()))
    {
        return Err(ConfigError::HoldsProject {
            dir: root.as_ref().clone(),
            project: held.name().clone(),
        });
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
            params![root_text(root)?, name.as_ref(), epoch_param(epoch_now())],
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
                readable_where(conn, &format!("{SELECT_PROJECT} ORDER BY name"), [])?
                    .into_iter()
                    .map(|(_, project)| project)
                    .collect(),
            )
        })
    }

    fn unreadable_projects(&self) -> Result<Vec<UnreadableProject>, ConfigError> {
        self.read(|conn| {
            Ok(
                stored_where(conn, &format!("{SELECT_PROJECT} ORDER BY root"), [])?
                    .into_iter()
                    .filter_map(Result::err)
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
                one_where(conn, &format!("{SELECT_PROJECT} WHERE name = ?1"), params![
                    name.as_ref()
                ])?
                .map(|(_, project)| project),
            )
        })
    }

    fn project_containing(&self, dir: &Path) -> Result<Option<Project>, ConfigError> {
        self.read(|conn| {
            let deepest = stored_where(conn, SELECT_PROJECT, [])?
                .into_iter()
                .filter(|stored| match stored {
                    Ok((_, project)) => project.root().contains(dir),
                    Err(unreadable) => dir.starts_with(unreadable.root()),
                })
                .max_by_key(|stored| match stored {
                    Ok((_, project)) => project.root().as_ref().components().count(),
                    Err(unreadable) => unreadable.root().components().count(),
                });
            match deepest {
                None => Ok(None),
                Some(Ok((_, project))) => Ok(Some(project)),
                Some(Err(unreadable)) => Err(unreadable_row(&unreadable)),
            }
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

    fn remove_unreadable_project(&self, stored: &Path) -> Result<bool, ConfigError> {
        let Some(text) = stored.to_str() else {
            return Ok(false);
        };
        self.write(|conn| {
            let still_unreadable =
                stored_where(conn, &format!("{SELECT_PROJECT} WHERE root = ?1"), params![
                    text
                ])?
                .iter()
                .any(Result::is_err);
            if !still_unreadable {
                return Ok(false);
            }
            let removed = conn
                .execute("DELETE FROM projects WHERE root = ?1", params![text])
                .map_err(db_error)?;
            Ok(removed > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Barrier, thread};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::store::test_support::{database, fresh_store, spec},
        domain::{
            config::Environment,
            coordination::{Author, KvKey, KvValue},
            port::{CoordinationStore, StackReader, StackStore},
            value::{EnvName, EnvValue},
        },
    };

    /// How many times the registration race is run.
    const RACES: usize = 20;

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
            .add_process(&root, &spec("web", "npm run dev"), &Author::human())
            .unwrap();
        let second = store
            .add_process(&root, &spec("api", "cargo run"), &Author::human())
            .unwrap();

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
                    .add_process(
                        &folder(&scratch, relative),
                        &spec("web", "true"),
                        &Author::human(),
                    )
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

    /// A folder holding a project, which a caller's earlier check missed
    /// when the project inside it registered in between, is refused in the
    /// write itself, naming that project, and nothing of it is written; a
    /// sibling sharing a name prefix is not inside it.
    #[test]
    fn registering_a_folder_holding_a_project_is_refused_in_the_write() {
        let (store, scratch) = fresh_store();
        let outer = folder(&scratch, "a");
        let inner = folder(&scratch, "a/b");
        store
            .add_process(&inner, &spec("web", "true"), &Author::human())
            .unwrap();

        let refused = store
            .add_process(&outer, &spec("web", "true"), &Author::human())
            .unwrap_err();

        assert!(
            matches!(&refused, ConfigError::HoldsProject { dir, project }
                if dir == outer.as_ref() && project.as_ref() == "b"),
            "{refused:?}"
        );
        assert_eq!(store.project_at(&outer).unwrap(), None);
        assert_eq!(rows_in(&store, "processes"), 1);
        store
            .add_process(
                &folder(&scratch, "ab"),
                &spec("web", "true"),
                &Author::human(),
            )
            .unwrap();
    }

    /// A row whose stored root is not absolute is listed under that text and
    /// removed by it, with its processes; a readable project is never removed
    /// that way.
    #[test]
    fn an_unreadable_row_is_removed_by_the_text_it_is_listed_under() {
        let (store, scratch) = fresh_store();
        let shop = folder(&scratch, "shop");
        let blog = folder(&scratch, "blog");
        for root in [&shop, &blog] {
            store
                .add_process(root, &spec("web", "true"), &Author::human())
                .unwrap();
        }
        store
            .write(|conn| {
                conn.execute(
                    "UPDATE projects SET root = 'relative/blog' WHERE name = 'blog'",
                    [],
                )
                .map_err(db_error)
            })
            .unwrap();

        let unreadable = store.unreadable_projects().unwrap();
        assert_eq!(unreadable.len(), 1);
        let stored = unreadable[0].root().clone();
        assert_eq!(stored, PathBuf::from("relative/blog"));
        assert_eq!(unreadable[0].reason(), FOLDER_NOT_ABSOLUTE);

        assert!(!store.remove_unreadable_project(shop.as_ref()).unwrap());
        assert!(store.remove_unreadable_project(&stored).unwrap());
        assert!(!store.remove_unreadable_project(&stored).unwrap());
        assert!(store.unreadable_projects().unwrap().is_empty());
        assert_eq!(rows_in(&store, "processes"), 1);
        assert!(store.project_at(&shop).unwrap().is_some());
    }

    /// A project row whose name the domain refuses is left out of the
    /// listing and of other folders' lookups, named by its folder when looked
    /// up itself, listed as unreadable, and removable by its folder.
    #[test]
    fn a_corrupt_project_row_is_skipped_and_named_by_its_folder() {
        let (store, scratch) = fresh_store();
        let shop = folder(&scratch, "shop");
        let blog = folder(&scratch, "blog");
        for root in [&shop, &blog] {
            store
                .add_process(root, &spec("web", "true"), &Author::human())
                .unwrap();
        }
        store
            .write(|conn| {
                conn.execute("UPDATE projects SET name = 'x\u{7}' WHERE name = 'blog'", [
                ])
                .map_err(db_error)
            })
            .unwrap();

        let listed: Vec<String> = store
            .projects()
            .unwrap()
            .iter()
            .map(|project| project.name().to_string())
            .collect();
        assert_eq!(listed, ["shop"]);
        assert_eq!(
            store
                .project_containing(&shop.as_ref().join("src"))
                .unwrap(),
            store.project_at(&shop).unwrap()
        );
        for refused in [
            store.project_at(&blog).map(|_| ()),
            store
                .project_containing(&blog.as_ref().join("src"))
                .map(|_| ()),
        ] {
            let message = refused.unwrap_err().to_string();
            assert!(
                message.contains(&blog.as_ref().display().to_string()),
                "{message}"
            );
            assert!(!message.contains('\u{7}'), "{message}");
        }
        let unreadable = store.unreadable_projects().unwrap();
        assert_eq!(unreadable.len(), 1);
        assert_eq!(unreadable[0].root(), blog.as_ref());
        assert_eq!(unreadable[0].reason(), NAME_UNSAFE);

        assert!(store.remove_project(&blog).unwrap());
        assert!(store.unreadable_projects().unwrap().is_empty());
    }

    /// Adds racing to register a folder and one inside it never leave the
    /// outer folder registered after the inner one: whichever registers
    /// second, holding the other, is refused.
    #[test]
    fn concurrent_adds_never_register_a_folder_around_a_project() {
        for _ in 0..RACES {
            let (store, scratch) = fresh_store();
            let outer = folder(&scratch, "a");
            let inner = folder(&scratch, "a/b");
            let other = SqliteStore::open(&database(&scratch)).unwrap();
            let barrier = Barrier::new(2);
            let (outer_added, inner_added) = thread::scope(|scope| {
                let outer_add = scope.spawn(|| {
                    barrier.wait();
                    store.add_process(&outer, &spec("web", "true"), &Author::human())
                });
                let inner_add = scope.spawn(|| {
                    barrier.wait();
                    other.add_process(&inner, &spec("web", "true"), &Author::human())
                });
                (outer_add.join().unwrap(), inner_add.join().unwrap())
            });

            assert!(inner_added.is_ok(), "{inner_added:?}");
            if outer_added.is_ok() {
                let id = |root: &ProjectRoot| {
                    store
                        .read(|conn| Ok(require_project(conn, root)?.0))
                        .unwrap()
                };
                assert!(
                    id(&outer) < id(&inner),
                    "the outer folder registered second"
                );
            } else {
                assert!(matches!(outer_added, Err(ConfigError::HoldsProject { .. })));
            }
        }
    }

    /// The deepest registered ancestor wins, and containment is by whole
    /// components.
    #[test]
    fn project_containing_picks_the_deepest_root_by_components() {
        let (store, scratch) = fresh_store();
        let outer = folder(&scratch, "a");
        let inner = folder(&scratch, "a/b");
        store
            .add_process(&outer, &spec("web", "true"), &Author::human())
            .unwrap();
        store
            .add_process(&inner, &spec("web", "true"), &Author::human())
            .unwrap();
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
        store
            .add_process(&root, &with_port, &Author::human())
            .unwrap();
        store
            .add_process(&other, &with_port, &Author::human())
            .unwrap();
        store
            .set_value(
                &root,
                &Author::human(),
                &KvKey::try_new("port").unwrap(),
                &KvValue::try_new("3000").unwrap(),
            )
            .unwrap();
        store
            .set_value(
                &other,
                &Author::human(),
                &KvKey::try_new("port").unwrap(),
                &KvValue::try_new("4000").unwrap(),
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

//! Merging two BDE configurations.
//!
//! A "minor" configuration is merged into a "major" one.  The major
//! configuration wins: when the same variable is present in both files the
//! value from the major file is kept.  Variables and containers that appear
//! only in the minor file are added to the result.  Containers are matched by
//! name and merged recursively, so the rules apply at every nesting level.

use crate::bdeconf::{Config, Container, Entry};

/// Merges `minor` into `major`, returning the combined configuration.
///
/// The major configuration wins on conflicts.  Entries present only in the
/// minor configuration are appended.  Matching is by name and is applied
/// recursively to nested containers.
pub fn merge(major: &Config, minor: &Config) -> Config {
    Config::new(merge_containers(&major.root, &minor.root))
}

fn merge_containers(major: &Container, minor: &Container) -> Container {
    let mut merged = major.clone();

    for entry in &minor.entries {
        match entry {
            Entry::Variable(variable) => {
                if !contains_variable(&merged, &variable.name) {
                    merged.entries.push(entry.clone());
                }
            }
            Entry::Container(container) => match find_container(&merged, &container.name) {
                Some(index) => {
                    let child = match &merged.entries[index] {
                        Entry::Container(existing) => merge_containers(existing, container),
                        Entry::Variable(_) => unreachable!("index points at a container"),
                    };
                    merged.entries[index] = Entry::Container(child);
                }
                None => merged.entries.push(entry.clone()),
            },
        }
    }

    merged
}

/// Whether `container` already holds a variable with the given name.
fn contains_variable(container: &Container, name: &str) -> bool {
    container
        .entries
        .iter()
        .any(|entry| matches!(entry, Entry::Variable(v) if v.name == name))
}

/// Position of the first nested container with the given name, if any.
fn find_container(container: &Container, name: &str) -> Option<usize> {
    container
        .entries
        .iter()
        .position(|entry| matches!(entry, Entry::Container(c) if c.name == name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(root: Container) -> Config {
        Config::new(root)
    }

    #[test]
    fn keeps_major_values_and_adds_minor_variables() {
        let major = config({
            let mut root = Container::new("ROOT");
            root.push_variable("shared", "major");
            root.push_variable("only-major", "1");
            root
        });
        let minor = config({
            let mut root = Container::new("ROOT");
            root.push_variable("shared", "minor");
            root.push_variable("only-minor", "2");
            root
        });

        let merged = merge(&major, &minor);
        assert_eq!(
            merged.root.entries,
            vec![
                Entry::Variable(crate::bdeconf::Variable::new("shared", "major")),
                Entry::Variable(crate::bdeconf::Variable::new("only-major", "1")),
                Entry::Variable(crate::bdeconf::Variable::new("only-minor", "2")),
            ]
        );
    }

    #[test]
    fn merges_nested_containers_recursively() {
        let major = config({
            let mut root = Container::new("ROOT");
            let mut nested = Container::new("NESTED");
            nested.push_variable("shared", "major");
            root.push_container(nested);
            root
        });
        let minor = config({
            let mut root = Container::new("ROOT");
            let mut nested = Container::new("NESTED");
            nested.push_variable("shared", "minor");
            nested.push_variable("only-minor", "2");
            root.push_container(nested);
            root
        });

        let merged = merge(&major, &minor);
        let nested = match &merged.root.entries[0] {
            Entry::Container(container) => container,
            other => panic!("expected a container, got {other:?}"),
        };
        assert_eq!(
            nested.entries,
            vec![
                Entry::Variable(crate::bdeconf::Variable::new("shared", "major")),
                Entry::Variable(crate::bdeconf::Variable::new("only-minor", "2")),
            ]
        );
    }

    #[test]
    fn adds_minor_only_containers() {
        let major = config({
            let mut root = Container::new("ROOT");
            root.push_container(Container::new("A"));
            root
        });
        let minor = config({
            let mut root = Container::new("ROOT");
            root.push_container(Container::new("A"));
            root.push_container(Container::new("B"));
            root
        });

        let merged = merge(&major, &minor);
        let names: Vec<&str> = merged
            .root
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Container(container) => Some(container.name.as_str()),
                Entry::Variable(_) => None,
            })
            .collect();
        assert_eq!(names, vec!["A", "B"]);
    }

    fn variable(name: &str, value: &str) -> Entry {
        Entry::Variable(crate::bdeconf::Variable::new(name, value))
    }

    #[test]
    fn merging_with_an_empty_minor_keeps_the_major_unchanged() {
        let major = config({
            let mut root = Container::new("ROOT");
            root.push_variable("a", "1");
            root.push_container({
                let mut c = Container::new("NESTED");
                c.push_variable("b", "2");
                c
            });
            root
        });
        let minor = config(Container::new("ROOT"));

        assert_eq!(merge(&major, &minor), major);
    }

    #[test]
    fn merging_with_an_empty_major_takes_the_minor() {
        let major = config(Container::new("ROOT"));
        let minor = config({
            let mut root = Container::new("ROOT");
            root.push_variable("a", "1");
            root.push_container(Container::new("C"));
            root
        });

        let merged = merge(&major, &minor);
        assert_eq!(merged, minor);
    }

    #[test]
    fn keeps_the_major_root_name() {
        let major = config(Container::new("MAJOR"));
        let minor = config(Container::new("MINOR"));
        let merged = merge(&major, &minor);
        // The root container names are not reconciled; the major one wins.
        assert_eq!(merged.root.name, "MAJOR");
    }

    #[test]
    fn merging_a_configuration_into_itself_is_a_no_op() {
        let mut root = Container::new("ROOT");
        root.push_variable("a", "1");
        let mut nested = Container::new("NESTED");
        nested.push_variable("b", "2");
        root.push_container(nested);
        let input = config(root);

        assert_eq!(merge(&input, &input), input);
    }

    #[test]
    fn preserves_the_major_entry_order_and_appends_minor_entries() {
        let major = config({
            let mut root = Container::new("ROOT");
            root.push_variable("a", "1");
            root.push_container(Container::new("C"));
            root
        });
        let minor = config({
            let mut root = Container::new("ROOT");
            root.push_variable("x", "new");
            root.push_container({
                let mut c = Container::new("D");
                c.push_variable("z", "9");
                c
            });
            root
        });

        let merged = merge(&major, &minor);
        assert_eq!(
            merged.root.entries,
            vec![
                variable("a", "1"),
                Entry::Container(Container::new("C")),
                variable("x", "new"),
                Entry::Container({
                    let mut c = Container::new("D");
                    c.push_variable("z", "9");
                    c
                }),
            ]
        );
    }

    #[test]
    fn matches_containers_by_name_at_every_nesting_level() {
        let major = config({
            let mut root = Container::new("ROOT");
            let mut a = Container::new("A");
            let mut a1 = Container::new("A1");
            a1.push_variable("shared", "major");
            a.push_container(a1);
            a.push_variable("only-major", "1");
            root.push_container(a);
            root
        });
        let minor = config({
            let mut root = Container::new("ROOT");
            let mut a = Container::new("A");
            let mut a1 = Container::new("A1");
            a1.push_variable("shared", "minor");
            a1.push_variable("only-minor", "2");
            a.push_container(a1);
            a.push_variable("only-minor-top", "3");
            root.push_container(a);
            root
        });

        let merged = merge(&major, &minor);
        let a = match &merged.root.entries[0] {
            Entry::Container(c) => c,
            other => panic!("expected container A, got {other:?}"),
        };
        let a1 = match &a.entries[0] {
            Entry::Container(c) => c,
            other => panic!("expected container A1, got {other:?}"),
        };
        assert_eq!(
            a1.entries,
            vec![variable("shared", "major"), variable("only-minor", "2")]
        );
        assert_eq!(
            a.entries[1],
            variable("only-major", "1"),
            "major entry order is kept"
        );
        assert_eq!(a.entries[2], variable("only-minor-top", "3"));
    }

    #[test]
    fn does_not_repeatedly_append_duplicate_names() {
        // Both files contain two containers named "A"; each pair is matched and
        // merged, so the result holds exactly one "A".
        let mut major_root = Container::new("ROOT");
        major_root.push_container(Container::new("A"));
        major_root.push_container(Container::new("A"));
        let major = config(major_root);

        let mut minor_root = Container::new("ROOT");
        let mut a1 = Container::new("A");
        a1.push_variable("new", "1");
        minor_root.push_container(a1);
        let mut a2 = Container::new("A");
        a2.push_variable("other", "2");
        minor_root.push_container(a2);
        let minor = config(minor_root);

        let merged = merge(&major, &minor);
        let count = merged
            .root
            .entries
            .iter()
            .filter(|entry| matches!(entry, Entry::Container(c) if c.name == "A"))
            .count();
        assert_eq!(count, 2);
    }
}

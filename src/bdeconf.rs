//! In-memory representation of a Borland Database Engine (BDE) configuration
//! file (`IDAPI.CFG` / `IDAPI32.CFG`).
//!
//! The binary file is a tree: a single top-level ("root") [`Container`] holds
//! variables and nested containers.  The order of the entries is significant
//! and is preserved exactly so that a file can be exported and imported again
//! without changes.

/// A complete BDE configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The single top-level container stored in the file.
    pub root: Container,
}

impl Config {
    /// Creates a configuration from its root container.
    pub fn new(root: Container) -> Self {
        Self { root }
    }
}

/// A named container holding variables and nested containers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    /// Container name (for example a section or an alias name).
    pub name: String,
    /// Entries in their original order.
    pub entries: Vec<Entry>,
}

impl Container {
    /// Creates an empty container with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: Vec::new(),
        }
    }

    /// Appends a variable to this container.
    #[allow(dead_code)]
    pub fn push_variable(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.entries
            .push(Entry::Variable(Variable::new(name, value)));
    }

    /// Appends a nested container to this container.
    #[allow(dead_code)]
    pub fn push_container(&mut self, container: Container) {
        self.entries.push(Entry::Container(container));
    }
}

/// A single entry inside a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A nested container.
    Container(Container),
    /// A name/value pair.
    Variable(Variable),
}

/// A name/value pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    /// Variable name.
    pub name: String,
    /// Variable value.
    pub value: String,
}

impl Variable {
    /// Creates a variable with the given name and value.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_config_from_root() {
        let config = Config::new(Container::new("ROOT"));
        assert_eq!(config.root, Container::new("ROOT"));
    }

    #[test]
    fn new_container_is_empty() {
        let container = Container::new("EMPTY");
        assert_eq!(container.name, "EMPTY");
        assert!(container.entries.is_empty());
    }

    #[test]
    fn push_variable_appends_in_order() {
        let mut container = Container::new("C");
        container.push_variable("a", "1");
        container.push_variable("b", "2");
        assert_eq!(
            container.entries,
            vec![
                Entry::Variable(Variable::new("a", "1")),
                Entry::Variable(Variable::new("b", "2")),
            ]
        );
    }

    #[test]
    fn push_container_appends_a_nested_container() {
        let mut root = Container::new("ROOT");
        let mut child = Container::new("CHILD");
        child.push_variable("x", "y");
        root.push_container(child.clone());
        assert_eq!(root.entries, vec![Entry::Container(child)]);
    }

    #[test]
    fn accepts_owned_and_borrowed_strings() {
        let variable = Variable::new(String::from("name"), String::from("value"));
        assert_eq!(variable.name, "name");
        assert_eq!(variable.value, "value");

        let container = Container::new(String::from("name"));
        assert_eq!(container.name, "name");
    }

    #[test]
    fn derives_equality_and_clone() {
        let mut root = Container::new("ROOT");
        root.push_variable("k", "v");
        let config = Config::new(root);

        let clone = config.clone();
        assert_eq!(config, clone);
        assert_ne!(config, Config::new(Container::new("OTHER")));
    }
}

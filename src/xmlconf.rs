//! Conversion between a [`Config`] and the XML representation.
//!
//! The XML document mirrors the tree stored in the BDE file:
//!
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <bde-config version="1">
//!   <container name="ROOT">
//!     <variable name="key" value="value"/>
//!     <container name="NESTED">
//!       <variable name="other" value="42"/>
//!     </container>
//!   </container>
//! </bde-config>
//! ```

use anyhow::{Context, Result, bail};
use quick_xml::XmlVersion;
use quick_xml::events::BytesStart;
use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::bdeconf::{Config, Container, Entry, Variable};

/// Name of the document element.
pub const DOCUMENT_ELEMENT: &str = "bde-config";
/// Name of container elements.
pub const CONTAINER_ELEMENT: &str = "container";
/// Name of variable elements.
pub const VARIABLE_ELEMENT: &str = "variable";
/// Name of the `name` attribute.
pub const NAME_ATTRIBUTE: &str = "name";
/// Name of the `value` attribute.
pub const VALUE_ATTRIBUTE: &str = "value";
/// Value of the `version` attribute written to new documents.
pub const FORMAT_VERSION: &str = "1";

/// Renders a configuration as an XML document.
///
/// `xml_encoding_name` is written to the XML declaration and must match the
/// encoding the returned text is later encoded with.
pub fn to_xml_string(config: &Config, xml_encoding_name: &str) -> Result<String> {
    let mut out = String::new();
    out.push_str(r#"<?xml version="1.0" encoding=""#);
    out.push_str(xml_encoding_name);
    out.push_str("\"?>\n");
    out.push('<');
    out.push_str(DOCUMENT_ELEMENT);
    out.push_str(" version=\"");
    out.push_str(FORMAT_VERSION);
    out.push_str("\">\n");
    write_container(&config.root, &mut out, 1)?;
    out.push_str("</");
    out.push_str(DOCUMENT_ELEMENT);
    out.push_str(">\n");
    Ok(out)
}

fn write_container(container: &Container, out: &mut String, depth: usize) -> Result<()> {
    let indent = "  ".repeat(depth);
    out.push_str(&indent);
    out.push('<');
    out.push_str(CONTAINER_ELEMENT);
    out.push(' ');
    out.push_str(NAME_ATTRIBUTE);
    out.push_str("=\"");
    out.push_str(&escape_attribute(&container.name)?);
    out.push_str("\">\n");

    for entry in &container.entries {
        match entry {
            Entry::Container(child) => write_container(child, out, depth + 1)?,
            Entry::Variable(variable) => {
                out.push_str(&"  ".repeat(depth + 1));
                out.push('<');
                out.push_str(VARIABLE_ELEMENT);
                out.push(' ');
                out.push_str(NAME_ATTRIBUTE);
                out.push_str("=\"");
                out.push_str(&escape_attribute(&variable.name)?);
                out.push_str("\" ");
                out.push_str(VALUE_ATTRIBUTE);
                out.push_str("=\"");
                out.push_str(&escape_attribute(&variable.value)?);
                out.push_str("\"/>\n");
            }
        }
    }

    out.push_str(&indent);
    out.push_str("</");
    out.push_str(CONTAINER_ELEMENT);
    out.push_str(">\n");
    Ok(())
}

/// Parses an XML document into a [`Config`].
pub fn from_xml_string(xml: &str) -> Result<Config> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    reader.config_mut().expand_empty_elements = true;

    let mut buffer = Vec::new();
    let mut stack: Vec<Container> = Vec::new();
    let mut root: Option<Container> = None;
    let mut saw_document_element = false;

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .with_context(|| format!("invalid XML at byte {}", reader.error_position()))?;

        match event {
            Event::Eof => break,
            Event::Start(element) => match element.name().as_ref() {
                "bde-config" => {
                    if saw_document_element {
                        bail!("the document contains a nested <{DOCUMENT_ELEMENT}> element");
                    }
                    saw_document_element = true;
                }
                "container" => {
                    let name = attribute(&element, NAME_ATTRIBUTE)?.unwrap_or_default();
                    stack.push(Container::new(name));
                }
                "variable" => {
                    let name = attribute(&element, NAME_ATTRIBUTE)?
                        .context("a <variable> element is missing its 'name' attribute")?;
                    let value = attribute(&element, VALUE_ATTRIBUTE)?
                        .context("a <variable> element is missing its 'value' attribute")?;
                    let container = stack.last_mut().context(
                        "a <variable> element appears outside of any <container> element",
                    )?;
                    container
                        .entries
                        .push(Entry::Variable(Variable::new(name, value)));
                }
                other => bail!("unexpected XML element <{other}>"),
            },
            Event::End(element) if element.name().as_ref() == "container" => {
                let finished = stack.pop().context("unbalanced </container> element")?;
                match stack.last_mut() {
                    Some(parent) => parent.entries.push(Entry::Container(finished)),
                    None => {
                        if root.is_some() {
                            bail!("the document contains more than one top-level container");
                        }
                        root = Some(finished);
                    }
                }
            }
            _ => {}
        }
        buffer.clear();
    }

    if !saw_document_element {
        bail!("the document has no <{DOCUMENT_ELEMENT}> root element");
    }
    let root = root.context("the document does not contain a top-level <container> element")?;
    Ok(Config::new(root))
}

fn attribute(element: &BytesStart<'_>, name: &str) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute.context("invalid attribute")?;
        if attribute.key.as_ref() == name {
            let value = attribute
                .normalized_value(XmlVersion::Explicit1_0)
                .with_context(|| format!("invalid value for attribute '{name}'"))?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn escape_attribute(value: &str) -> Result<String> {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => {
                let code = c as u32;
                if code < 0x20 || code == 0xFFFE || code == 0xFFFF {
                    bail!(
                        "character U+{code:04X} cannot be represented in XML 1.0; \
                         the BDE file cannot be exported without losing data"
                    );
                }
                out.push(c);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> Config {
        let mut root = Container::new("ROOT");
        root.push_variable("alpha", "1");
        let mut nested = Container::new("NESTED");
        nested.push_variable("beta", "2 & 3 < 4");
        nested.push_variable("empty", "");
        root.push_container(nested);
        root.push_variable("gamma", "line1\nline2\ttabbed");
        Config::new(root)
    }

    #[test]
    fn round_trips_through_xml() {
        let config = sample_config();
        let xml = to_xml_string(&config, "UTF-8").unwrap();
        let parsed = from_xml_string(&xml).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn escapes_special_characters() {
        let mut root = Container::new("A&B");
        root.push_variable("q\"uote", "<>");
        let xml = to_xml_string(&Config::new(root), "UTF-8").unwrap();
        assert!(xml.contains("name=\"A&amp;B\""));
        assert!(xml.contains("name=\"q&quot;uote\""));
        assert!(xml.contains("value=\"&lt;&gt;\""));
        assert_eq!(
            from_xml_string(&xml).unwrap(),
            Config::new({
                let mut c = Container::new("A&B");
                c.push_variable("q\"uote", "<>");
                c
            })
        );
    }

    #[test]
    fn rejects_control_characters() {
        let mut root = Container::new("R");
        root.push_variable("bad", "a\u{1}b");
        assert!(to_xml_string(&Config::new(root), "UTF-8").is_err());
    }

    #[test]
    fn rejects_xml_non_characters() {
        let mut root = Container::new("R");
        root.push_variable("bad", "a\u{FFFE}b");
        assert!(to_xml_string(&Config::new(root), "UTF-8").is_err());
    }

    #[test]
    fn parses_a_minimal_document() {
        let xml = "<bde-config version=\"1\"><container name=\"ROOT\"/></bde-config>";
        let config = from_xml_string(xml).unwrap();
        assert_eq!(config.root, Container::new("ROOT"));
    }

    #[test]
    fn parses_documents_without_a_declaration() {
        let xml = "<bde-config><container name=\"R\"><variable name=\"k\" value=\"v\"/></container></bde-config>";
        let config = from_xml_string(xml).unwrap();
        assert_eq!(
            config.root.entries,
            vec![Entry::Variable(Variable::new("k", "v"))]
        );
    }

    #[test]
    fn preserves_entry_order() {
        let xml = r#"<bde-config>
          <container name="R">
            <variable name="a" value="1"/>
            <container name="C"><variable name="b" value="2"/></container>
            <variable name="c" value="3"/>
          </container>
        </bde-config>"#;
        let config = from_xml_string(xml).unwrap();
        assert_eq!(
            config.root.entries,
            vec![
                Entry::Variable(Variable::new("a", "1")),
                Entry::Container({
                    let mut c = Container::new("C");
                    c.push_variable("b", "2");
                    c
                }),
                Entry::Variable(Variable::new("c", "3")),
            ]
        );
    }

    #[test]
    fn container_without_a_name_gets_an_empty_name() {
        let xml =
            "<bde-config><container><variable name=\"k\" value=\"v\"/></container></bde-config>";
        let config = from_xml_string(xml).unwrap();
        assert_eq!(config.root.name, "");
    }

    #[test]
    fn parses_an_empty_top_level_container() {
        let xml = "<bde-config><container name=\"EMPTY\"/></bde-config>";
        let config = from_xml_string(xml).unwrap();
        assert_eq!(config.root, Container::new("EMPTY"));
    }

    #[test]
    fn from_xml_trims_text_whitespace() {
        let xml = "<bde-config>\n  <container name=\"ROOT\">\n  </container>\n</bde-config>";
        let config = from_xml_string(xml).unwrap();
        assert_eq!(config.root.entries, Vec::new());
    }

    #[test]
    fn rejects_a_document_without_the_document_element() {
        let xml = "<container name=\"R\"/>";
        let error = from_xml_string(xml).unwrap_err();
        assert!(format!("{error:#}").contains("bde-config"));
    }

    #[test]
    fn rejects_a_document_without_a_container() {
        let xml = "<bde-config version=\"1\"/>";
        let error = from_xml_string(xml).unwrap_err();
        assert!(
            format!("{error:#}").contains("top-level <container>"),
            "{error:#}"
        );
    }

    #[test]
    fn rejects_more_than_one_top_level_container() {
        let xml = "<bde-config><container name=\"A\"/><container name=\"B\"/></bde-config>";
        let error = from_xml_string(xml).unwrap_err();
        assert!(format!("{error:#}").contains("more than one top-level container"));
    }

    #[test]
    fn rejects_a_variable_outside_a_container() {
        let xml = "<bde-config><variable name=\"k\" value=\"v\"/></bde-config>";
        let error = from_xml_string(xml).unwrap_err();
        assert!(format!("{error:#}").contains("outside of any <container>"));
    }

    #[test]
    fn rejects_a_variable_missing_its_name() {
        let xml =
            "<bde-config><container name=\"R\"><variable value=\"v\"/></container></bde-config>";
        let error = from_xml_string(xml).unwrap_err();
        assert!(format!("{error:#}").contains("missing its 'name'"));
    }

    #[test]
    fn rejects_a_variable_missing_its_value() {
        let xml =
            "<bde-config><container name=\"R\"><variable name=\"k\"/></container></bde-config>";
        let error = from_xml_string(xml).unwrap_err();
        assert!(format!("{error:#}").contains("missing its 'value'"));
    }

    #[test]
    fn rejects_an_unbalanced_container_end() {
        // quick-xml itself flags the mismatched closing tag.
        let xml = "<bde-config></container></bde-config>";
        let error = from_xml_string(xml).unwrap_err();
        assert!(format!("{error:#}").contains("invalid XML"), "{error:#}");
    }

    #[test]
    fn rejects_an_end_without_an_open_container() {
        // The document element closes before the container; quick-xml reports
        // the mismatched closing tag rather than delivering it to us.
        let xml = "<bde-config><container name=\"R\"></bde-config>";
        let error = from_xml_string(xml).unwrap_err();
        assert!(format!("{error:#}").contains("invalid XML"), "{error:#}");
    }

    #[test]
    fn rejects_unexpected_elements() {
        let xml = "<bde-config><container name=\"R\"><foo/></container></bde-config>";
        let error = from_xml_string(xml).unwrap_err();
        assert!(format!("{error:#}").contains("unexpected XML element <foo>"));
    }

    #[test]
    fn rejects_a_nested_document_element() {
        let xml = "<bde-config><container name=\"R\"><bde-config/></container></bde-config>";
        let error = from_xml_string(xml).unwrap_err();
        assert!(
            format!("{error:#}").contains("nested <bde-config>"),
            "{error:#}"
        );
    }

    #[test]
    fn rejects_malformed_xml() {
        let error = from_xml_string("<bde-config><container").unwrap_err();
        assert!(format!("{error:#}").contains("invalid XML"));
    }

    #[test]
    fn rejects_duplicate_attributes() {
        // quick-xml reports a duplicate attribute as an error.
        let xml = r#"<bde-config><container name="R"><variable name="k" name="k2" value="v"/></container></bde-config>"#;
        let error = from_xml_string(xml).unwrap_err();
        assert!(format!("{error:#}").contains("invalid"));
    }

    #[test]
    fn escapes_all_xml_special_characters() {
        let mut root = Container::new("A&B<C>\"D\" 'E'\t\n");
        root.push_variable("k&<>\t\n\r\"'", "&\t\n\r\"'<>");
        let xml = to_xml_string(&Config::new(root.clone()), "UTF-8").unwrap();
        assert!(
            xml.contains(r#"name="A&amp;B&lt;C&gt;&quot;D&quot; &apos;E&apos;&#9;&#10;""#),
            "{xml}"
        );
        assert!(
            xml.contains(r#"name="k&amp;&lt;&gt;&#9;&#10;&#13;&quot;&apos;""#),
            "{xml}"
        );
        assert!(
            xml.contains(r#"value="&amp;&#9;&#10;&#13;&quot;&apos;&lt;&gt;""#),
            "{xml}"
        );

        let parsed = from_xml_string(&xml).unwrap();
        assert_eq!(parsed, Config::new(root));
    }

    #[test]
    fn round_trips_unicode_without_escaping() {
        let mut root = Container::new("РОУТ");
        root.push_variable("ключ", "значение € 𝔀");
        let config = Config::new(root);

        let xml = to_xml_string(&config, "UTF-8").unwrap();
        assert!(xml.contains("значение € 𝔀"), "{xml}");
        assert_eq!(from_xml_string(&xml).unwrap(), config);
    }

    #[test]
    fn output_contains_the_requested_encoding_declaration() {
        let xml = to_xml_string(&sample_config(), "windows-1251").unwrap();
        assert!(
            xml.starts_with("<?xml version=\"1.0\" encoding=\"windows-1251\"?>\n"),
            "{xml}"
        );
    }

    #[test]
    fn output_mirrors_the_document_structure() {
        let xml = to_xml_string(&sample_config(), "UTF-8").unwrap();
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(xml.contains("<bde-config version=\"1\">"));
        assert!(xml.contains("</bde-config>\n"), "trailing newline expected");
        assert!(
            xml.contains("  <container name=\"ROOT\">"),
            "root container is indented by one level"
        );
        assert!(
            xml.contains("    <variable name=\"alpha\" value=\"1\"/>"),
            "variables are indented by two levels"
        );
        assert!(
            xml.contains("      <variable name=\"beta\" value=\"2 &amp; 3 &lt; 4\"/>"),
            "nested variables are indented by three levels"
        );
    }

    #[test]
    fn from_xml_ignores_comments_and_declarations() {
        let xml =
            "<?xml version=\"1.0\"?><!--comment--><bde-config><container name=\"R\"/></bde-config>";
        let config = from_xml_string(xml).unwrap();
        assert_eq!(config.root, Container::new("R"));
    }
}

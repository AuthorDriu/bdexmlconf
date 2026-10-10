//! Parser for the binary Borland BDE configuration format.
//!
//! The format is a stream of tagged records:
//!
//! ```text
//! record start          : 0x03 0x00
//! container record      : 0x00 <NUL-terminated name> 0x01 0x00
//! variable record       : 0x01 <NUL-terminated name> 0x04 <NUL-terminated value>
//! end of block          : 0x02 0x00
//! ```
//!
//! Every string is stored as `0x00 <bytes...> 0x00`.  A container is opened by
//! its record and closed by an end-of-block marker after all of its children;
//! the outermost container is closed implicitly at the end of the file (an
//! explicit marker is tolerated as well).

use anyhow::{Context, Result, bail};

use crate::bdeconf::{Config, Container, Entry, Variable};
use crate::encoding::BdeCodec;

const TAG_NEW: u8 = 0x03;
const TAG_CONTAINER: u8 = 0x00;
const TAG_VARIABLE: u8 = 0x01;
const TAG_END_OF_BLOCK: u8 = 0x02;
const TAG_EQUALS: u8 = 0x04;
const SEPARATOR: u8 = 0x00;

/// Parses the bytes of a BDE configuration file into a [`Config`].
pub fn parse(bytes: &[u8], codec: &BdeCodec) -> Result<Config> {
    Parser {
        data: bytes,
        pos: 0,
        codec,
    }
    .parse_config()
}

struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
    codec: &'a BdeCodec,
}

impl Parser<'_> {
    fn read_u8(&mut self) -> Result<u8> {
        let byte = *self.data.get(self.pos).with_context(|| {
            format!(
                "unexpected end of BDE configuration file at offset {}",
                self.pos
            )
        })?;
        self.pos += 1;
        Ok(byte)
    }

    fn expect(&mut self, expected: u8, what: &str) -> Result<()> {
        let offset = self.pos;
        let actual = self.read_u8()?;
        if actual != expected {
            bail!(
                "invalid BDE configuration file at offset {offset}: expected {what} \
                 (0x{expected:02X}), found 0x{actual:02X}"
            );
        }
        Ok(())
    }

    fn read_string(&mut self) -> Result<String> {
        let offset = self.pos;
        self.expect(SEPARATOR, "the start of a string")?;
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != SEPARATOR {
            self.pos += 1;
        }
        if self.pos >= self.data.len() {
            bail!("unterminated string starting at offset {offset}");
        }
        let raw = &self.data[start..self.pos];
        self.pos += 1; // Consume the terminating NUL.
        Ok(self.codec.decode(raw))
    }

    fn parse_config(&mut self) -> Result<Config> {
        // Containers that have been opened but not yet closed.  The bottom of
        // the stack is the top-level container; it is never popped by an
        // end-of-block marker.
        let mut stack: Vec<Container> = Vec::new();

        while self.pos < self.data.len() {
            let offset = self.pos;
            let tag = self.read_u8()?;

            match tag {
                TAG_NEW => {
                    self.expect(SEPARATOR, "a record separator")?;
                    let kind = self.read_u8()?;
                    match kind {
                        TAG_CONTAINER => {
                            let name = self.read_string()?;
                            self.expect(TAG_VARIABLE, "a container terminator")?;
                            self.expect(SEPARATOR, "a container terminator")?;
                            stack.push(Container::new(name));
                        }
                        TAG_VARIABLE => {
                            let name = self.read_string()?;
                            self.expect(TAG_EQUALS, "'=' between name and value")?;
                            let value = self.read_string()?;
                            let container = stack.last_mut().with_context(|| {
                                format!(
                                    "variable '{name}' at offset {offset} is not inside a container"
                                )
                            })?;
                            container
                                .entries
                                .push(Entry::Variable(Variable::new(name, value)));
                        }
                        other => bail!(
                            "invalid BDE configuration file at offset {offset}: \
                             unknown record type 0x{other:02X}"
                        ),
                    }
                }
                TAG_END_OF_BLOCK => {
                    self.expect(SEPARATOR, "an end-of-block terminator")?;
                    if stack.len() > 1 {
                        let finished = stack.pop().expect("stack is not empty");
                        stack
                            .last_mut()
                            .expect("stack has a parent")
                            .entries
                            .push(Entry::Container(finished));
                    }
                }
                other => bail!(
                    "invalid BDE configuration file at offset {offset}: expected a record \
                     start (0x03) or an end-of-block marker (0x02), found 0x{other:02X}"
                ),
            }
        }

        // Close containers that were left open.  Only the top-level container
        // is allowed to remain open.
        while stack.len() > 1 {
            let finished = stack.pop().expect("stack is not empty");
            stack
                .last_mut()
                .expect("stack has a parent")
                .entries
                .push(Entry::Container(finished));
        }

        let root = stack
            .pop()
            .with_context(|| "invalid BDE configuration file: no container found")?;
        Ok(Config::new(root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codec() -> BdeCodec {
        BdeCodec::Raw
    }

    fn cp1251() -> BdeCodec {
        BdeCodec::CodePage(encoding_rs::WINDOWS_1251)
    }

    /// Builds the byte sequence for a container header (`03 00 00 <name> 01 00`).
    fn container(name: &[u8]) -> Vec<u8> {
        let mut out = vec![0x03, 0x00, 0x00, 0x00];
        out.extend_from_slice(name);
        out.extend_from_slice(&[0x00, 0x01, 0x00]);
        out
    }

    /// Builds the byte sequence for a variable (`03 00 01 <name> 00 04 00 <value> 00`).
    fn variable(name: &[u8], value: &[u8]) -> Vec<u8> {
        let mut out = vec![0x03, 0x00, 0x01, 0x00];
        out.extend_from_slice(name);
        out.extend_from_slice(&[0x00, 0x04, 0x00]);
        out.extend_from_slice(value);
        out.push(0x00);
        out
    }

    #[test]
    fn parses_simple_configuration() {
        let bytes = [
            0x03, 0x00, 0x00, 0x00, b'A', 0x00, 0x01, 0x00, // container "A"
            0x03, 0x00, 0x01, 0x00, b'x', 0x00, 0x04, 0x00, b'1', 0x00, // x = 1
            0x02, 0x00, // end of block
        ];
        let config = parse(&bytes, &codec()).unwrap();
        assert_eq!(config.root.name, "A");
        assert_eq!(config.root.entries.len(), 1);
        assert_eq!(
            config.root.entries[0],
            Entry::Variable(Variable::new("x", "1"))
        );
    }

    #[test]
    fn parses_nested_containers() {
        let bytes = [
            0x03, 0x00, 0x00, 0x00, b'R', 0x00, 0x01, 0x00, // root "R"
            0x03, 0x00, 0x00, 0x00, b'S', 0x00, 0x01, 0x00, // sub "S"
            0x03, 0x00, 0x01, 0x00, b'y', 0x00, 0x04, 0x00, b'2', 0x00, // y = 2
            0x02, 0x00, // close "S"
            0x03, 0x00, 0x01, 0x00, b'z', 0x00, 0x04, 0x00, b'3', 0x00, // z = 3
            0x02, 0x00, // close "R" (tolerated)
        ];
        let config = parse(&bytes, &codec()).unwrap();
        assert_eq!(config.root.name, "R");
        assert_eq!(
            config.root.entries,
            vec![
                Entry::Container({
                    let mut c = Container::new("S");
                    c.push_variable("y", "2");
                    c
                }),
                Entry::Variable(Variable::new("z", "3")),
            ]
        );
    }

    #[test]
    fn rejects_truncated_input() {
        assert!(parse(&[0x03, 0x00, 0x00], &codec()).is_err());
    }

    #[test]
    fn parses_reference_writer_trailing_markers() {
        // [root A, variable v, container B, variable w], as emitted by the
        // reference tool with two trailing end-of-block markers.
        let bytes = [
            0x03, 0x00, 0x00, 0x00, b'A', 0x00, 0x01, 0x00, //
            0x03, 0x00, 0x01, 0x00, b'v', 0x00, 0x04, 0x00, b'1', 0x00, //
            0x03, 0x00, 0x00, 0x00, b'B', 0x00, 0x01, 0x00, //
            0x03, 0x00, 0x01, 0x00, b'w', 0x00, 0x04, 0x00, b'2', 0x00, //
            0x02, 0x00, //
            0x02, 0x00, //
        ];
        let config = parse(&bytes, &codec()).unwrap();
        assert_eq!(config.root.name, "A");
        assert_eq!(
            config.root.entries,
            vec![
                Entry::Variable(Variable::new("v", "1")),
                Entry::Container({
                    let mut c = Container::new("B");
                    c.push_variable("w", "2");
                    c
                }),
            ]
        );
    }

    #[test]
    fn parses_implicitly_closed_root() {
        // The last entry is a variable and there is no trailing marker.
        let bytes = [
            0x03, 0x00, 0x00, 0x00, b'A', 0x00, 0x01, 0x00, //
            0x03, 0x00, 0x01, 0x00, b'v', 0x00, 0x04, 0x00, b'1', 0x00, //
        ];
        let config = parse(&bytes, &codec()).unwrap();
        assert_eq!(
            config.root.entries,
            vec![Entry::Variable(Variable::new("v", "1"))]
        );
    }

    #[test]
    fn parses_container_as_last_entry() {
        let bytes = [
            0x03, 0x00, 0x00, 0x00, b'A', 0x00, 0x01, 0x00, //
            0x03, 0x00, 0x00, 0x00, b'B', 0x00, 0x01, 0x00, //
        ];
        let config = parse(&bytes, &codec()).unwrap();
        assert_eq!(
            config.root.entries,
            vec![Entry::Container(Container::new("B"))]
        );
    }

    #[test]
    fn built_helpers_match_expected_layout() {
        assert_eq!(
            container(b"X"),
            vec![0x03, 0x00, 0x00, 0x00, b'X', 0x00, 0x01, 0x00]
        );
        assert_eq!(
            variable(b"k", b"v"),
            vec![0x03, 0x00, 0x01, 0x00, b'k', 0x00, 0x04, 0x00, b'v', 0x00]
        );
    }

    #[test]
    fn parses_multiple_variables_between_containers() {
        let mut bytes = container(b"ROOT");
        bytes.extend(variable(b"a", b"1"));
        bytes.extend(container(b"CHILD"));
        bytes.extend(variable(b"b", b"2"));
        bytes.extend([0x02, 0x00]);
        bytes.extend(variable(b"c", b"3"));
        bytes.extend([0x02, 0x00]);

        let config = parse(&bytes, &codec()).unwrap();
        assert_eq!(
            config.root.entries,
            vec![
                Entry::Variable(Variable::new("a", "1")),
                Entry::Container({
                    let mut child = Container::new("CHILD");
                    child.push_variable("b", "2");
                    child
                }),
                Entry::Variable(Variable::new("c", "3")),
            ]
        );
    }

    #[test]
    fn parses_deeply_nested_containers() {
        let mut bytes = container(b"L0");
        bytes.extend(container(b"L1"));
        bytes.extend(container(b"L2"));
        bytes.extend(variable(b"deep", b"yes"));
        bytes.extend([0x02, 0x00]);
        bytes.extend([0x02, 0x00]);
        bytes.extend([0x02, 0x00]);

        let config = parse(&bytes, &codec()).unwrap();
        let mut current = &config.root;
        for level in ["L1", "L2"] {
            let child = match &current.entries[0] {
                Entry::Container(child) => child,
                other => panic!("expected container at {level}, got {other:?}"),
            };
            assert_eq!(child.name, level);
            current = child;
        }
        assert_eq!(
            current.entries,
            vec![Entry::Variable(Variable::new("deep", "yes"))]
        );
    }

    #[test]
    fn parses_empty_container() {
        let bytes = [&container(b"EMPTY")[..], &[0x02, 0x00]].concat();
        let config = parse(&bytes, &codec()).unwrap();
        assert_eq!(config.root, Container::new("EMPTY"));
    }

    #[test]
    fn parses_empty_names_and_values() {
        let mut bytes = container(b"");
        bytes.extend(variable(b"", b""));
        let config = parse(&bytes, &codec()).unwrap();
        assert_eq!(config.root.name, "");
        assert_eq!(
            config.root.entries,
            vec![Entry::Variable(Variable::new("", ""))]
        );
    }

    #[test]
    fn decodes_text_with_the_selected_code_page() {
        // "Привет" in windows-1251, used both as a container name and a value.
        let cyrillic = b"\xCF\xF0\xE8\xE2\xE5\xF2";
        let mut bytes = container(cyrillic);
        bytes.extend(variable(cyrillic, cyrillic));

        let config = parse(&bytes, &cp1251()).unwrap();
        assert_eq!(config.root.name, "Привет");
        assert_eq!(
            config.root.entries,
            vec![Entry::Variable(Variable::new("Привет", "Привет"))]
        );
    }

    #[test]
    fn raw_codec_preserves_high_bytes() {
        let config = parse(&container(b"\x80\xFF"), &codec()).unwrap();
        assert_eq!(config.root.name, "\u{80}\u{FF}");
    }

    #[test]
    fn rejects_empty_input() {
        let error = parse(&[], &codec()).unwrap_err();
        assert!(format!("{error:#}").contains("no container found"));
    }

    #[test]
    fn rejects_truncated_record_start() {
        assert!(parse(&[0x03], &codec()).is_err());
    }

    #[test]
    fn rejects_missing_record_separator() {
        // 0x03 must be followed by 0x00.
        assert!(parse(&[0x03, 0x01, 0x00], &codec()).is_err());
    }

    #[test]
    fn rejects_unknown_record_type() {
        let error = parse(&[0x03, 0x00, 0x05], &codec()).unwrap_err();
        assert!(format!("{error:#}").contains("unknown record type"));
    }

    #[test]
    fn rejects_invalid_record_start() {
        let error = parse(&[0xFF], &codec()).unwrap_err();
        assert!(format!("{error:#}").contains("expected a record start"));
    }

    #[test]
    fn rejects_wrong_container_terminator() {
        // Terminator after the name must be 0x01 0x00.
        assert!(parse(&[0x03, 0x00, 0x00, 0x00, b'A', 0x00, 0x02, 0x00], &codec()).is_err());
    }

    #[test]
    fn rejects_missing_equals_between_name_and_value() {
        // The byte between the name and the value must be 0x04.
        let bytes = [0x03, 0x00, 0x01, 0x00, b'k', 0x00, 0x05, 0x00, b'v', 0x00];
        assert!(parse(&bytes, &codec()).is_err());
    }

    #[test]
    fn rejects_variable_outside_any_container() {
        let error = parse(&variable(b"k", b"v"), &codec()).unwrap_err();
        assert!(format!("{error:#}").contains("not inside a container"));
    }

    #[test]
    fn rejects_unterminated_string() {
        // The name string is opened but never closed before the end of input.
        assert!(parse(&[0x03, 0x00, 0x01, 0x00, b'k'], &codec()).is_err());
    }

    #[test]
    fn rejects_missing_end_of_block_separator() {
        // 0x02 must be followed by 0x00.
        let mut bytes = container(b"ROOT");
        bytes.push(0x02);
        bytes.push(0x01);
        assert!(parse(&bytes, &codec()).is_err());
    }

    #[test]
    fn accepts_missing_end_of_block_separator_after_root_only() {
        // A trailing 0x02 0x00 closing the root is allowed; an extra one is
        // ignored because the root is never popped.
        let mut bytes = container(b"ROOT");
        bytes.extend([0x02, 0x00, 0x02, 0x00]);
        let config = parse(&bytes, &codec()).unwrap();
        assert_eq!(config.root, Container::new("ROOT"));
    }

    #[test]
    fn error_messages_report_offsets() {
        let error = parse(&[0x03, 0x00, 0x05], &codec()).unwrap_err();
        assert!(format!("{error:#}").contains("unknown record type"));
        assert!(format!("{error:#}").contains("at offset 0"));
    }
}

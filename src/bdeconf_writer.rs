//! Serializer for the binary Borland BDE configuration format.
//!
//! See [`crate::bdeconf_reader`] for a description of the byte layout.

use anyhow::{Result, bail};

use crate::bdeconf::{Config, Container, Entry};
use crate::encoding::BdeCodec;

const TAG_NEW: u8 = 0x03;
const TAG_CONTAINER: u8 = 0x00;
const TAG_VARIABLE: u8 = 0x01;
const TAG_END_OF_BLOCK: u8 = 0x02;
const TAG_EQUALS: u8 = 0x04;
const SEPARATOR: u8 = 0x00;

/// Serializes a [`Config`] into the bytes of a BDE configuration file.
pub fn serialize(config: &Config, codec: &BdeCodec) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    write_container(&config.root, &mut out, codec)?;
    Ok(out)
}

fn write_record_start(out: &mut Vec<u8>) {
    out.push(TAG_NEW);
    out.push(SEPARATOR);
}

fn write_string(out: &mut Vec<u8>, text: &str, codec: &BdeCodec) -> Result<()> {
    let bytes = codec.encode(text)?;
    if bytes.contains(&SEPARATOR) {
        bail!("text {text:?} contains a NUL byte and cannot be stored in a BDE file");
    }
    out.push(SEPARATOR);
    out.extend_from_slice(&bytes);
    out.push(SEPARATOR);
    Ok(())
}

fn write_container(container: &Container, out: &mut Vec<u8>, codec: &BdeCodec) -> Result<()> {
    write_record_start(out);
    out.push(TAG_CONTAINER);
    write_string(out, &container.name, codec)?;
    out.push(TAG_VARIABLE);
    out.push(SEPARATOR);

    for entry in &container.entries {
        match entry {
            Entry::Variable(variable) => {
                write_record_start(out);
                out.push(TAG_VARIABLE);
                write_string(out, &variable.name, codec)?;
                out.push(TAG_EQUALS);
                write_string(out, &variable.value, codec)?;
            }
            Entry::Container(child) => write_container(child, out, codec)?,
        }
    }

    // Close the container (including the outermost one).
    out.push(TAG_END_OF_BLOCK);
    out.push(SEPARATOR);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bdeconf_reader;

    fn codec() -> BdeCodec {
        BdeCodec::Raw
    }

    #[test]
    fn writes_expected_bytes() {
        let mut root = Container::new("A");
        root.push_variable("x", "1");
        let config = Config::new(root);

        let bytes = serialize(&config, &codec()).unwrap();
        assert_eq!(
            bytes,
            vec![
                0x03, 0x00, 0x00, 0x00, b'A', 0x00, 0x01, 0x00, //
                0x03, 0x00, 0x01, 0x00, b'x', 0x00, 0x04, 0x00, b'1', 0x00, //
                0x02, 0x00,
            ]
        );
    }

    #[test]
    fn round_trips_through_parser() {
        let mut root = Container::new("ROOT");
        root.push_variable("alpha", "1");
        let mut nested = Container::new("NESTED");
        nested.push_variable("beta", "2");
        nested.push_variable("gamma", "with spaces & symbols <>");
        root.push_container(nested);
        root.push_variable("omega", "");
        let config = Config::new(root);

        let bytes = serialize(&config, &codec()).unwrap();
        let parsed = bdeconf_reader::parse(&bytes, &codec()).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn writes_trailing_end_of_block_following_the_root() {
        let config = Config::new(Container::new("A"));
        let bytes = serialize(&config, &codec()).unwrap();
        assert_eq!(
            bytes,
            vec![0x03, 0x00, 0x00, 0x00, b'A', 0x00, 0x01, 0x00, 0x02, 0x00]
        );
    }

    #[test]
    fn serializes_an_empty_value() {
        let mut root = Container::new("R");
        root.push_variable("empty", "");
        let bytes = serialize(&Config::new(root), &codec()).unwrap();
        assert_eq!(
            bytes,
            vec![
                0x03, 0x00, 0x00, 0x00, b'R', 0x00, 0x01, 0x00, //
                0x03, 0x00, 0x01, 0x00, b'e', b'm', b'p', b't', b'y', 0x00, //
                0x04, 0x00, 0x00, //
                0x02, 0x00,
            ]
        );
    }

    #[test]
    fn serializes_nested_and_empty_containers() {
        let mut root = Container::new("ROOT");
        root.push_container(Container::new("EMPTY"));
        let mut nested = Container::new("NESTED");
        nested.push_variable("k", "v");
        root.push_container(nested);

        let bytes = serialize(&Config::new(root), &codec()).unwrap();
        let parsed = bdeconf_reader::parse(&bytes, &codec()).unwrap();
        assert_eq!(
            parsed.root.entries,
            vec![
                Entry::Container(Container::new("EMPTY")),
                Entry::Container({
                    let mut c = Container::new("NESTED");
                    c.push_variable("k", "v");
                    c
                }),
            ]
        );
    }

    #[test]
    fn rejects_nul_byte_in_the_name() {
        let mut root = Container::new("ROOT");
        root.push_variable("a\u{0}b", "value");
        assert!(serialize(&Config::new(root), &codec()).is_err());
    }

    #[test]
    fn rejects_nul_byte_in_the_value() {
        let mut root = Container::new("ROOT");
        root.push_variable("key", "a\u{0}b");
        assert!(serialize(&Config::new(root), &codec()).is_err());
    }

    #[test]
    fn rejects_nul_byte_in_the_container_name() {
        assert!(serialize(&Config::new(Container::new("a\u{0}b")), &codec()).is_err());
    }

    #[test]
    fn raw_codec_rejects_characters_above_ff() {
        let mut root = Container::new("ROOT");
        root.push_variable("key", "Привет");
        let error = serialize(&Config::new(root), &codec()).unwrap_err();
        assert!(format!("{error:#}").contains("--bde-encoding"), "{error:#}");
    }

    #[test]
    fn code_page_round_trips_cyrillic() {
        let mut root = Container::new("IDAPI");
        root.push_variable("ХОСТ", "Сервер");
        let config = Config::new(root);
        let codec = BdeCodec::CodePage(encoding_rs::WINDOWS_1251);

        let bytes = serialize(&config, &codec).unwrap();
        let parsed = bdeconf_reader::parse(&bytes, &codec).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn code_page_rejects_unmappable_characters() {
        let mut root = Container::new("ROOT");
        root.push_variable("key", "漢");
        let codec = BdeCodec::CodePage(encoding_rs::WINDOWS_1251);
        assert!(serialize(&Config::new(root), &codec).is_err());
    }

    #[test]
    fn round_trips_high_byte_values_with_raw_codec() {
        // A name and value that use every byte value in the upper range must
        // survive a write/read cycle unchanged.
        let name = (0x80u8..=0xFF).map(|b| b as char).collect::<String>();
        let value = "text \u{FF} end";
        let mut root = Container::new(&name);
        root.push_variable("key", value);
        let config = Config::new(root);

        let bytes = serialize(&config, &codec()).unwrap();
        let parsed = bdeconf_reader::parse(&bytes, &codec()).unwrap();
        assert_eq!(parsed.root.name, name);
        assert_eq!(
            parsed.root.entries,
            vec![Entry::Variable(crate::bdeconf::Variable::new("key", value))]
        );
    }
}

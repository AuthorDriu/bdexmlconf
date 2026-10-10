//! Character-encoding helpers.
//!
//! Two unrelated encodings are involved:
//!
//! * the encoding of the XML document, selected with `--encoding`; and
//! * the (implicit) encoding of text stored inside the binary BDE file.
//!
//! The binary BDE format is byte oriented and predates Unicode.  By default we
//! therefore map bytes to code points one to one (ISO-8859-1, called `raw`
//! here).  This guarantees that `export` followed by `import` is lossless for
//! every possible byte value.  A real code page (for example `windows-1251`)
//! can be selected with `--bde-encoding` when the file is known to use it.

use anyhow::{Context, Result, bail};
use encoding_rs::Encoding;

/// Codec used to translate text stored in the binary BDE file.
#[derive(Debug, Clone, Copy, Default)]
pub enum BdeCodec {
    /// Lossless ISO-8859-1 mapping (byte value == code point).
    #[default]
    Raw,
    /// A named legacy code page.
    CodePage(&'static Encoding),
}

impl BdeCodec {
    /// Decodes raw bytes read from a BDE file into a string.
    pub fn decode(&self, bytes: &[u8]) -> String {
        match self {
            BdeCodec::Raw => bytes.iter().map(|&b| b as char).collect(),
            BdeCodec::CodePage(encoding) => {
                let (text, _, _) = encoding.decode(bytes);
                text.into_owned()
            }
        }
    }

    /// Encodes a string for storage in a BDE file.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>> {
        match self {
            BdeCodec::Raw => {
                let mut out = Vec::with_capacity(text.len());
                for ch in text.chars() {
                    let code = ch as u32;
                    if code > 0xFF {
                        bail!(
                            "character {ch:?} (U+{code:04X}) cannot be stored in a BDE file \
                             with the lossless 'raw' byte mapping; select a suitable \
                             --bde-encoding"
                        );
                    }
                    out.push(code as u8);
                }
                Ok(out)
            }
            BdeCodec::CodePage(encoding) => {
                let (bytes, _, had_errors) = encoding.encode(text);
                if had_errors {
                    bail!(
                        "text contains characters that cannot be represented in {}",
                        encoding.name()
                    );
                }
                Ok(bytes.into_owned())
            }
        }
    }
}

/// Resolves the encoding used for BDE file text (`--bde-encoding`).
pub fn parse_bde_codec(name: &str) -> Result<BdeCodec> {
    let trimmed = name.trim();
    if trimmed.eq_ignore_ascii_case("raw")
        || trimmed.eq_ignore_ascii_case("latin1")
        || trimmed.eq_ignore_ascii_case("iso-8859-1")
    {
        return Ok(BdeCodec::Raw);
    }
    let encoding =
        lookup_encoding(trimmed).with_context(|| format!("unknown BDE encoding '{name}'"))?;
    Ok(BdeCodec::CodePage(encoding))
}

/// Resolves the encoding used for the XML document (`--encoding`).
pub fn parse_xml_encoding(name: &str) -> Result<&'static Encoding> {
    lookup_encoding(name.trim()).with_context(|| format!("unknown XML encoding '{name}'"))
}

fn lookup_encoding(name: &str) -> Option<&'static Encoding> {
    if let Some(encoding) = Encoding::for_label(name.as_bytes()) {
        return Some(encoding);
    }
    // Also accept bare numeric code pages such as "1251" or "cp866".
    let digits = name
        .trim_start_matches(|c: char| c.is_ascii_alphabetic())
        .trim();
    if let Ok(number) = digits.parse::<u16>() {
        let label = format!("windows-{number}");
        if let Some(encoding) = Encoding::for_label(label.as_bytes()) {
            return Some(encoding);
        }
    }
    None
}

/// Detects the encoding of an XML document from a byte-order mark or the XML
/// declaration.  Returns `None` when nothing conclusive was found.
pub fn detect_xml_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    if let Some((encoding, _)) = Encoding::for_bom(bytes) {
        return Some(encoding);
    }
    detect_encoding_from_declaration(bytes)
}

/// Decodes the bytes of an XML document.
///
/// When `forced` is set it wins over any detection; otherwise a byte-order
/// mark or the `encoding` pseudo-attribute of the XML declaration is used,
/// falling back to UTF-8.
pub fn decode_xml(bytes: &[u8], forced: Option<&'static Encoding>) -> Result<String> {
    let encoding = forced
        .or_else(|| detect_xml_encoding(bytes))
        .unwrap_or(encoding_rs::UTF_8);

    let (text, had_errors) = encoding.decode_with_bom_removal(bytes);
    if had_errors {
        bail!("input is not valid {}", encoding.name());
    }
    Ok(text.into_owned())
}

/// Encodes XML text using the requested encoding.
///
/// Characters that the target encoding cannot represent are written as
/// numeric character references, which keeps the document valid.
pub fn encode_xml(text: &str, encoding: &'static Encoding) -> Vec<u8> {
    let (bytes, _, _) = encoding.encode(text);
    bytes.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_codec_round_trips_every_byte() {
        let codec = parse_bde_codec("raw").unwrap();
        let bytes: Vec<u8> = (0u8..=255).collect();
        let text = codec.decode(&bytes);
        assert_eq!(codec.encode(&text).unwrap(), bytes);
    }

    #[test]
    fn raw_codec_is_also_spelled_latin1() {
        for name in ["raw", "RAW", "latin1", "iso-8859-1"] {
            let codec = parse_bde_codec(name).unwrap();
            assert_eq!(codec.decode(&[0xE9]), "é", "name = {name}");
        }
    }

    #[test]
    fn raw_codec_rejects_characters_above_ff() {
        let codec = parse_bde_codec("raw").unwrap();
        assert!(codec.encode("Привет").is_err());
    }

    #[test]
    fn code_page_round_trips_cyrillic() {
        let codec = parse_bde_codec("windows-1251").unwrap();
        let encoded = codec.encode("Привет").unwrap();
        assert_eq!(encoded, vec![0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]);
        assert_eq!(codec.decode(&encoded), "Привет");
    }

    #[test]
    fn rejects_unknown_bde_encoding() {
        assert!(parse_bde_codec("does-not-exist").is_err());
    }

    #[test]
    fn resolves_xml_encodings_by_name_and_number() {
        assert_eq!(parse_xml_encoding("UTF-8").unwrap(), encoding_rs::UTF_8);
        assert_eq!(
            parse_xml_encoding("windows-1251").unwrap(),
            encoding_rs::WINDOWS_1251
        );
        // A bare numeric code page is accepted as well.
        assert_eq!(
            parse_xml_encoding("1251").unwrap(),
            encoding_rs::WINDOWS_1251
        );
        assert_eq!(parse_xml_encoding("866").unwrap(), encoding_rs::IBM866);
        assert!(parse_xml_encoding("nope").is_err());
    }

    #[test]
    fn detects_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"<x/>");
        assert_eq!(detect_xml_encoding(&bytes), Some(encoding_rs::UTF_8));
    }

    #[test]
    fn detects_encoding_from_declaration() {
        let xml = b"<?xml version=\"1.0\" encoding=\"windows-1251\"?><x/>";
        assert_eq!(detect_xml_encoding(xml), Some(encoding_rs::WINDOWS_1251));
        // Single quotes and surrounding whitespace are tolerated.
        let xml = b"  <?xml version='1.0' encoding = 'cp866' ?><x/>";
        assert_eq!(detect_xml_encoding(xml), Some(encoding_rs::IBM866));
    }

    #[test]
    fn declaration_is_ignored_when_absent() {
        assert_eq!(detect_xml_encoding(b"<x/>"), None);
        // A declaration without an encoding pseudo-attribute.
        assert_eq!(detect_xml_encoding(b"<?xml version=\"1.0\"?><x/>"), None);
    }

    #[test]
    fn decode_xml_honours_forced_encoding() {
        let bytes = [0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]; // "Привет" in cp1251.
        assert_eq!(
            decode_xml(&bytes, Some(encoding_rs::WINDOWS_1251)).unwrap(),
            "Привет"
        );
        // Without a hint UTF-8 is assumed and the bytes are invalid.
        assert!(decode_xml(&bytes, None).is_err());
    }

    #[test]
    fn encode_xml_uses_numeric_references_for_unmappable_characters() {
        // windows-1251 has no mapping for U+6F22, so the encoder must fall
        // back to a numeric character reference to keep the document valid.
        let bytes = encode_xml("a漢b", encoding_rs::WINDOWS_1251);
        assert_eq!(bytes, b"a&#28450;b".to_vec());
    }

    #[test]
    fn bde_codec_defaults_to_raw() {
        assert!(matches!(BdeCodec::default(), BdeCodec::Raw));
    }

    #[test]
    fn parse_bde_codec_accepts_code_pages_and_numbers() {
        assert!(matches!(
            parse_bde_codec("windows-1251").unwrap(),
            BdeCodec::CodePage(enc) if enc == encoding_rs::WINDOWS_1251
        ));
        assert!(matches!(
            parse_bde_codec("cp866").unwrap(),
            BdeCodec::CodePage(enc) if enc == encoding_rs::IBM866
        ));
        assert!(matches!(
            parse_bde_codec("866").unwrap(),
            BdeCodec::CodePage(enc) if enc == encoding_rs::IBM866
        ));
    }

    #[test]
    fn parse_bde_codec_trims_and_ignores_case() {
        for name in ["  raw  ", "RAW", "RaW", " latin1 ", "ISO-8859-1"] {
            assert!(
                matches!(parse_bde_codec(name).unwrap(), BdeCodec::Raw),
                "name = {name}"
            );
        }
        // Unmapped code pages are still resolved from their number.
        assert!(matches!(
            parse_bde_codec(" cp1251 ").unwrap(),
            BdeCodec::CodePage(enc) if enc == encoding_rs::WINDOWS_1251
        ));
    }

    #[test]
    fn parse_xml_encoding_ignores_aliases_and_case() {
        assert_eq!(
            parse_xml_encoding("WINDOWS-1251").unwrap(),
            encoding_rs::WINDOWS_1251
        );
        assert_eq!(
            parse_xml_encoding("cp1251").unwrap(),
            encoding_rs::WINDOWS_1251
        );
        assert_eq!(
            parse_xml_encoding("ks_c_5601-1987").unwrap(),
            encoding_rs::EUC_KR
        );
    }

    #[test]
    fn decode_xml_strips_a_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"<x/>");
        assert_eq!(decode_xml(&bytes, None).unwrap(), "<x/>");
    }

    #[test]
    fn decode_xml_detects_a_utf16_bom() {
        let bytes = [0xFF, 0xFE, b'<', 0x00, b'x', 0x00, b'/', 0x00, b'>', 0x00];
        assert_eq!(decode_xml(&bytes, None).unwrap(), "<x/>");
    }

    #[test]
    fn decode_xml_prefers_an_explicit_encoding_over_the_declaration() {
        // The declaration claims UTF-8, but the bytes are windows-1251.
        // With no hint the declared (wrong) encoding is used and decoding
        // fails; forcing the right encoding must win.
        let text = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><doc>Привет</doc>";
        let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(text);

        assert!(decode_xml(&bytes, None).is_err());
        assert_eq!(
            decode_xml(&bytes, Some(encoding_rs::WINDOWS_1251)).unwrap(),
            text
        );
    }

    #[test]
    fn decode_xml_detects_encoding_from_declaration() {
        let xml = b"<?xml version=\"1.0\" encoding=\"cp866\"?><x/>";
        assert_eq!(
            decode_xml(xml, None).unwrap(),
            "<?xml version=\"1.0\" encoding=\"cp866\"?><x/>"
        );
    }

    #[test]
    fn decode_xml_rejects_bad_utf8_without_a_declaration() {
        // 0xC3 0x28 is an invalid UTF-8 sequence and carries no BOM, so the
        // UTF-8 fallback must refuse it.
        let bytes = b"\xC3\x28 not utf8";
        assert!(decode_xml(bytes, None).is_err());
    }

    #[test]
    fn detect_xml_encoding_prefers_bom_over_declaration() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"<?xml version=\"1.0\" encoding=\"windows-1251\"?>");
        // Even though the declaration names windows-1251, the BOM wins.
        assert_eq!(detect_xml_encoding(&bytes), Some(encoding_rs::UTF_8));
    }

    #[test]
    fn declaration_detection_tolerates_crlf_and_spacing() {
        let xml = b"\r\n<?xml version=\"1.0\"  encoding = \"UTF-8\" ?>\n<x/>";
        assert_eq!(detect_xml_encoding(xml), Some(encoding_rs::UTF_8));
    }

    #[test]
    fn declaration_detection_ignores_similar_text_without_equals() {
        // A declaration where "encoding" is not followed by '=' must be ignored.
        let xml = b"<?xml version=\"1.0\" encoding?> <x/>";
        assert_eq!(detect_xml_encoding(xml), None);
    }

    #[test]
    fn declaration_detection_requires_a_closing_quote() {
        let xml = b"<?xml version=\"1.0\" encoding=\"UTF-8?><x/>";
        assert_eq!(detect_xml_encoding(xml), None);
    }

    #[test]
    fn encode_xml_round_trips_through_decode_xml() {
        for encoding in [
            encoding_rs::UTF_8,
            encoding_rs::WINDOWS_1251,
            encoding_rs::IBM866,
        ] {
            let text = "value \u{42} \u{43}"; // ASCII stays identical in all of them.
            let bytes = encode_xml(text, encoding);
            let decoded = decode_xml(&bytes, Some(encoding)).unwrap();
            assert_eq!(decoded, text, "encoding = {}", encoding.name());
        }
    }

    #[test]
    fn utf8_encoding_is_byte_identity() {
        // For UTF-8, encode must not introduce any escapes for representable
        // characters — the raw bytes come back unchanged.
        let text = "a&=<>\t\n\r\"' /\u{20AC}";
        let bytes = encode_xml(text, encoding_rs::UTF_8);
        assert_eq!(bytes, text.as_bytes());
    }
}

fn detect_encoding_from_declaration(bytes: &[u8]) -> Option<&'static Encoding> {
    let head = &bytes[..bytes.len().min(256)];
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start();
    if !text.starts_with("<?xml") {
        return None;
    }
    let start = text.find("encoding")? + "encoding".len();
    let rest = text[start..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[1..];
    let end = rest.find(quote)?;
    let name = rest[..end].trim();
    lookup_encoding(name)
}

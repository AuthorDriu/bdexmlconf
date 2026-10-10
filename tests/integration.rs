//! End-to-end tests that drive the compiled `bdexmlconf` binary.

use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_bdexmlconf");

/// Wraps raw bytes in the NUL-delimited form used for BDE strings.
fn s_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    out.extend_from_slice(bytes);
    out.push(0);
    out
}

fn container_bytes(name: &[u8]) -> Vec<u8> {
    let mut out = vec![0x03, 0x00, 0x00];
    out.extend(s_bytes(name));
    out.extend([0x01, 0x00]);
    out
}

fn variable_bytes(name: &[u8], value: &[u8]) -> Vec<u8> {
    let mut out = vec![0x03, 0x00, 0x01];
    out.extend(s_bytes(name));
    out.push(0x04);
    out.extend(s_bytes(value));
    out
}

fn container(name: &str) -> Vec<u8> {
    container_bytes(name.as_bytes())
}

fn variable(name: &str, value: &str) -> Vec<u8> {
    variable_bytes(name.as_bytes(), value.as_bytes())
}

fn end_of_block() -> [u8; 2] {
    [0x02, 0x00]
}

fn sample_config() -> Vec<u8> {
    let mut data = Vec::new();
    data.extend(container("IDAPI"));
    data.extend(variable("VERSION", "5.0"));
    data.extend(container("ALIASES"));
    data.extend(container("MyAlias"));
    data.extend(variable("PATH", "C:\\Data"));
    data.extend(variable("DRIVER", "PARADOX"));
    data.extend(end_of_block());
    data.extend(end_of_block());
    data.extend(container("PARADOX"));
    data.extend(variable("NET DIR", ""));
    data.extend(end_of_block());
    data.extend(end_of_block());
    data
}

/// A configuration whose names and values are Cyrillic text stored in cp1251.
fn sample_config_cp1251() -> Vec<u8> {
    let encode = |text: &str| -> Vec<u8> {
        let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(text);
        bytes.into_owned()
    };

    let mut data = Vec::new();
    data.extend(container("IDAPI"));
    data.extend(variable("VERSION", "v5"));
    data.extend(container_bytes(&encode("РАЗДЕЛ")));
    data.extend(variable_bytes(&encode("ПУТЬ"), &encode("Данные")));
    data.extend(end_of_block());
    data.extend(end_of_block());
    data
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let unique = format!(
            "bdexmlconf-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        );
        let path = std::env::temp_dir().join(unique);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("failed to run bdexmlconf")
}

#[test]
fn export_then_import_is_lossless() {
    let dir = TempDir::new("roundtrip");
    let cfg = dir.file("sample.cfg");
    let xml = dir.file("sample.xml");
    let copy = dir.file("copy.cfg");

    std::fs::write(&cfg, sample_config()).unwrap();

    let output = run(&["export", "-i", &cfg, "-o", &xml]);
    assert!(output.status.success(), "export failed: {output:?}");
    let xml_text = std::fs::read_to_string(&xml).unwrap();
    assert!(
        xml_text.contains("<container name=\"IDAPI\">"),
        "{xml_text}"
    );
    assert!(
        xml_text.contains("<variable name=\"PATH\" value=\"C:\\Data\"/>"),
        "{xml_text}"
    );

    let output = run(&["import", "-i", &xml, "-o", &copy]);
    assert!(output.status.success(), "import failed: {output:?}");
    assert_eq!(std::fs::read(&cfg).unwrap(), std::fs::read(&copy).unwrap());
}

#[test]
fn import_refuses_to_overwrite_without_rewrite() {
    let dir = TempDir::new("overwrite");
    let cfg = dir.file("sample.cfg");
    let xml = dir.file("sample.xml");
    let target = dir.file("target.cfg");

    std::fs::write(&cfg, sample_config()).unwrap();
    assert!(run(&["export", "-i", &cfg, "-o", &xml]).status.success());

    let original = b"do not touch me".to_vec();
    std::fs::write(&target, &original).unwrap();

    let output = run(&["import", "-i", &xml, "-o", &target]);
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&target).unwrap(), original);

    let output = run(&["import", "-i", &xml, "-o", &target, "--rewrite"]);
    assert!(output.status.success(), "rewrite failed: {output:?}");
    assert_ne!(std::fs::read(&target).unwrap(), original);
    assert!(!output.stderr.is_empty(), "expected a warning on stderr");
}

#[test]
fn rewrite_leaves_no_backup_file() {
    let dir = TempDir::new("no-backup");
    let cfg = dir.file("sample.cfg");
    let xml = dir.file("sample.xml");
    let target = dir.file("target.cfg");

    std::fs::write(&cfg, sample_config()).unwrap();
    assert!(run(&["export", "-i", &cfg, "-o", &xml]).status.success());

    std::fs::write(&target, b"old contents").unwrap();

    let output = run(&["import", "-i", &xml, "-o", &target, "--rewrite"]);
    assert!(output.status.success(), "rewrite failed: {output:?}");

    assert_ne!(std::fs::read(&target).unwrap(), b"old contents");
    assert!(
        !std::path::Path::new(&format!("{target}.bak")).exists(),
        "the backup file must be removed after a successful write"
    );
}

#[test]
fn honours_requested_xml_encoding() {
    let dir = TempDir::new("encoding");
    let cfg = dir.file("sample.cfg");
    let xml = dir.file("sample.xml");

    std::fs::write(&cfg, sample_config()).unwrap();
    let output = run(&["export", "-i", &cfg, "-o", &xml, "-e", "windows-1251"]);
    assert!(output.status.success(), "export failed: {output:?}");
    let bytes = std::fs::read(&xml).unwrap();
    assert!(
        bytes.starts_with(b"<?xml version=\"1.0\" encoding=\"windows-1251\"?>"),
        "unexpected declaration"
    );
}

const MAJOR_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<bde-config version="1">
  <container name="IDAPI">
    <variable name="VERSION" value="5.0"/>
    <container name="ALIASES">
      <container name="MyAlias">
        <variable name="PATH" value="C:\Major"/>
        <variable name="DRIVER" value="PARADOX"/>
      </container>
    </container>
  </container>
</bde-config>
"#;

const MINOR_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<bde-config version="1">
  <container name="IDAPI">
    <variable name="VERSION" value="9.9"/>
    <variable name="NEW" value="added"/>
    <container name="ALIASES">
      <container name="MyAlias">
        <variable name="PATH" value="C:\Minor"/>
        <variable name="LANGDRIVER" value="DBWINRUS"/>
      </container>
      <container name="OtherAlias">
        <variable name="DRIVER" value="DBASE"/>
      </container>
    </container>
  </container>
</bde-config>
"#;

#[test]
fn merge_keeps_major_values_and_adds_minor_entries() {
    let dir = TempDir::new("merge");
    let major = dir.file("major.xml");
    let minor = dir.file("minor.xml");
    let out = dir.file("merged.xml");
    std::fs::write(&major, MAJOR_XML).unwrap();
    std::fs::write(&minor, MINOR_XML).unwrap();

    let output = run(&["merge", "-M", &major, "-m", &minor, "-o", &out]);
    assert!(output.status.success(), "merge failed: {output:?}");
    let merged = std::fs::read_to_string(&out).unwrap();

    // Values from the major file win on conflicts.
    assert!(
        merged.contains(r#"<variable name="VERSION" value="5.0"/>"#),
        "{merged}"
    );
    assert!(
        merged.contains(r#"<variable name="PATH" value="C:\Major"/>"#),
        "{merged}"
    );
    // Variables that exist only in the minor file are added.
    assert!(
        merged.contains(r#"<variable name="NEW" value="added"/>"#),
        "{merged}"
    );
    assert!(
        merged.contains(r#"<variable name="LANGDRIVER" value="DBWINRUS"/>"#),
        "{merged}"
    );
    // Containers that exist only in the minor file are added recursively.
    assert!(
        merged.contains(r#"<container name="OtherAlias">"#),
        "{merged}"
    );
    // The conflicting minor value must not leak into the output.
    assert!(!merged.contains(r#"value="C:\Minor""#), "{merged}");
    assert!(
        !merged.contains(r#"name="VERSION" value="9.9""#),
        "{merged}"
    );
}

#[test]
fn merge_honours_per_file_encodings() {
    let dir = TempDir::new("merge-encoding");
    let major = dir.file("major.xml");
    let minor = dir.file("minor.xml");
    let out = dir.file("merged.xml");

    // The major file is written in windows-1251 (with a matching declaration),
    // the minor file in UTF-8; the output is requested in windows-1251.
    let major_text = MAJOR_XML.replace("UTF-8", "windows-1251");
    let (major_bytes, _, _) = encoding_rs::WINDOWS_1251.encode(&major_text);
    std::fs::write(&major, &major_bytes).unwrap();
    std::fs::write(&minor, MINOR_XML).unwrap();

    let output = run(&[
        "merge",
        "--major",
        &major,
        "--minor",
        &minor,
        "--output",
        &out,
        "--encoding",
        "windows-1251",
    ]);
    assert!(output.status.success(), "merge failed: {output:?}");
    let bytes = std::fs::read(&out).unwrap();
    assert!(
        bytes.starts_with(b"<?xml version=\"1.0\" encoding=\"windows-1251\"?>"),
        "unexpected declaration"
    );
    let (merged, _, _) = encoding_rs::WINDOWS_1251.decode(&bytes);
    assert!(
        merged.contains(r#"<variable name="NEW" value="added"/>"#),
        "{merged}"
    );
}

#[test]
fn export_and_import_with_bde_encoding_round_trips_cyrillic() {
    let dir = TempDir::new("bde-encoding");
    let cfg = dir.file("cyrillic.cfg");
    let xml = dir.file("cyrillic.xml");
    let copy = dir.file("copy.cfg");

    std::fs::write(&cfg, sample_config_cp1251()).unwrap();

    let output = run(&[
        "export",
        "-i",
        &cfg,
        "-o",
        &xml,
        "--bde-encoding",
        "windows-1251",
    ]);
    assert!(output.status.success(), "export failed: {output:?}");
    let xml_text = std::fs::read_to_string(&xml).unwrap();
    assert!(xml_text.contains("РАЗДЕЛ"), "{xml_text}");
    assert!(xml_text.contains(r#"value="Данные""#), "{xml_text}");

    let output = run(&[
        "import",
        "-i",
        &xml,
        "-o",
        &copy,
        "--bde-encoding",
        "windows-1251",
    ]);
    assert!(output.status.success(), "import failed: {output:?}");
    assert_eq!(std::fs::read(&cfg).unwrap(), std::fs::read(&copy).unwrap());
}

#[test]
fn raw_bde_encoding_round_trips_high_byte_values_end_to_end() {
    let dir = TempDir::new("high-bytes");
    let cfg = dir.file("high.cfg");
    let xml = dir.file("high.xml");
    let copy = dir.file("copy.cfg");

    // A value storing the bytes 0x80..=0xFF (avoiding 0x00, the separator).
    let high: Vec<u8> = (0x80u8..=0xFF).collect();
    let mut data = container("IDAPI");
    data.extend(variable_bytes(b"PACKED", &high));
    data.extend(end_of_block());
    std::fs::write(&cfg, &data).unwrap();

    let output = run(&["export", "-i", &cfg, "-o", &xml]);
    assert!(output.status.success(), "export failed: {output:?}");

    let output = run(&["import", "-i", &xml, "-o", &copy]);
    assert!(output.status.success(), "import failed: {output:?}");
    assert_eq!(std::fs::read(&cfg).unwrap(), std::fs::read(&copy).unwrap());
}

/// An XML document with Cyrillic text whose declaration claims windows-1251
/// (and is actually encoded that way).
fn cyrillic_xml_declared_cp1251() -> (Vec<u8>, Vec<u8>) {
    let text = r#"<?xml version="1.0" encoding="windows-1251"?>
<bde-config version="1">
  <container name="IDAPI">
    <variable name="ПУТЬ" value="Данные"/>
  </container>
</bde-config>
"#;
    let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(text);

    // The exact binary file those contents should import into.
    let encode = |t: &str| -> Vec<u8> {
        let (b, _, _) = encoding_rs::WINDOWS_1251.encode(t);
        b.into_owned()
    };
    let mut binary = container("IDAPI");
    binary.extend(variable_bytes(&encode("ПУТЬ"), &encode("Данные")));
    binary.extend(end_of_block());
    (bytes.into_owned(), binary)
}

#[test]
fn import_detects_encoding_from_the_xml_declaration() {
    let dir = TempDir::new("declaration");
    let xml = dir.file("doc.xml");
    let cfg = dir.file("out.cfg");

    let (xml_bytes, expected) = cyrillic_xml_declared_cp1251();
    std::fs::write(&xml, &xml_bytes).unwrap();

    // No `-e` flag: the encoding must be discovered from the declaration. The
    // BDE text encoding is independent and must be given explicitly.
    let output = run(&[
        "import",
        "-i",
        &xml,
        "-o",
        &cfg,
        "--bde-encoding",
        "windows-1251",
    ]);
    assert!(output.status.success(), "import failed: {output:?}");
    assert_eq!(std::fs::read(&cfg).unwrap(), expected);
}

#[test]
fn import_detects_encoding_from_a_utf8_bom() {
    let dir = TempDir::new("bom");
    let xml = dir.file("doc.xml");
    let cfg = dir.file("out.cfg");

    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(
        b"<bde-config><container name=\"ROOT\"><variable name=\"k\" value=\"v\"/></container></bde-config>",
    );
    std::fs::write(&xml, &bytes).unwrap();

    let output = run(&["import", "-i", &xml, "-o", &cfg]);
    assert!(output.status.success(), "import failed: {output:?}");

    // Round-trip back to XML to confirm the parsed contents.
    let round = dir.file("round.xml");
    assert!(run(&["export", "-i", &cfg, "-o", &round]).status.success());
    let round_text = std::fs::read_to_string(&round).unwrap();
    assert!(
        round_text.contains(r#"<variable name="k" value="v"/>"#),
        "{round_text}"
    );
}

#[test]
fn import_honours_an_explicit_encoding_flag() {
    let dir = TempDir::new("forced");
    let xml = dir.file("doc.xml");
    let cfg = dir.file("out.cfg");

    // The declaration claims UTF-8 while the bytes are actually windows-1251.
    let text = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><bde-config><container name=\"R\"><variable name=\"П\" value=\"Д\"/></container></bde-config>";
    let (bytes, _, _) = encoding_rs::WINDOWS_1251.encode(text);
    std::fs::write(&xml, &bytes).unwrap();

    let output = run(&[
        "import",
        "-i",
        &xml,
        "-o",
        &cfg,
        "-e",
        "windows-1251",
        "--bde-encoding",
        "windows-1251",
    ]);
    assert!(output.status.success(), "import failed: {output:?}");
}

#[test]
fn merge_detects_per_file_encodings_from_declarations() {
    let dir = TempDir::new("merge-declared");
    let major = dir.file("major.xml");
    let minor = dir.file("minor.xml");
    let out = dir.file("merged.xml");

    // Major: cp866-encoded with a matching declaration; Minor: plain UTF-8.
    let major_text = r#"<?xml version="1.0" encoding="cp866"?>
<bde-config version="1">
  <container name="IDAPI">
    <variable name="МАЖОР" value="главный"/>
  </container>
</bde-config>
"#;
    let (major_bytes, _, _) = encoding_rs::IBM866.encode(major_text);
    std::fs::write(&major, &major_bytes).unwrap();
    std::fs::write(
        &minor,
        "<bde-config><container name=\"IDAPI\"><variable name=\"MINOR_ONLY\" value=\"x\"/></container></bde-config>",
    )
    .unwrap();

    let output = run(&["merge", "-M", &major, "-m", &minor, "-o", &out]);
    assert!(output.status.success(), "merge failed: {output:?}");
    let merged = std::fs::read_to_string(&out).unwrap();
    assert!(merged.contains("МАЖОР"), "{merged}");
    assert!(merged.contains("MINOR_ONLY"), "{merged}");
}

#[test]
fn export_reports_a_missing_input_file() {
    let dir = TempDir::new("missing-input");
    let missing = dir.file("does-not-exist.cfg");
    let out = dir.file("out.xml");

    let output = run(&["export", "-i", &missing, "-o", &out]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to read"), "{stderr}");
    assert!(
        !std::path::Path::new(&out).exists(),
        "no output must be written"
    );
}

#[test]
fn export_reports_unparseable_input() {
    let dir = TempDir::new("garbage");
    let cfg = dir.file("garbage.cfg");
    let out = dir.file("out.xml");
    std::fs::write(&cfg, b"\x99\x88\x77").unwrap();

    let output = run(&["export", "-i", &cfg, "-o", &out]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid BDE configuration file"),
        "{stderr}"
    );
}

#[test]
fn export_rejects_control_characters_that_xml_cannot_hold() {
    let dir = TempDir::new("control");
    let cfg = dir.file("control.cfg");
    let out = dir.file("out.xml");

    // A raw byte 0x01 maps to U+0001, which XML 1.0 cannot represent.
    let mut data = container("IDAPI");
    data.extend(variable_bytes(b"BAD", &[0x01]));
    data.extend(end_of_block());
    std::fs::write(&cfg, &data).unwrap();

    let output = run(&["export", "-i", &cfg, "-o", &out]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cannot be represented in XML"), "{stderr}");
    assert!(!std::path::Path::new(&out).exists());
}

#[test]
fn import_rejects_invalid_xml() {
    let dir = TempDir::new("bad-xml");
    let xml = dir.file("bad.xml");
    let out = dir.file("out.cfg");
    std::fs::write(&xml, "<bde-config><container").unwrap();

    let output = run(&["import", "-i", &xml, "-o", &out]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid XML"), "{stderr}");
    assert!(!std::path::Path::new(&out).exists());
}

#[cfg(not(windows))]
#[test]
fn import_without_an_output_path_fails_on_non_windows() {
    let dir = TempDir::new("no-output");
    let xml = dir.file("doc.xml");
    std::fs::write(&xml, "<bde-config><container name=\"R\"/></bde-config>").unwrap();

    let output = run(&["import", "-i", &xml]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--output <PATH>"), "{stderr}");
}

#[cfg(not(windows))]
#[test]
fn set_default_is_rejected_on_non_windows() {
    let dir = TempDir::new("set-default");
    let xml = dir.file("doc.xml");
    let cfg = dir.file("out.cfg");
    std::fs::write(&xml, "<bde-config><container name=\"R\"/></bde-config>").unwrap();

    let output = run(&["import", "-i", &xml, "-o", &cfg, "--set-default"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("only supported on Windows"), "{stderr}");
    assert!(
        !std::path::Path::new(&cfg).exists(),
        "nothing may be written when --set-default fails"
    );
}

#[test]
fn export_rejects_an_unknown_xml_encoding() {
    let dir = TempDir::new("unknown-encoding");
    let cfg = dir.file("config.cfg");
    let out = dir.file("out.xml");
    std::fs::write(&cfg, sample_config()).unwrap();

    let output = run(&["export", "-i", &cfg, "-o", &out, "-e", "nope"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown XML encoding"), "{stderr}");
    assert!(!std::path::Path::new(&out).exists());
}

#[test]
fn export_rejects_an_unknown_bde_encoding() {
    let dir = TempDir::new("unknown-bde");
    let cfg = dir.file("config.cfg");
    let out = dir.file("out.xml");
    std::fs::write(&cfg, sample_config()).unwrap();

    let output = run(&["export", "-i", &cfg, "-o", &out, "--bde-encoding", "nope"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown BDE encoding"), "{stderr}");
    assert!(!std::path::Path::new(&out).exists());
}

#[test]
fn merge_rejects_an_unknown_input_encoding() {
    let dir = TempDir::new("merge-unknown");
    let major = dir.file("major.xml");
    let minor = dir.file("minor.xml");
    let out = dir.file("merged.xml");
    std::fs::write(&major, MAJOR_XML).unwrap();
    std::fs::write(&minor, MINOR_XML).unwrap();

    let output = run(&[
        "merge",
        "-M",
        &major,
        "-m",
        &minor,
        "-o",
        &out,
        "--major-encoding",
        "nope",
    ]);
    assert!(!output.status.success());
    assert!(!std::path::Path::new(&out).exists());
}

#[test]
fn rejects_unknown_flags_with_usage_error() {
    let dir = TempDir::new("usage");
    let output = run(&["export", "--output", "x.xml", "--frobnicate"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unexpected argument"), "{stderr}");
    let _ = dir;
}

#[test]
fn rejects_missing_required_arguments_with_usage_error() {
    let output = run(&["merge", "-M", "a.xml"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("required"), "{stderr}");
}

#[test]
fn prints_version_and_help() {
    let dir = TempDir::new("help");
    let output = run(&["--version"]);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("bdexmlconf"));

    let output = run(&["--help"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for command in ["export", "import", "merge"] {
        assert!(stdout.contains(command), "help must mention {command}");
    }
    let _ = dir;
}

#[test]
fn export_to_a_missing_directory_fails_cleanly() {
    let dir = TempDir::new("missing-dir");
    let cfg = dir.file("config.cfg");
    let out = dir.0.join("no-such-dir").join("out.xml");
    std::fs::write(&cfg, sample_config()).unwrap();

    let output = run(&["export", "-i", &cfg, "-o", out.to_str().unwrap()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to write"), "{stderr}");
    assert!(!dir.0.join("no-such-dir").exists());
}

#[test]
fn merge_output_overwrites_an_existing_file() {
    let dir = TempDir::new("merge-overwrite");
    let major = dir.file("major.xml");
    let minor = dir.file("minor.xml");
    let out = dir.file("merged.xml");
    std::fs::write(&major, MAJOR_XML).unwrap();
    std::fs::write(&minor, MINOR_XML).unwrap();

    // Pre-existing output: merge must overwrite it atomically without a
    // --rewrite flag (that flag exists for import only).
    std::fs::write(&out, "stale contents").unwrap();

    let output = run(&["merge", "-M", &major, "-m", &minor, "-o", &out]);
    assert!(output.status.success(), "merge failed: {output:?}");
    let merged = std::fs::read_to_string(&out).unwrap();
    assert!(merged.contains("<bde-config"), "{merged}");
    assert!(!std::path::Path::new(&format!("{out}.bak")).exists());
}

#[test]
fn import_requires_the_input_xml_to_have_a_container() {
    let dir = TempDir::new("no-container");
    let xml = dir.file("doc.xml");
    let out = dir.file("out.cfg");
    std::fs::write(&xml, "<bde-config version=\"1\"/>").unwrap();

    let output = run(&["import", "-i", &xml, "-o", &out]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("top-level <container>"), "{stderr}");
    assert!(!std::path::Path::new(&out).exists());
}

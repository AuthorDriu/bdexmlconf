//! Command-line interface: the `export`, `import` and `merge` subcommands.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use encoding_rs::Encoding;

use crate::atomic;
use crate::bdeconf::Config;
use crate::bdeconf_reader;
use crate::bdeconf_writer;
use crate::encoding;
use crate::merge;
use crate::paths;
use crate::xmlconf;

/// Convert Borland BDE configuration files (IDAPI.CFG / IDAPI32.CFG) to and
/// from XML.
#[derive(Debug, Parser)]
#[command(name = "bdexmlconf", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Available operations.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Export a BDE configuration file to XML.
    Export(ExportArgs),
    /// Import an XML file into a BDE configuration file.
    Import(ImportArgs),
    /// Merge a minor XML configuration into a major one.
    Merge(MergeArgs),
}

/// Arguments of the `export` subcommand.
#[derive(Debug, Args)]
pub struct ExportArgs {
    /// Path to the BDE configuration file to read.
    ///
    /// When omitted, the path from the Windows registry is used, falling back
    /// to the standard installation location.
    #[arg(short, long, value_name = "PATH")]
    pub input: Option<PathBuf>,

    /// Path of the XML file to write.
    #[arg(short, long, value_name = "PATH")]
    pub output: PathBuf,

    /// Character encoding of the XML file.
    #[arg(short, long, value_name = "ENCODING", default_value = "UTF-8")]
    pub encoding: String,

    /// Character encoding of the text inside the binary BDE file.
    #[arg(long, value_name = "ENCODING", default_value = "raw")]
    pub bde_encoding: String,
}

/// Arguments of the `import` subcommand.
#[derive(Debug, Args)]
pub struct ImportArgs {
    /// Path to the XML file to read.
    #[arg(short, long, value_name = "PATH")]
    pub input: PathBuf,

    /// Path of the BDE configuration file to write.
    ///
    /// When omitted, the path from the Windows registry is used, falling back
    /// to the standard installation location.
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Character encoding of the input XML file.
    ///
    /// When omitted, the encoding is detected from a byte-order mark or the XML
    /// declaration, defaulting to UTF-8.
    #[arg(short, long, value_name = "ENCODING")]
    pub encoding: Option<String>,

    /// Character encoding of the text inside the binary BDE file.
    #[arg(long, value_name = "ENCODING", default_value = "raw")]
    pub bde_encoding: String,

    /// Make the newly written file the default BDE configuration file in the
    /// Windows registry after a successful import.
    #[arg(long)]
    pub set_default: bool,

    /// Overwrite an existing BDE configuration file.
    ///
    /// Without this flag the program aborts if the target file already exists,
    /// leaving it untouched.
    #[arg(long)]
    pub rewrite: bool,
}

/// Arguments of the `merge` subcommand.
#[derive(Debug, Args)]
pub struct MergeArgs {
    /// Path to the major (base) XML file.
    #[arg(short = 'M', long, value_name = "PATH")]
    pub major: PathBuf,

    /// Path to the minor XML file.
    ///
    /// Entries that are present only here are added to the result; on a
    /// conflict the value from the major file is kept.
    #[arg(short = 'm', long, value_name = "PATH")]
    pub minor: PathBuf,

    /// Path of the merged XML file to write.
    #[arg(short, long, value_name = "PATH")]
    pub output: PathBuf,

    /// Character encoding of the major XML file.
    ///
    /// When omitted, the encoding is detected from a byte-order mark or the XML
    /// declaration, defaulting to UTF-8.
    #[arg(long, value_name = "ENCODING")]
    pub major_encoding: Option<String>,

    /// Character encoding of the minor XML file.
    ///
    /// When omitted, the encoding is detected from a byte-order mark or the XML
    /// declaration, defaulting to UTF-8.
    #[arg(long, value_name = "ENCODING")]
    pub minor_encoding: Option<String>,

    /// Character encoding of the output XML file.
    #[arg(short, long, value_name = "ENCODING", default_value = "UTF-8")]
    pub encoding: String,
}

/// Parses the command line and runs the requested operation.
pub fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Export(args) => export(args),
        Command::Import(args) => import(args),
        Command::Merge(args) => merge(args),
    }
}

fn export(args: ExportArgs) -> Result<()> {
    let codec = encoding::parse_bde_codec(&args.bde_encoding)?;
    let xml_encoding = encoding::parse_xml_encoding(&args.encoding)?;

    let input = paths::resolve_input_path(args.input.as_deref())?;
    let bytes = std::fs::read(&input).with_context(|| {
        format!(
            "failed to read BDE configuration file '{}'",
            input.display()
        )
    })?;
    let config = bdeconf_reader::parse(&bytes, &codec).with_context(|| {
        format!(
            "failed to parse '{}' as a BDE configuration file",
            input.display()
        )
    })?;

    let xml = xmlconf::to_xml_string(&config, xml_encoding.name())
        .with_context(|| format!("failed to convert '{}' to XML", input.display()))?;
    let output = encoding::encode_xml(&xml, xml_encoding);

    atomic::write_atomic(&args.output, &output)
        .with_context(|| format!("failed to write XML file '{}'", args.output.display()))?;
    Ok(())
}

fn import(args: ImportArgs) -> Result<()> {
    let codec = encoding::parse_bde_codec(&args.bde_encoding)?;
    let xml_encoding = match &args.encoding {
        Some(name) => Some(encoding::parse_xml_encoding(name)?),
        None => None,
    };

    let config = read_xml_config(&args.input, xml_encoding)?;

    let output = paths::resolve_output_path(args.output.as_deref())?;
    if args.set_default && !paths::set_default_supported() {
        bail!(
            "--set-default is only supported on Windows: the default BDE configuration file \
             is located through the Windows registry"
        );
    }
    if std::fs::symlink_metadata(&output).is_ok() && !args.rewrite {
        bail!(
            "'{}' already exists; use --rewrite to overwrite it",
            output.display()
        );
    }
    if args.rewrite && std::fs::symlink_metadata(&output).is_ok() {
        eprintln!(
            "warning: overwriting existing BDE configuration file '{}'",
            output.display()
        );
    }

    let bytes = bdeconf_writer::serialize(&config, &codec)?;

    // Back up the existing file (if any) so that it can be restored should the
    // write itself or the subsequent registry update fail.  The file itself is
    // replaced atomically, so it is never observed half-written.
    let guard = atomic::BackupGuard::create(&output)?;
    atomic::replace(&output, &bytes).with_context(|| {
        format!(
            "failed to write BDE configuration file '{}'",
            output.display()
        )
    })?;

    if args.set_default {
        paths::set_registry_config_path(&output)
            .with_context(|| "failed to make the new file the default in the registry")?;
    }

    // Everything succeeded: remove the backup.
    guard.commit();
    Ok(())
}

fn merge(args: MergeArgs) -> Result<()> {
    let output_encoding = encoding::parse_xml_encoding(&args.encoding)?;
    let major_encoding = match &args.major_encoding {
        Some(name) => Some(encoding::parse_xml_encoding(name)?),
        None => None,
    };
    let minor_encoding = match &args.minor_encoding {
        Some(name) => Some(encoding::parse_xml_encoding(name)?),
        None => None,
    };

    let major = read_xml_config(&args.major, major_encoding)?;
    let minor = read_xml_config(&args.minor, minor_encoding)?;
    let merged = merge::merge(&major, &minor);

    let xml = xmlconf::to_xml_string(&merged, output_encoding.name())
        .with_context(|| "failed to convert the merged configuration to XML")?;
    let bytes = encoding::encode_xml(&xml, output_encoding);

    atomic::write_atomic(&args.output, &bytes)
        .with_context(|| format!("failed to write XML file '{}'", args.output.display()))?;
    Ok(())
}

/// Reads an XML document and parses it into a [`Config`].
///
/// `forced` selects the input encoding explicitly; when it is `None` the
/// encoding is detected from a byte-order mark or the XML declaration.
fn read_xml_config(path: &Path, forced: Option<&'static Encoding>) -> Result<Config> {
    let raw = std::fs::read(path)
        .with_context(|| format!("failed to read XML file '{}'", path.display()))?;
    let xml = encoding::decode_xml(&raw, forced)
        .with_context(|| format!("failed to decode XML file '{}'", path.display()))?;
    xmlconf::from_xml_string(&xml)
        .with_context(|| format!("failed to parse XML file '{}'", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("bdexmlconf").chain(args.iter().copied()))
    }

    fn export_args(cli: &Cli) -> &ExportArgs {
        match &cli.command {
            Command::Export(args) => args,
            other => panic!("expected Export, got {other:?}"),
        }
    }

    fn import_args(cli: &Cli) -> &ImportArgs {
        match &cli.command {
            Command::Import(args) => args,
            other => panic!("expected Import, got {other:?}"),
        }
    }

    fn merge_args(cli: &Cli) -> &MergeArgs {
        match &cli.command {
            Command::Merge(args) => args,
            other => panic!("expected Merge, got {other:?}"),
        }
    }

    #[test]
    fn export_parses_all_arguments() {
        let cli = parse(&[
            "export",
            "--input",
            "in.cfg",
            "--output",
            "out.xml",
            "--encoding",
            "windows-1251",
            "--bde-encoding",
            "cp866",
        ])
        .unwrap();
        let args = export_args(&cli);
        assert_eq!(args.input.as_deref(), Some(Path::new("in.cfg")));
        assert_eq!(args.output, PathBuf::from("out.xml"));
        assert_eq!(args.encoding, "windows-1251");
        assert_eq!(args.bde_encoding, "cp866");
    }

    #[test]
    fn export_defaults_are_utf8_and_raw() {
        let cli = parse(&["export", "-o", "out.xml"]).unwrap();
        let args = export_args(&cli);
        assert_eq!(args.encoding, "UTF-8");
        assert_eq!(args.bde_encoding, "raw");
        assert!(args.input.is_none());
    }

    #[test]
    fn export_accepts_short_flags() {
        let cli = parse(&["export", "-i", "in.cfg", "-o", "out.xml", "-e", "866"]).unwrap();
        let args = export_args(&cli);
        assert_eq!(args.input.as_deref(), Some(Path::new("in.cfg")));
        assert_eq!(args.encoding, "866");
    }

    #[test]
    fn export_requires_an_output_path() {
        let error = parse(&["export"]).unwrap_err();
        assert!(
            error.to_string().contains("required"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn import_parses_all_arguments() {
        let cli = parse(&[
            "import",
            "-i",
            "in.xml",
            "-o",
            "out.cfg",
            "-e",
            "cp1251",
            "--bde-encoding",
            "windows-1251",
            "--set-default",
            "--rewrite",
        ])
        .unwrap();
        let args = import_args(&cli);
        assert_eq!(args.input, PathBuf::from("in.xml"));
        assert_eq!(args.output.as_deref(), Some(Path::new("out.cfg")));
        assert_eq!(args.encoding.as_deref(), Some("cp1251"));
        assert_eq!(args.bde_encoding, "windows-1251");
        assert!(args.set_default);
        assert!(args.rewrite);
    }

    #[test]
    fn import_defaults() {
        let cli = parse(&["import", "--input", "in.xml", "--output", "out.cfg"]).unwrap();
        let args = import_args(&cli);
        assert_eq!(args.bde_encoding, "raw");
        assert_eq!(args.encoding, None);
        assert!(!args.set_default);
        assert!(!args.rewrite);
    }

    #[test]
    fn import_requires_an_input_path() {
        let error = parse(&["import"]).unwrap_err();
        assert!(
            error.to_string().contains("required"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn import_output_is_optional() {
        // Without --output the path falls back to the registry or default
        // location at runtime.
        let cli = parse(&["import", "-i", "in.xml"]).unwrap();
        assert!(import_args(&cli).output.is_none());
    }

    #[test]
    fn merge_parses_short_and_long_forms() {
        let cli = parse(&["merge", "-M", "maj.xml", "-m", "min.xml", "-o", "out.xml"]).unwrap();
        let args = merge_args(&cli);
        assert_eq!(args.major, PathBuf::from("maj.xml"));
        assert_eq!(args.minor, PathBuf::from("min.xml"));
        assert_eq!(args.output, PathBuf::from("out.xml"));

        let cli = parse(&[
            "merge",
            "--major",
            "maj.xml",
            "--minor",
            "min.xml",
            "--output",
            "out.xml",
            "--major-encoding",
            "cp866",
            "--minor-encoding",
            "windows-1251",
        ])
        .unwrap();
        let args = merge_args(&cli);
        assert_eq!(args.major_encoding.as_deref(), Some("cp866"));
        assert_eq!(args.minor_encoding.as_deref(), Some("windows-1251"));
    }

    #[test]
    fn merge_defaults() {
        let cli = parse(&["merge", "-M", "a.xml", "-m", "b.xml", "-o", "c.xml"]).unwrap();
        let args = merge_args(&cli);
        assert_eq!(args.encoding, "UTF-8");
        assert!(args.major_encoding.is_none());
        assert!(args.minor_encoding.is_none());
    }

    #[test]
    fn merge_requires_major_minor_and_output() {
        for args in [
            vec!["merge"],
            vec!["merge", "-M", "a.xml", "-m", "b.xml"],
            vec!["merge", "-M", "a.xml", "-o", "c.xml"],
            vec!["merge", "-m", "b.xml", "-o", "c.xml"],
        ] {
            let error = parse(&args).unwrap_err();
            assert!(
                error.to_string().contains("required"),
                "args {args:?}: unexpected error: {error}"
            );
        }
    }

    #[test]
    fn rejects_an_unknown_subcommand() {
        let error = parse(&["frobnicate"]).unwrap_err();
        assert!(
            error.to_string().contains("unrecognized subcommand"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_an_unknown_flag() {
        let error = parse(&["export", "-o", "x.xml", "--nope"]).unwrap_err();
        assert!(
            error.to_string().contains("unexpected argument"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_a_missing_flag_value() {
        let error = parse(&["export", "--output"]).unwrap_err();
        assert!(
            error.to_string().contains("required"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_extra_positional_arguments() {
        let error = parse(&["export", "-o", "x.xml", "stray"]).unwrap_err();
        assert!(
            error.to_string().contains("unexpected argument"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn merge_does_not_accept_a_shared_short_output_flag_in_short_position() {
        // `-o` is the output flag for merge; make sure `--output` cannot be
        // mistaken for the major file.
        let cli = parse(&["merge", "-M", "a.xml", "-m", "b.xml", "-o", "c.xml"]).unwrap();
        assert_eq!(merge_args(&cli).output, PathBuf::from("c.xml"));
    }

    #[test]
    fn version_and_help_are_valid_invocations() {
        let error = parse(&["--version"]).unwrap_err();
        assert!(error.kind() == clap::error::ErrorKind::DisplayVersion);

        let error = parse(&["--help"]).unwrap_err();
        assert!(error.kind() == clap::error::ErrorKind::DisplayHelp);

        let error = parse(&["export", "--help"]).unwrap_err();
        assert!(error.kind() == clap::error::ErrorKind::DisplayHelp);
    }
}

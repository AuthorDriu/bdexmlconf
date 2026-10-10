# bdexmlconf

A command-line tool that converts Borland Database Engine (BDE) configuration
files — `IDAPI.CFG` / `IDAPI32.CFG` — to and from XML.

The binary BDE format is opaque and hard to inspect or edit by hand.
`bdexmlconf` renders it as a readable XML tree, lets you edit and compare it
with ordinary text tools, and writes it back without losing data.

*Russian version: [README_ru.md](README_ru.md).*

## Features

- **`export`** — read a binary BDE configuration file and write it as XML.
- **`import`** — turn an XML document back into a binary BDE configuration file.
- **`merge`** — combine two XML configurations: a *minor* file is merged into a
  *major* one, with the major file winning on conflicts.
- **Lossless by default.** The default `raw` byte mapping guarantees that
  `export` followed by `import` reproduces the file byte for byte, including
  text in legacy code pages.
- **Explicit encodings.** Independent control over the XML file encoding
  (`--encoding`) and the text encoding stored inside the binary file
  (`--bde-encoding`). Legacy code pages are accepted by name (`windows-1251`,
  `cp866`) or number (`1251`, `866`).
- **Encoding auto-detection.** On `import` and `merge` the XML encoding is read
  from a byte-order mark or the XML declaration when not given explicitly.
- **Safe writes.** Output files are written atomically (write to a temporary
  file, then rename) and an existing file is backed up and restored if anything
  fails. `import` refuses to overwrite an existing file unless `--rewrite` is
  passed.
- **Windows integration.** When no path is given, the active configuration file
  is located through the registry (`CONFIGFILE01` under
  `SOFTWARE\Borland\Database Engine`), falling back to the standard install
  locations. `import --set-default` records the new file in the registry.

## Requirements

- A Rust toolchain (the project uses the 2024 edition, so Rust 1.85 or newer).
  The code is regularly built with recent stable toolchains.

## Building

```sh
# Debug build; the binary is placed at target/debug/bdexmlconf
cargo build

# Optimized build; the binary is placed at target/release/bdexmlconf
cargo build --release
```

Run the tool without building a binary first:

```sh
cargo run -- export --help
```

Install it into `~/.cargo/bin`:

```sh
cargo install --path .
```

### Running the tests

```sh
cargo test
```

This runs the unit tests inside each module plus the end-to-end tests in
`tests/integration.rs`, which drive the compiled binary.

### Dependencies

| Crate         | Purpose                                             |
| ------------- | --------------------------------------------------- |
| `clap`        | Command-line parsing                                |
| `anyhow`      | Error reporting                                     |
| `encoding_rs` | XML and legacy code-page (de)coding                 |
| `quick-xml`   | Streaming XML parser                                |
| `winreg`      | Windows registry access (Windows builds only)       |

## Usage

```
bdexmlconf <COMMAND>
```

Available commands: `export`, `import`, `merge`.

### `export`

Reads a BDE configuration file and writes an XML document.

```sh
bdexmlconf export -i IDAPI32.CFG -o idapi.xml
```

| Option                   | Default | Description                                                        |
| ------------------------ | ------- | ------------------------------------------------------------------ |
| `-i, --input <PATH>`     | —       | BDE file to read. When omitted, the path is resolved from the OS.  |
| `-o, --output <PATH>`    | —       | XML file to write. **Required.**                                   |
| `-e, --encoding <ENC>`   | `UTF-8` | Character encoding of the XML file.                                |
| `--bde-encoding <ENC>`   | `raw`   | Character encoding of the text inside the binary BDE file.         |

### `import`

Reads an XML document and writes a BDE configuration file.

```sh
bdexmlconf import -i idapi.xml -o IDAPI32.CFG
```

| Option                   | Default | Description                                                          |
| ------------------------ | ------- | -------------------------------------------------------------------- |
| `-i, --input <PATH>`     | —       | XML file to read. **Required.**                                      |
| `-o, --output <PATH>`    | —       | BDE file to write. When omitted, the path is resolved from the OS.   |
| `-e, --encoding <ENC>`   | auto    | Encoding of the input XML. Detected from BOM/declaration, else UTF-8.|
| `--bde-encoding <ENC>`   | `raw`   | Character encoding of the text inside the binary BDE file.           |
| `--set-default`          | off     | Make the written file the default in the Windows registry.           |
| `--rewrite`              | off     | Overwrite an existing BDE file. Without it, an existing target aborts. |

`--set-default` is only meaningful on Windows; on other platforms it is
rejected with an error.

### `merge`

Merges a minor XML configuration into a major one.

```sh
bdexmlconf merge -M base.xml -m overrides.xml -o merged.xml
```

| Option                        | Default | Description                                                          |
| ----------------------------- | ------- | -------------------------------------------------------------------- |
| `-M, --major <PATH>`          | —       | Major (base) XML file. **Required.**                                 |
| `-m, --minor <PATH>`          | —       | Minor XML file. **Required.**                                        |
| `-o, --output <PATH>`         | —       | Merged XML file to write. **Required.**                              |
| `--major-encoding <ENC>`      | auto    | Encoding of the major file (detected if omitted).                    |
| `--minor-encoding <ENC>`      | auto    | Encoding of the minor file (detected if omitted).                    |
| `-e, --encoding <ENC>`        | `UTF-8` | Encoding of the output XML file.                                     |

Merging rules:

- Variables and containers are matched **by name** at every nesting level.
- On a conflict the value from the **major** file is kept.
- Entries that exist only in the minor file are added to the result.
- Containers present in both files are merged recursively.
- The major file's entry order is preserved; added minor entries are appended.

### Exit codes

| Code | Meaning                                                            |
| ---- | ------------------------------------------------------------------ |
| `0`  | Success.                                                           |
| `1`  | A runtime error (I/O, parse, encoding, …); details go to stderr.   |
| `2`  | Command-line usage error (unknown flag, missing argument, …).      |

## XML format

The XML document mirrors the tree stored in the BDE file:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<bde-config version="1">
  <container name="IDAPI">
    <variable name="VERSION" value="5.0"/>
    <container name="ALIASES">
      <container name="MyAlias">
        <variable name="PATH" value="C:\Data"/>
        <variable name="DRIVER" value="PARADOX"/>
      </container>
    </container>
  </container>
</bde-config>
```

- The root element is `<bde-config>` with a `version` attribute.
- `<container name="…">` groups variables and nested containers.
- `<variable name="…" value="…"/>` is a single name/value pair.
- The order of entries is significant and is preserved exactly.

See [`examples/idapi.xml`](examples/idapi.xml) for a complete example.

## Encodings

Two unrelated encodings are involved:

- **XML encoding** (`-e/--encoding`) — how the XML document itself is stored.
  Names are resolved with the WHATWG label set; bare code-page numbers are also
  accepted (`1251` → `windows-1251`, `866` → `cp866`).
- **BDE encoding** (`--bde-encoding`) — how text is stored inside the binary
  BDE file. Use a specific code page (for example `windows-1251` or `cp866`)
  when you know the file uses it.

The default `--bde-encoding raw` is a lossless ISO-8859-1 mapping where every
byte maps to the same Unicode code point. It makes round-trips exact for any
byte value, but cannot represent characters above `U+00FF` — use a real code
page in that case. Selecting a code page that cannot represent some character
is reported as an error rather than silently dropping data.

## Platform notes

**Windows.** With `-i`/`-o` omitted, the tool looks up the active configuration
file in the registry under
`HKEY_LOCAL_MACHINE` / `HKEY_CURRENT_USER\SOFTWARE\Borland\Database Engine`
(value `CONFIGFILE01`), then falls back to the usual install locations under
`Common Files\Borland Shared\BDE`. `import --set-default` writes the new path
back to the registry.

**Linux / macOS.** Registry access is unavailable, so input and output paths
must be given explicitly; `--set-default` is rejected.

## How the binary format works

The BDE file is a stream of tagged, NUL-delimited records:

```text
record start      : 0x03 0x00
container record   : 0x00 <name> 0x01 0x00
variable record    : 0x01 <name> 0x04 <value>
end of block       : 0x02 0x00
```

Every string is stored as `0x00 <bytes…> 0x00`. A container is opened by its
record and closed by an end-of-block marker; the outermost container is closed
implicitly at end of file (an explicit marker is tolerated too). The reader
accepts files both with and without the trailing markers, and reports errors
with byte offsets.

## Project layout

```
src/
  main.rs            entry point
  cli.rs             command-line interface (export / import / merge)
  bdeconf.rs         in-memory configuration tree
  bdeconf_reader.rs  binary BDE parser
  bdeconf_writer.rs  binary BDE serializer
  xmlconf.rs         Config <-> XML conversion
  merge.rs           merge logic
  encoding.rs        encoding parsing and detection
  paths.rs           locating/filing the config (registry, standard paths)
  atomic.rs          atomic writes with backup/restore
tests/integration.rs end-to-end tests of the binary
examples/idapi.xml   sample configuration
```

## License

Released under the MIT License. See [LICENSE](LICENSE).

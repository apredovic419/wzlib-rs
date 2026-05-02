---
name: wzltool
description: Inspect and edit MapleStory WZ/IMG files using the wzltool CLI. Use this skill whenever the user wants to view WZ file contents, extract images/sounds, modify property values, add/remove nodes, or rebuild WZ files. Trigger on keywords like "WZ", "IMG", "MapleStory data", "wz file", "extract sprite", "game data", or any reference to WZ/IMG paths.
---

# WZ/IMG File Operations

You have access to `wzltool`, a CLI for reading and writing MapleStory WZ and IMG files. Use it via the Bash tool.

## File Types

| Extension | Description |
|-----------|-------------|
| `.wz` (PKG1) | Standard WZ archive — contains a directory tree of `.img` entries |
| `.wz` (hotfix) | Headerless Data.wz — a single bare WzImage (first byte `0x73`) |
| `.img` | Standalone image file — a property tree |

`wzltool` auto-detects the file kind. You do not need to specify it.

## Encryption Versions

Pass `--version` (`-V`) before the subcommand. Default is `auto` (auto-detects by trying each variant and picking the one with the highest printable-ASCII rate in directory/image names).

| Version | Regions |
|---------|---------|
| `auto` | Default — probe gms/ems/bms and pick the best fit |
| `bms` | BMS, Classic, KMS, TMS, CMS (zero IV) |
| `gms` | GMS |
| `ems` | EMS, MSEA |

## Path Convention

- **WZ archives**: paths can include subdirectories. The image segment is identified by the rightmost component ending in `.img`. Property path follows.
  - `Mob/0100100.img/info/maxHP` → image `Mob/0100100.img`, property `info/maxHP`
  - `UI/Sub/Deep.img/x` → image `UI/Sub/Deep.img`, property `x`
- **Standalone .img**: paths are property-relative, e.g. `info/maxHP`
- **Directory listing**: `ls <wz> <dir-path>` (no `.img` suffix) lists immediate subdirs + images at that prefix.
- Separator is `/`.

## Commands Reference

### Read operations (no file modification)

```bash
# File info — kind, version, image count
wzltool info <file>
wzltool --json info <file>

# List contents — images in a .wz, or properties in an image/node
wzltool ls <file> [path]
wzltool --json ls <file> [path]

# Property tree — recursive view with types and values
wzltool tree <file> [path] [--depth N]
wzltool --json tree <file> [path] [--depth N]

# Get one or more values — type + value + children names
wzltool get <file> <path> [<path2> ...]
wzltool --json get <file> <path>           # single → object (backwards compat)
wzltool --json get <file> <p1> <p2> <p3>  # multiple → array
```

### Write operations (modify and save)

```bash
# Set an existing node's value
wzltool set <file> <path> <value> [-o output] [-t type_hint]

# Add a new node (parent must be a container: SubProperty, Canvas, etc.)
wzltool add <file> <path> <value> [-o output] [-t type_hint]

# Remove a node
wzltool rm <file> <path> [-o output]
```

- `-o` / `--output`: write to a different file (default: overwrite input)
- `-t` / `--type`: force a WZ type — `short`, `int`, `long`, `float`, `double`, `string`, `uol`, `lua`
- Values are auto-detected: integers → Int, decimals → Float, otherwise → String
- For large integers (> i32), use `-t long`
- **Canvas nodes**: pass a PNG/image file path as the value — the node's pixel data is replaced (requires Pillow). Always re-encodes as BGRA8888.

```bash
wzltool set Mob.wz "0100100.img/stand/0" new_sprite.png -o Mob_modified.wz
```

For `patch`, `"op":"set"` on a Canvas node works the same way — `"value"` is treated as a file path:

```json
{"op": "set", "path": "0100100.img/stand/0", "value": "new_sprite.png"}
```

### Batch operations — `patch` (parse once, save once)

Use `patch` when making multiple changes to the same file. It opens the file once and saves once, which is significantly faster than chaining individual `set`/`add`/`rm` calls.

```bash
wzltool patch <file> --ops '<json_array>' [-o output]
wzltool --json patch <file> --ops '<json_array>'
```

Each element of the JSON array is an operation object:

| Field | Required | Values |
|-------|----------|--------|
| `op` | yes | `"get"`, `"set"`, `"add"`, `"rm"` |
| `path` | yes | node path (same convention as other commands) |
| `value` | for set/add | any scalar |
| `type` | no | type hint: `"int"`, `"long"`, `"float"`, `"string"`, etc. |

Example — read two values and apply two writes in one call:

```bash
wzltool --json patch Mob.wz --ops '[
  {"op":"get",  "path":"0100100.img/info/maxHP"},
  {"op":"get",  "path":"0100100.img/info/level"},
  {"op":"set",  "path":"0100100.img/info/maxHP",  "value":99999},
  {"op":"add",  "path":"0100100.img/info/custom", "value":1, "type":"int"},
  {"op":"rm",   "path":"0100100.img/info/exp"}
]' -o Mob_patched.wz
```

Output (JSON mode) is an array of result objects, one per operation, each with `"op"` and `"path"`. Errors include an `"error"` field; successful writes include `"saved_to"`. The file is saved only if at least one write operation succeeded.

### Extract operations

```bash
# Extract Canvas as PNG (requires Pillow) or raw RGBA
wzltool extract <file> <path> -o output.png

# Extract Sound as raw audio bytes
wzltool extract <file> <path> -o output.mp3
```

### XML export / import

```bash
# Export entire image to WZ XML (server mode — metadata only, no binary data)
wzltool xml <file>                          # prints to stdout
wzltool xml <file> -o output.xml            # writes to file
wzltool xml Character.wz "00002000.img" -o 00002000.img.xml

# Export a subtree
wzltool xml Character.wz "00002000.img/walk1" -o walk1.xml

# Client mode — includes base64-encoded binary data (Canvas pixels, Sound, Lua, etc.)
wzltool xml <file> --mode client -o output.xml

# Import XML back to IMG
wzltool xml-import output.xml -o rebuilt.img
```

- `--mode server` (default): metadata only — no binary data. Suitable for server tools.
- `--mode client`: binary data base64-encoded in `basedata` attributes. Enables round-trip.
- The XML format matches the WzLib/HaRepacker convention used by server emulators.

### Bulk export — `export`

Walk a `.wz` and dump every image to a mirrored directory tree. Useful when you want to edit many images at once or diff/version-control the contents.

```bash
# XML export (default, client mode — binary data preserved)
wzltool export <file.wz> -o <out_dir>

# Raw IMG binary export (zero-loss, no XML parsing overhead)
wzltool export <file.wz> -o <out_dir> --format img

# Server-mode XML (smaller, metadata only — Canvas/Sound data dropped)
wzltool export <file.wz> -o <out_dir> --format xml --mode server
```

Output mirrors the WZ folder structure:
```
out_dir/
  UI/UIWindow.img.xml      (xml format)
  Mob/0100100.img          (img format)
```

Defaults: `--format xml --mode client`. Progress is shown on stderr (single-line `\r`-overwrite when interactive).

### Build a WZ from a directory tree — `build`

Inverse of `export`: pack a directory of `.img` and/or `.img.xml` files into a complete WZ archive.

```bash
wzltool build <src_dir> -o <file.wz> [-e gms|ems|bms] [--patch-version N] [--64bit]
```

- Walks `src_dir` recursively. Files with `.img.xml` are parsed as WZ XML; files with `.img` are treated as already-serialized WZ image binaries (read verbatim).
- **Mixed formats are allowed** — most images can stay as `.img` (fast) and only edited ones need to be `.img.xml`.
- **Same-name conflict** (both `X.img` and `X.img.xml` present): `.xml` wins, a warning is logged to stderr.
- Defaults: `-e gms`, `--patch-version 83`, 32-bit. Specify these explicitly to match the source file's region/version.

Round-trip example (export → edit → rebuild):

```bash
wzltool export Custom.wz -o /tmp/dump
# Edit /tmp/dump/UI/UIWindow.img.xml ...
wzltool build /tmp/dump -o Custom.modified.wz -e gms --patch-version 83
```

Note: the rebuilt file is **structurally equivalent** to the original but **not byte-identical** (offsets and string-dedup order are recomputed).

### JSON mode

Add `--json` before the subcommand for machine-readable output. All commands support it.

```bash
wzltool --json tree Base.wz "StandardPDD.img" --depth 2
```

## Property Types

| Type | Value accessor | Description |
|------|---------------|-------------|
| Short | integer | 16-bit signed int |
| Int | integer | 32-bit signed int |
| Long | integer | 64-bit signed int |
| Float | decimal | 32-bit float |
| Double | decimal | 64-bit float |
| String | string | UTF-8 text |
| UOL | string | Path reference (symlink-like) |
| Null | null | No value |
| SubProperty | (container) | Named child nodes |
| Canvas | (binary) | Image data — use `extract` |
| Sound | (binary) | Audio data — use `extract` |
| Vector | (pair) | x, y coordinates |
| Convex | (container) | Array of vectors |
| Lua | (binary) | Lua bytecode blob |
| RawData | (container) | Raw binary with child properties |
| Video | (binary) | Video data |

Container types (SubProperty, Canvas, Convex, RawData, Video) can have children.
Only containers accept `add` / `rm` operations.

## Typical Workflows

### Inspect a WZ file structure

```bash
wzltool info Character.wz
wzltool ls Character.wz
wzltool ls Character.wz "00002000.img"
wzltool tree Character.wz "00002000.img" --depth 3
```

### Read specific values

```bash
# Single value
wzltool get Mob.wz "0100100.img/info/maxHP"

# Multiple values in one call
wzltool --json get Mob.wz "0100100.img/info/maxHP" "0100100.img/info/level"
```

### Modify a value and save

```bash
# Save to a new file (safe — preserves original)
wzltool set Mob.wz "0100100.img/info/maxHP" 50000 -o Mob_modified.wz

# Or overwrite in place
wzltool set Mob.wz "0100100.img/info/maxHP" 50000
```

### Apply multiple changes at once (preferred for LLM tool calls)

```bash
# Parse once, apply all changes, save once — much faster than chaining set/add/rm
wzltool --json patch Mob.wz --ops '[
  {"op":"set", "path":"0100100.img/info/maxHP", "value":99999},
  {"op":"set", "path":"0100100.img/info/level", "value":10},
  {"op":"add", "path":"0100100.img/info/newProp", "value":"hello", "type":"string"}
]' -o Mob_modified.wz
```

### Add a new property

```bash
wzltool add data.img "info/customFlag" 1 -t int -o data_modified.img
```

### Bulk edit via export + build

When making large or sweeping edits, the dump-edit-rebuild flow is often simpler than chaining `set` / `patch`:

```bash
# 1. Dump everything to XML
wzltool export Custom.wz -o /tmp/dump

# 2. Edit the XMLs in /tmp/dump (any text editor / scripted xmllint / sed)

# 3. Repack
wzltool build /tmp/dump -o Custom.modified.wz -e gms --patch-version 83
```

For partial edits, only the files you change need to be `.img.xml`; leave the rest as `.img` (export with `--format img` first to get them).

### Extract a sprite image

```bash
wzltool extract Character.wz "00002000.img/0/0" -o sprite.png
```

### Extract background music

```bash
wzltool extract Sound.wz "BgmGL.img/Amoria" -o amoria.mp3
```

## Important Notes

- Always use `--json` when you need to parse the output programmatically
- Use `-o` to write to a new file when the user hasn't explicitly asked to overwrite
- For `.wz` archives, `save` re-serializes all images — this can be slow for large files
- Canvas extraction to PNG requires Pillow (`pip install Pillow`); without it, raw RGBA is saved
- The default encryption version is `auto` — it probes gms/ems/bms and picks the best fit; pass `-V <version>` only if auto-detection produces wrong results
- `WzFile.detect_version(path)` (Python API) reads the file and returns the auto-detected variant name as a string

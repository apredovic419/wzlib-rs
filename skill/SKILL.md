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

Pass `--version` (`-V`) before the subcommand. Default is `bms`.

| Version | Regions |
|---------|---------|
| `bms` | BMS, Classic, KMS, TMS, CMS (zero IV — works for most files) |
| `gms` | GMS |
| `ems` | EMS, MSEA |

## Path Convention

- **WZ archives**: paths start with the image name, e.g. `Mob/0100100.img/info/maxHP`
- **Standalone .img**: paths are property-relative, e.g. `info/maxHP`
- Separator is `/`

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

# Get a single value — type + value + children names
wzltool get <file> <path>
wzltool --json get <file> <path>
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

### Read a specific value

```bash
wzltool get Mob.wz "0100100.img/info/maxHP"
```

### Modify a value and save

```bash
# Save to a new file (safe — preserves original)
wzltool set Mob.wz "0100100.img/info/maxHP" 50000 -o Mob_modified.wz

# Or overwrite in place
wzltool set Mob.wz "0100100.img/info/maxHP" 50000
```

### Add a new property

```bash
wzltool add data.img "info/customFlag" 1 -t int -o data_modified.img
```

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
- The default encryption version `bms` (zero IV) works for most files; try `gms` or `ems` if parsing fails

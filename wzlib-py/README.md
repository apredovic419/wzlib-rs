# wzlib — Python bindings for wzlib-rs

Python bindings for the MapleStory WZ file parser and editor, powered by [wzlib-rs](https://github.com/user/wzlib-rs) via PyO3.

## Installation

```bash
cd wzlib-py
pip install -e .
```

This installs:
- The `wzlib` Python package (WzFile, WzImage, WzNode)
- The `wzltool` CLI for command-line WZ/IMG file operations

## Quick Start

### Python API

```python
from wzlib import WzFile, WzImage

# Open a WZ archive
wz = WzFile.open("Mob.wz", version="gms")
print(wz.list_images())          # ['0100100.img', 'Mob/8800000.img', ...]

img = wz.image("0100100.img")
node = img.get("info/maxHP")
print(node.as_int())              # 150

node.set(9999)
wz.save("Mob_patched.wz")

# Open a standalone img / hotfix Data.wz
img = WzImage.open("Data.wz", version="bms")
for name in img.children():
    node = img.get(name)
    print(f"{name}: {node.node_type()}")
```

### CLI (wzltool)

```bash
wzltool info Mob.wz
wzltool ls Mob.wz
wzltool tree Mob.wz "0100100.img" --depth 2
wzltool get Mob.wz "0100100.img/info/maxHP"
wzltool set Mob.wz "0100100.img/info/maxHP" 9999 -o Mob_patched.wz
wzltool extract Character.wz "00002000.img/stand1/0" -o sprite.png
wzltool --json ls Mob.wz   # JSON output for scripting / LLM usage
```

## API Overview

### Classes

| Class | Description |
|-------|-------------|
| `WzFile` | A parsed WZ archive (PKG1 format). Opens `.wz` files, lists images, lazy-loads and caches image data. |
| `WzImage` | A single image property tree. Can be obtained from `WzFile.image()` or opened standalone. |
| `WzNode` | A handle to a node in the property tree. Supports read, write, add, remove, canvas decode, sound extraction. |

### Encryption Versions

| Version string | Regions |
|---------------|---------|
| `"bms"` / `"classic"` | BMS, Classic, KMS, TMS, CMS (zero IV — default, works for most files) |
| `"gms"` | GMS |
| `"ems"` / `"msea"` | EMS, MSEA |

### Property Types

Nodes have one of 16 types. Scalar types can be read/written directly:

| Type | Read with | Python type |
|------|-----------|-------------|
| Short, Int, Long | `node.as_int()` | `int` |
| Float, Double | `node.as_float()` | `float` |
| String, UOL | `node.as_str()` | `str` |
| Null | — | `None` |
| Canvas | `node.decode_canvas()` | `(bytes, int, int)` — RGBA, width, height |
| Sound | `node.extract_sound()` | `bytes` — raw audio |

Container types (SubProperty, Canvas, Convex, RawData, Video) have children accessible via `node.children()` and `node.get(name)`.

### Editing

```python
# Set existing values
node.set(42)            # int → Int/Short/Long (preserves original type)
node.set(3.14)          # float → Float/Double
node.set("hello")       # str → String/UOL

# Add new child nodes (parent must be a container)
node.add("hp", 100)                       # auto-detect type → Int
node.add_typed("bigVal", "long", 10**12)  # explicit Long
img.add("rootProp", "value")              # add at image root

# Remove
node.remove("obsolete")  # returns True/False

# Replace canvas image
node.replace_canvas(rgba_bytes, width, height)  # always encodes as BGRA8888

# Save
img.save("output.img")  # standalone image
wz.save("output.wz")    # full WZ archive
```

## Type Stubs

Full type stubs are provided in `wzlib/__init__.pyi` for IDE autocompletion and type checking. The package is PEP 561 compliant (`py.typed` marker included).

## Testing

```bash
cd wzlib-py
pytest tests/
```

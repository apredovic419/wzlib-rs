"""wzltool — CLI for inspecting and editing MapleStory WZ/IMG files.

Designed for both human and LLM usage. Use --json for machine-readable output.
"""

import argparse
import json
import os
import sys
from pathlib import Path

from wzlib import WzFile, WzImage, WzNode


# ── Helpers ──────────────────────────────────────────────────────────────

def _detect_file_kind(path: str) -> str:
    """Detect whether a file is a .wz archive or a standalone .img."""
    ext = Path(path).suffix.lower()
    if ext == ".wz":
        with open(path, "rb") as f:
            magic = f.read(4)
        if magic == b"PKG1":
            return "wz"
        return "img"  # hotfix Data.wz — no PKG1 header
    return "img"  # .img or anything else treated as standalone image


def _open(path: str, version: str, patch_version=None):
    """Open a file, returning either (WzFile, None) or (None, WzImage)."""
    kind = _detect_file_kind(path)
    if kind == "wz":
        wz = WzFile.open(path, version=version, patch_version=patch_version)
        return wz, None
    else:
        img = WzImage.open(path, version=version)
        return None, img


def _resolve_path(wz, img, node_path: str):
    """Given a node path, return the WzNode.

    For .wz files the path is "ImageName.img/prop/path".
    For .img files the path is "prop/path".
    """
    if wz is not None:
        # Split into image name and property path
        parts = node_path.split("/", 1) if "/" in node_path else [node_path, ""]
        image_name = parts[0]
        prop_path = parts[1] if len(parts) > 1 else ""
        img_obj = wz.image(image_name)
        if not prop_path:
            return img_obj, None  # return the image itself
        node = img_obj.get(prop_path)
        return img_obj, node
    else:
        if not node_path:
            return img, None
        node = img.get(node_path)
        return img, node


def _parse_value(value_str: str, type_hint: str = None):
    """Parse a string value into the appropriate Python type."""
    if type_hint:
        hint = type_hint.lower()
        if hint in ("short", "int", "long"):
            return int(value_str)
        elif hint in ("float", "double"):
            return float(value_str)
        elif hint in ("string", "str", "uol"):
            return value_str
        elif hint in ("lua", "bytes"):
            # Expect hex string or file path
            if os.path.isfile(value_str):
                with open(value_str, "rb") as f:
                    return f.read()
            return bytes.fromhex(value_str)

    # Auto-detect type
    try:
        return int(value_str)
    except ValueError:
        pass
    try:
        return float(value_str)
    except ValueError:
        pass
    return value_str


def _load_image_rgba(path: str):
    """Load an image file and return (rgba_bytes, width, height).

    Requires Pillow. Raises ImportError with a hint if it's not installed.
    """
    try:
        from PIL import Image
    except ImportError:
        raise ImportError("Pillow is required for Canvas operations: pip install Pillow")
    img = Image.open(path).convert("RGBA")
    width, height = img.size
    return bytes(img.tobytes()), width, height


def _set_canvas(node, value_str: str) -> None:
    """Replace canvas pixel data. value_str must be a path to an image file."""
    if not os.path.isfile(value_str):
        raise FileNotFoundError(f"Image file not found: {value_str}")
    rgba, width, height = _load_image_rgba(value_str)
    node.replace_canvas(rgba, width, height)


# ── Commands ─────────────────────────────────────────────────────────────

def cmd_info(args):
    """Show file information."""
    kind = _detect_file_kind(args.file)
    result = {"file": args.file, "kind": kind}

    if kind == "wz":
        wz = WzFile.open(args.file, version=args.version)
        images = wz.list_images()
        result.update({
            "version": wz.version(),
            "is_64bit": wz.is_64bit(),
            "image_count": len(images),
        })
    else:
        img = WzImage.open(args.file, version=args.version)
        children = img.children()
        result.update({
            "root_nodes": len(children),
        })

    if args.json:
        print(json.dumps(result, indent=2))
    else:
        print(f"File: {args.file}")
        print(f"Kind: {kind}")
        if kind == "wz":
            print(f"Patch version: {result['version']}")
            print(f"64-bit format: {result['is_64bit']}")
            print(f"Images: {result['image_count']}")
        else:
            print(f"Root nodes: {result['root_nodes']}")


def cmd_ls(args):
    """List contents."""
    wz, img = _open(args.file, args.version)

    if wz is not None:
        if args.path:
            # List properties inside an image
            parts = args.path.split("/", 1)
            image_name = parts[0]
            prop_path = parts[1] if len(parts) > 1 else ""
            img_obj = wz.image(image_name)

            if prop_path:
                node = img_obj.get(prop_path)
                if node is None:
                    print(f"Error: path not found: {args.path}", file=sys.stderr)
                    return 1
                children = node.children()
                items = []
                for name in children:
                    child = node.get(name)
                    items.append({"name": name, "type": child.node_type() if child else "?"})
            else:
                children = img_obj.children()
                items = []
                for name in children:
                    n = img_obj.get(name)
                    items.append({"name": name, "type": n.node_type() if n else "?"})
        else:
            # List images in the WZ file
            images = wz.list_images()
            items = [{"name": img_name, "type": "Image"} for img_name in images]
    else:
        if args.path:
            node = img.get(args.path)
            if node is None:
                print(f"Error: path not found: {args.path}", file=sys.stderr)
                return 1
            children = node.children()
            items = []
            for name in children:
                child = node.get(name)
                items.append({"name": name, "type": child.node_type() if child else "?"})
        else:
            children = img.children()
            items = []
            for name in children:
                n = img.get(name)
                items.append({"name": name, "type": n.node_type() if n else "?"})

    if args.json:
        print(json.dumps(items, indent=2))
    else:
        for item in items:
            print(f"  {item['type']:15s} {item['name']}")


def cmd_tree(args):
    """Show property tree."""
    wz, img = _open(args.file, args.version)

    if wz is not None:
        if not args.path:
            # Show top-level directory tree (image names only)
            images = wz.list_images()
            if args.json:
                print(json.dumps([{"name": n, "type": "Image"} for n in images], indent=2))
            else:
                for name in images:
                    print(f"  {name}")
            return

        parts = args.path.split("/", 1)
        image_name = parts[0]
        prop_path = parts[1] if len(parts) > 1 else ""
        img_obj = wz.image(image_name)

        if prop_path:
            node = img_obj.get(prop_path)
            if node is None:
                print(f"Error: path not found: {args.path}", file=sys.stderr)
                return 1
            _print_tree(node, args)
        else:
            _print_tree(img_obj, args)
    else:
        if args.path:
            node = img.get(args.path)
            if node is None:
                print(f"Error: path not found: {args.path}", file=sys.stderr)
                return 1
            _print_tree(node, args)
        else:
            _print_tree(img, args)


def _print_tree(obj, args):
    """Print a WzNode or WzImage as tree or JSON using Rust-side traversal."""
    if args.json:
        print(json.dumps(json.loads(obj.to_json(args.depth)), indent=2))
    else:
        print(obj.to_tree_str(args.depth))


def _get_node_result(node, path: str) -> dict:
    """Extract a node's value/type/children into a result dict."""
    ntype = node.node_type()
    value = None
    if ntype in ("Short", "Int", "Long"):
        value = node.as_int()
    elif ntype in ("Float", "Double"):
        value = node.as_float()
    elif ntype in ("String", "UOL"):
        value = node.as_str()

    result = {"path": path, "type": ntype, "value": value}
    children = node.children()
    if children:
        result["children"] = children
    return result


def cmd_get(args):
    """Get one or more node values."""
    wz, img = _open(args.file, args.version)

    # Support multiple paths
    paths = args.path  # now a list
    results = []
    exit_code = 0

    for path in paths:
        img_obj, node = _resolve_path(wz, img, path)
        if node is None:
            if args.json:
                results.append({"path": path, "error": "not found"})
            else:
                print(f"Error: path not found: {path}", file=sys.stderr)
            exit_code = 1
            continue
        results.append(_get_node_result(node, path))

    if args.json:
        # Single path → object for backwards compat; multiple → array
        output = results[0] if len(results) == 1 else results
        print(json.dumps(output, indent=2))
    else:
        for r in results:
            if "error" in r:
                continue
            if len(paths) > 1:
                print(f"--- {r['path']} ---")
            print(f"Path: {r['path']}")
            print(f"Type: {r['type']}")
            if r["value"] is not None:
                print(f"Value: {r['value']}")
            elif r["type"] == "Null":
                print("Value: null")
            if r.get("children"):
                print(f"Children: {', '.join(r['children'])}")

    return exit_code or None


def cmd_set(args):
    """Set a node value (or replace Canvas pixel data with an image file)."""
    wz, img = _open(args.file, args.version)
    img_obj, node = _resolve_path(wz, img, args.path)

    if node is None:
        print(f"Error: path not found: {args.path}", file=sys.stderr)
        return 1

    ntype = node.node_type()
    if ntype == "Canvas":
        _set_canvas(node, args.value)
        display_value = f"<image:{args.value}>"
    else:
        value = _parse_value(args.value, args.type)
        node.set(value)
        display_value = repr(value)

    output = args.output or args.file
    if wz is not None:
        wz.save(output)
    else:
        img_obj.save(output)

    if args.json:
        result = {"status": "ok", "path": args.path, "output": output}
        if ntype == "Canvas":
            result["canvas_source"] = args.value
        else:
            result["value"] = value
        print(json.dumps(result))
    else:
        print(f"Set {args.path} = {display_value}")
        print(f"Saved to: {output}")


def cmd_add(args):
    """Add a new node."""
    wz, img = _open(args.file, args.version)

    # Split path into parent + new node name
    if "/" in args.path:
        parent_path, name = args.path.rsplit("/", 1)
    else:
        parent_path = ""
        name = args.path

    value = _parse_value(args.value, args.type)

    if wz is not None:
        # For WZ: parent_path must include image name
        parts = (parent_path or name).split("/", 1)
        image_name = parts[0]
        img_obj = wz.image(image_name)

        if parent_path:
            inner_parent = parts[1] if len(parts) > 1 else ""
            if inner_parent:
                parent_node = img_obj.get(inner_parent)
                if parent_node is None:
                    print(f"Error: parent path not found: {parent_path}", file=sys.stderr)
                    return 1
                if args.type:
                    parent_node.add_typed(name, args.type, value)
                else:
                    parent_node.add(name, value)
            else:
                if args.type:
                    img_obj.add_typed(name, args.type, value)
                else:
                    img_obj.add(name, value)
        else:
            # name is the image name itself — can't add at this level
            print("Error: cannot add at the image level in a .wz file", file=sys.stderr)
            return 1
    else:
        if parent_path:
            parent_node = img.get(parent_path)
            if parent_node is None:
                print(f"Error: parent path not found: {parent_path}", file=sys.stderr)
                return 1
            if args.type:
                parent_node.add_typed(name, args.type, value)
            else:
                parent_node.add(name, value)
        else:
            if args.type:
                img.add_typed(name, args.type, value)
            else:
                img.add(name, value)

    output = args.output or args.file
    if wz is not None:
        wz.save(output)
    else:
        img_obj = img  # for standalone img
        img_obj.save(output)

    if args.json:
        print(json.dumps({"status": "ok", "path": args.path, "value": value, "output": output}))
    else:
        print(f"Added {args.path} = {value!r}")
        print(f"Saved to: {output}")


def cmd_rm(args):
    """Remove a node."""
    wz, img = _open(args.file, args.version)

    if "/" in args.path:
        parent_path, name = args.path.rsplit("/", 1)
    else:
        parent_path = ""
        name = args.path

    removed = False

    if wz is not None:
        parts = (parent_path or name).split("/", 1)
        image_name = parts[0]
        img_obj = wz.image(image_name)

        if parent_path:
            inner_parent = parts[1] if len(parts) > 1 else ""
            if inner_parent:
                parent_node = img_obj.get(inner_parent)
                if parent_node is None:
                    print(f"Error: parent path not found: {parent_path}", file=sys.stderr)
                    return 1
                removed = parent_node.remove(name)
            else:
                removed = img_obj.remove(name)
        else:
            removed = img_obj.remove(name)
    else:
        if parent_path:
            parent_node = img.get(parent_path)
            if parent_node is None:
                print(f"Error: parent path not found: {parent_path}", file=sys.stderr)
                return 1
            removed = parent_node.remove(name)
        else:
            removed = img.remove(name)

    if not removed:
        print(f"Error: node not found: {args.path}", file=sys.stderr)
        return 1

    output = args.output or args.file
    if wz is not None:
        wz.save(output)
    else:
        img.save(output)

    if args.json:
        print(json.dumps({"status": "ok", "path": args.path, "removed": True, "output": output}))
    else:
        print(f"Removed {args.path}")
        print(f"Saved to: {output}")


def _apply_op(wz, img, op: dict) -> dict:
    """Apply a single patch operation. Returns a result dict."""
    kind = op.get("op")
    path = op.get("path", "")

    if kind == "get":
        img_obj, node = _resolve_path(wz, img, path)
        if node is None:
            return {"op": "get", "path": path, "error": "not found"}
        return {"op": "get", **_get_node_result(node, path)}

    elif kind == "set":
        img_obj, node = _resolve_path(wz, img, path)
        if node is None:
            return {"op": "set", "path": path, "error": "not found"}
        ntype = node.node_type()
        if ntype == "Canvas":
            canvas_src = str(op["value"])
            try:
                _set_canvas(node, canvas_src)
            except (FileNotFoundError, ImportError) as e:
                return {"op": "set", "path": path, "error": str(e)}
            return {"op": "set", "path": path, "canvas_source": canvas_src}
        value = _parse_value(str(op["value"]), op.get("type"))
        node.set(value)
        return {"op": "set", "path": path, "value": value}

    elif kind == "add":
        if "/" in path:
            parent_path, name = path.rsplit("/", 1)
        else:
            parent_path, name = "", path

        value = _parse_value(str(op["value"]), op.get("type"))

        if wz is not None:
            parts = (parent_path or name).split("/", 1)
            image_name = parts[0]
            img_obj = wz.image(image_name)
            inner_parent = parts[1] if len(parts) > 1 else ""
            if parent_path and inner_parent:
                parent_node = img_obj.get(inner_parent)
                if parent_node is None:
                    return {"op": "add", "path": path, "error": f"parent not found: {parent_path}"}
                if op.get("type"):
                    parent_node.add_typed(name, op["type"], value)
                else:
                    parent_node.add(name, value)
            elif parent_path:
                if op.get("type"):
                    img_obj.add_typed(name, op["type"], value)
                else:
                    img_obj.add(name, value)
            else:
                return {"op": "add", "path": path, "error": "cannot add at image level in .wz file"}
        else:
            if parent_path:
                parent_node = img.get(parent_path)
                if parent_node is None:
                    return {"op": "add", "path": path, "error": f"parent not found: {parent_path}"}
                if op.get("type"):
                    parent_node.add_typed(name, op["type"], value)
                else:
                    parent_node.add(name, value)
            else:
                if op.get("type"):
                    img.add_typed(name, op["type"], value)
                else:
                    img.add(name, value)

        return {"op": "add", "path": path, "value": value}

    elif kind == "rm":
        if "/" in path:
            parent_path, name = path.rsplit("/", 1)
        else:
            parent_path, name = "", path

        removed = False
        if wz is not None:
            parts = (parent_path or name).split("/", 1)
            image_name = parts[0]
            img_obj = wz.image(image_name)
            inner_parent = parts[1] if len(parts) > 1 else ""
            if parent_path and inner_parent:
                parent_node = img_obj.get(inner_parent)
                if parent_node is None:
                    return {"op": "rm", "path": path, "error": f"parent not found: {parent_path}"}
                removed = parent_node.remove(name)
            else:
                removed = img_obj.remove(name)
        else:
            if parent_path:
                parent_node = img.get(parent_path)
                if parent_node is None:
                    return {"op": "rm", "path": path, "error": f"parent not found: {parent_path}"}
                removed = parent_node.remove(name)
            else:
                removed = img.remove(name)

        if not removed:
            return {"op": "rm", "path": path, "error": "not found"}
        return {"op": "rm", "path": path, "removed": True}

    else:
        return {"op": kind, "path": path, "error": f"unknown op: {kind!r}"}


def cmd_patch(args):
    """Apply multiple get/set/add/rm operations in a single file open/save cycle."""
    try:
        ops = json.loads(args.ops)
    except json.JSONDecodeError as e:
        print(f"Error: invalid JSON in --ops: {e}", file=sys.stderr)
        return 1

    if not isinstance(ops, list):
        print("Error: --ops must be a JSON array", file=sys.stderr)
        return 1

    wz, img = _open(args.file, args.version)
    results = []
    has_writes = False
    has_errors = False

    for op in ops:
        result = _apply_op(wz, img, op)
        results.append(result)
        if "error" in result:
            has_errors = True
        if op.get("op") in ("set", "add", "rm") and "error" not in result:
            has_writes = True

    if has_writes:
        output = args.output or args.file
        if wz is not None:
            wz.save(output)
        else:
            img.save(output)
        for r in results:
            if r.get("op") in ("set", "add", "rm") and "error" not in r:
                r["saved_to"] = output

    if args.json:
        print(json.dumps(results, indent=2))
    else:
        for r in results:
            op = r.get("op")
            path = r.get("path", "")
            if "error" in r:
                print(f"Error [{op}] {path}: {r['error']}", file=sys.stderr)
            elif op == "get":
                ntype = r.get("type", "?")
                value = r.get("value")
                print(f"get  {path} ({ntype}) = {value!r}")
                if r.get("children"):
                    print(f"     children: {', '.join(r['children'])}")
            elif op == "set":
                print(f"set  {path} = {r['value']!r}  → {r.get('saved_to', '(pending)')}")
            elif op == "add":
                print(f"add  {path} = {r['value']!r}  → {r.get('saved_to', '(pending)')}")
            elif op == "rm":
                print(f"rm   {path}  → {r.get('saved_to', '(pending)')}")

    return 1 if has_errors else None


def cmd_xml(args):
    """Export a WZ/IMG file (or subtree) as WZ XML."""
    wz, img = _open(args.file, args.version)
    mode = args.mode  # "server" or "client"

    if wz is not None:
        if not args.path:
            print("Error: --path required when exporting from a .wz file", file=sys.stderr)
            return 1
        parts = args.path.split("/", 1)
        image_name = parts[0]
        prop_path = parts[1] if len(parts) > 1 else ""
        img_obj = wz.image(image_name)
        if prop_path:
            node = img_obj.get(prop_path)
            if node is None:
                print(f"Error: path not found: {args.path}", file=sys.stderr)
                return 1
            xml_str = node.to_xml(mode)
        else:
            xml_str = img_obj.to_xml(mode, image_name)
    else:
        if args.path:
            node = img.get(args.path)
            if node is None:
                print(f"Error: path not found: {args.path}", file=sys.stderr)
                return 1
            xml_str = node.to_xml(mode)
        else:
            # Use filename as the root element name
            img_name = os.path.basename(args.file)
            xml_str = img.to_xml(mode, img_name)

    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            f.write(xml_str)
        if args.json:
            print(json.dumps({"status": "ok", "output": args.output}))
        else:
            print(f"XML exported to: {args.output}")
    else:
        print(xml_str)


def cmd_xml_import(args):
    """Import an XML file and save as WZ IMG."""
    with open(args.xml_file, encoding="utf-8") as f:
        xml_str = f.read()

    img = WzImage.from_xml(xml_str, version=args.version)
    output = args.output or (os.path.splitext(args.xml_file)[0] + ".img")
    img.save(output)

    if args.json:
        print(json.dumps({"status": "ok", "output": output}))
    else:
        print(f"Saved to: {output}")


def cmd_extract(args):
    """Extract canvas image or sound data."""
    wz, img = _open(args.file, args.version)
    img_obj, node = _resolve_path(wz, img, args.path)

    if node is None:
        print(f"Error: path not found: {args.path}", file=sys.stderr)
        return 1

    ntype = node.node_type()
    output = args.output

    if ntype == "Canvas":
        rgba, width, height = node.decode_canvas()
        if not output:
            output = "output.png"

        # Write as raw RGBA or as PNG if PIL is available
        try:
            from PIL import Image
            pil_img = Image.frombytes("RGBA", (width, height), rgba)
            pil_img.save(output)
            fmt = "PNG"
        except ImportError:
            # Fall back to raw RGBA with dimensions header
            with open(output, "wb") as f:
                f.write(width.to_bytes(4, "little"))
                f.write(height.to_bytes(4, "little"))
                f.write(rgba)
            fmt = "raw RGBA (install Pillow for PNG)"

        if args.json:
            print(json.dumps({
                "status": "ok", "type": "Canvas",
                "width": width, "height": height,
                "format": fmt, "output": output,
            }))
        else:
            print(f"Extracted canvas: {width}x{height} ({fmt})")
            print(f"Saved to: {output}")

    elif ntype == "Sound":
        audio_data = node.extract_sound()
        if not output:
            output = "output.mp3"

        with open(output, "wb") as f:
            f.write(audio_data)

        if args.json:
            print(json.dumps({
                "status": "ok", "type": "Sound",
                "size": len(audio_data), "output": output,
            }))
        else:
            print(f"Extracted sound: {len(audio_data)} bytes")
            print(f"Saved to: {output}")

    else:
        print(f"Error: node is {ntype}, not Canvas or Sound", file=sys.stderr)
        return 1


# ── Main ─────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        prog="wzltool",
        description="CLI for inspecting and editing MapleStory WZ/IMG files",
    )
    parser.add_argument("--json", action="store_true", help="JSON output mode")
    parser.add_argument("--version", "-V", default="bms",
                        help="Encryption version: gms, ems, bms (default: bms)")

    sub = parser.add_subparsers(dest="command", required=True)

    # info
    p = sub.add_parser("info", help="Show file information")
    p.add_argument("file", help="WZ or IMG file path")

    # ls
    p = sub.add_parser("ls", help="List contents (images or properties)")
    p.add_argument("file", help="WZ or IMG file path")
    p.add_argument("path", nargs="?", default="", help="Path to list children of")

    # tree
    p = sub.add_parser("tree", help="Show property tree")
    p.add_argument("file", help="WZ or IMG file path")
    p.add_argument("path", nargs="?", default="", help="Path to show tree for")
    p.add_argument("--depth", "-d", type=int, default=-1,
                   help="Maximum depth to display (-1 = unlimited)")

    # get
    p = sub.add_parser("get", help="Get one or more node values")
    p.add_argument("file", help="WZ or IMG file path")
    p.add_argument("path", nargs="+", help="Node path(s) (e.g. 'Image.img/info/hp')")

    # set
    p = sub.add_parser("set", help="Set a node value")
    p.add_argument("file", help="WZ or IMG file path")
    p.add_argument("path", help="Node path")
    p.add_argument("value", help="New value")
    p.add_argument("--type", "-t", help="Type hint: short, int, long, float, double, string, uol")
    p.add_argument("--output", "-o", help="Output file (default: overwrite input)")

    # add
    p = sub.add_parser("add", help="Add a new node")
    p.add_argument("file", help="WZ or IMG file path")
    p.add_argument("path", help="Path for new node (parent/newName)")
    p.add_argument("value", help="Value for the new node")
    p.add_argument("--type", "-t", help="Type hint: short, int, long, float, double, string, uol, lua")
    p.add_argument("--output", "-o", help="Output file (default: overwrite input)")

    # rm
    p = sub.add_parser("rm", help="Remove a node")
    p.add_argument("file", help="WZ or IMG file path")
    p.add_argument("path", help="Path of node to remove")
    p.add_argument("--output", "-o", help="Output file (default: overwrite input)")

    # extract
    p = sub.add_parser("extract", help="Extract canvas image or sound data")
    p.add_argument("file", help="WZ or IMG file path")
    p.add_argument("path", help="Path to Canvas or Sound node")
    p.add_argument("--output", "-o", help="Output file path")

    # xml
    p = sub.add_parser("xml", help="Export WZ/IMG to WZ XML format")
    p.add_argument("file", help="WZ or IMG file path")
    p.add_argument("path", nargs="?", default="", help="Subtree path to export (optional)")
    p.add_argument("--mode", "-m", default="server", choices=["server", "client"],
                   help="Export mode: server (metadata only) or client (with base64 binary data)")
    p.add_argument("--output", "-o", help="Output XML file (default: stdout)")

    # xml-import
    p = sub.add_parser("xml-import", help="Import WZ XML and save as IMG")
    p.add_argument("xml_file", help="XML file to import")
    p.add_argument("--output", "-o", help="Output IMG file (default: <xml_file>.img)")

    # patch — batch get/set/add/rm in one file open/save cycle
    p = sub.add_parser(
        "patch",
        help="Apply multiple operations in one file open/save cycle",
        description=(
            "Apply a JSON array of operations to a WZ/IMG file, parsing once and saving once.\n"
            "Each operation is an object with 'op' (get/set/add/rm), 'path', and optionally\n"
            "'value' and 'type'. Example:\n"
            '  --ops \'[{"op":"set","path":"info/hp","value":9999},'
            '{"op":"rm","path":"info/mp"}]\''
        ),
    )
    p.add_argument("file", help="WZ or IMG file path")
    p.add_argument(
        "--ops",
        required=True,
        help='JSON array of operations: [{"op":"get|set|add|rm","path":"...","value":..., "type":"..."}]',
    )
    p.add_argument("--output", "-o", help="Output file (default: overwrite input)")

    args = parser.parse_args()

    try:
        func = {
            "info": cmd_info,
            "ls": cmd_ls,
            "tree": cmd_tree,
            "get": cmd_get,
            "set": cmd_set,
            "add": cmd_add,
            "rm": cmd_rm,
            "extract": cmd_extract,
            "xml": cmd_xml,
            "xml-import": cmd_xml_import,
            "patch": cmd_patch,
        }[args.command]
        result = func(args)
        sys.exit(result or 0)
    except Exception as e:
        if args.json:
            print(json.dumps({"error": str(e)}), file=sys.stderr)
        else:
            print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()

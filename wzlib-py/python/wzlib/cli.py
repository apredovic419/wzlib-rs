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


def _list_dir_children(wz, prefix: str):
    """List immediate children (subdirs, images) under a WZ directory prefix.

    Empty prefix means the root. Returns (subdirs, images) as sorted lists.
    """
    norm = prefix.strip("/")
    subdirs = set()
    images_here = []
    for img_path in wz.list_images():
        if norm:
            if not img_path.startswith(norm + "/"):
                continue
            rest = img_path[len(norm) + 1:]
        else:
            rest = img_path
        parts = rest.split("/", 1)
        if len(parts) == 1:
            images_here.append(parts[0])
        else:
            subdirs.add(parts[0])
    return sorted(subdirs), sorted(images_here)


def _is_image_path(path: str) -> bool:
    """A path that points at an image (or a property inside one) contains
    a segment ending in `.img`. Bare directory paths do not."""
    return any(p.endswith(".img") for p in path.split("/"))


def _split_wz_path(node_path: str):
    """Split a WZ-relative path into (image_path, prop_path).

    Image paths can include subdirectories (e.g. "UI/UIWindow.img"). The
    image segment is identified by the rightmost component ending in ".img".
    Falls back to "first slash" if no such segment exists.
    """
    parts = node_path.split("/")
    last_img = -1
    for i, p in enumerate(parts):
        if p.endswith(".img"):
            last_img = i
    if last_img >= 0:
        return "/".join(parts[:last_img + 1]), "/".join(parts[last_img + 1:])
    if "/" in node_path:
        head, _, tail = node_path.partition("/")
        return head, tail
    return node_path, ""


def _resolve_path(wz, img, node_path: str):
    """Given a node path, return the WzNode.

    For .wz files the path is "[Sub/Dirs/]ImageName.img/prop/path".
    For .img files the path is "prop/path".
    """
    if wz is not None:
        image_name, prop_path = _split_wz_path(node_path)
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
        if args.path and not _is_image_path(args.path):
            # Directory listing: subdirs + images at this prefix
            subdirs, images_here = _list_dir_children(wz, args.path)
            if not subdirs and not images_here:
                print(f"Error: directory not found or empty: {args.path}", file=sys.stderr)
                return 1
            items = ([{"name": s, "type": "Directory"} for s in subdirs]
                     + [{"name": n, "type": "Image"} for n in images_here])
            if args.json:
                print(json.dumps(items, indent=2))
            else:
                for item in items:
                    print(f"  {item['type']:15s} {item['name']}")
            return
        if args.path:
            # List properties inside an image
            image_name, prop_path = _split_wz_path(args.path)
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

        image_name, prop_path = _split_wz_path(args.path)
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
        # For WZ: parent_path must include the image name
        image_name, inner_parent = _split_wz_path(parent_path or name)
        if not parent_path:
            # name is the image name itself — can't add at this level
            print("Error: cannot add at the image level in a .wz file", file=sys.stderr)
            return 1
        img_obj = wz.image(image_name)

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
        image_name, inner_parent = _split_wz_path(parent_path or name)
        img_obj = wz.image(image_name)

        if parent_path and inner_parent:
            parent_node = img_obj.get(inner_parent)
            if parent_node is None:
                print(f"Error: parent path not found: {parent_path}", file=sys.stderr)
                return 1
            removed = parent_node.remove(name)
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
            if not parent_path:
                return {"op": "add", "path": path, "error": "cannot add at image level in .wz file"}
            image_name, inner_parent = _split_wz_path(parent_path)
            img_obj = wz.image(image_name)
            if inner_parent:
                parent_node = img_obj.get(inner_parent)
                if parent_node is None:
                    return {"op": "add", "path": path, "error": f"parent not found: {parent_path}"}
                if op.get("type"):
                    parent_node.add_typed(name, op["type"], value)
                else:
                    parent_node.add(name, value)
            else:
                if op.get("type"):
                    img_obj.add_typed(name, op["type"], value)
                else:
                    img_obj.add(name, value)
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
            image_name, inner_parent = _split_wz_path(parent_path or name)
            img_obj = wz.image(image_name)
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
        image_name, prop_path = _split_wz_path(args.path)
        img_obj = wz.image(image_name)
        if prop_path:
            node = img_obj.get(prop_path)
            if node is None:
                print(f"Error: path not found: {args.path}", file=sys.stderr)
                return 1
            xml_str = node.to_xml(mode)
        else:
            # XML root element is the image's leaf name only (e.g. "UIWindow.img"),
            # not the full WZ path "UI/UIWindow.img".
            xml_str = img_obj.to_xml(mode, image_name.split("/")[-1])
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


def _collect_build_inputs(src_dir: Path):
    """Walk src_dir and return [(wz_path, source_file, kind)] in DFS order.

    Same-name conflicts (`X.img` and `X.img.xml` both present) → prefer .xml
    and warn on stderr. The .img sibling is silently dropped.
    """
    # Group by directory, then by stripped image name.
    # wz_path uses '/' separators regardless of platform.
    inputs = []
    warned = []

    def walk(dir_path: Path, wz_prefix: str):
        # Single iterdir pass: collect images here + subdir paths to recurse into.
        candidates = {}  # stripped_name -> (path, "xml"|"img")
        subdirs = []
        for child in sorted(dir_path.iterdir()):
            if child.is_dir():
                subdirs.append(child)
                continue
            name = child.name
            if name.endswith(".img.xml"):
                key, kind = name[:-4], "xml"  # "UIWindow.img.xml" → "UIWindow.img"
            elif name.endswith(".img"):
                key, kind = name, "img"
            else:
                continue  # ignore unrelated files (.bak, README.md, etc.)
            existing = candidates.get(key)
            if existing is None:
                candidates[key] = (child, kind)
            elif existing[1] == "xml":
                warned.append((wz_prefix, key, "kept .xml, ignored .img"))
            else:
                candidates[key] = (child, kind)
                warned.append((wz_prefix, key, "replaced .img with .xml"))

        for key in sorted(candidates):
            path, kind = candidates[key]
            wz_path = f"{wz_prefix}/{key}" if wz_prefix else key
            inputs.append((wz_path, path, kind))

        for child in subdirs:
            sub_prefix = f"{wz_prefix}/{child.name}" if wz_prefix else child.name
            walk(child, sub_prefix)

    walk(src_dir, "")

    for prefix, key, msg in warned:
        loc = f"{prefix}/{key}" if prefix else key
        print(f"warning: conflict at {loc}: {msg}", file=sys.stderr)

    return inputs


def cmd_build(args):
    """Build a WZ file from a directory tree of .img / .img.xml files."""
    src_dir = Path(args.src_dir)
    if not src_dir.is_dir():
        print(f"Error: source is not a directory: {src_dir}", file=sys.stderr)
        return 1

    # Warn when both -e and --patch-version are at their defaults: a roundtrip
    # from a non-GMS / non-v83 source would silently change encryption + version.
    if args.encryption is None and args.patch_version is None:
        print("warning: using defaults --encryption gms --patch-version 83. "
              "If the source was not GMS v83, pass -e and --patch-version explicitly "
              "to preserve the original variant.", file=sys.stderr)
    encryption = args.encryption or "gms"
    patch_version = args.patch_version if args.patch_version is not None else 83

    inputs = _collect_build_inputs(src_dir)
    if not inputs:
        print(f"Error: no .img or .img.xml files found in {src_dir}", file=sys.stderr)
        return 1

    total = len(inputs)
    use_progress = not args.json and sys.stderr.isatty()
    progress_width = 0

    entries = []
    for i, (wz_path, src_path, kind) in enumerate(inputs, 1):
        if use_progress:
            line = f"[{i}/{total}] {kind:3s} {wz_path}"
            pad = max(0, progress_width - len(line))
            sys.stderr.write("\r" + line + " " * pad)
            sys.stderr.flush()
            progress_width = max(progress_width, len(line))
        elif not args.json:
            print(f"[{i}/{total}] {kind} {wz_path}", file=sys.stderr)

        if kind == "xml":
            with open(src_path, encoding="utf-8") as f:
                xml_str = f.read()
            img = WzImage.from_xml(xml_str, version=encryption)
            entries.append((wz_path, img.build()))
        else:  # img
            with open(src_path, "rb") as f:
                entries.append((wz_path, f.read()))

    if use_progress:
        sys.stderr.write("\r" + " " * progress_width + "\r")
        sys.stderr.flush()

    if not args.json:
        print(f"Assembling {total} images into {args.output}...", file=sys.stderr)

    size = WzFile.build_to_file(
        entries, args.output,
        version=encryption,
        patch_version=patch_version,
        is_64bit=args.is_64bit,
    )

    if args.json:
        print(json.dumps({
            "status": "ok",
            "output": args.output,
            "size": size,
            "images": total,
            "encryption": encryption,
            "patch_version": patch_version,
            "is_64bit": args.is_64bit,
        }))
    else:
        print(f"Built {args.output}: {size} bytes, {total} images "
              f"(v{patch_version} {encryption}{' 64-bit' if args.is_64bit else ''})",
              file=sys.stderr)


def cmd_export(args):
    """Export an entire WZ file to a directory tree of XML or IMG files.

    Output mirrors the WZ folder structure:
      out_dir/UI/UIWindow.img.xml
      out_dir/Mob/0100100.img.xml
    For --format img, files are written as raw .img binaries (no .xml suffix).
    """
    wz, _ = _open(args.file, args.version)
    if wz is None:
        print("Error: export requires a .wz file (got a standalone .img)", file=sys.stderr)
        return 1

    out_dir = Path(args.output)
    out_dir.mkdir(parents=True, exist_ok=True)

    images = wz.list_images()
    total = len(images)
    fmt = args.format
    mode = args.mode

    use_progress = not args.json and sys.stderr.isatty()
    progress_width = 0  # track longest line for \r overwrite

    for i, img_path in enumerate(images, 1):
        # Mirror "UI/UIWindow.img" → out_dir/UI/UIWindow.img(.xml)
        parts = img_path.split("/")
        rel_parts = parts[:-1]
        leaf = parts[-1]
        target_dir = out_dir.joinpath(*rel_parts) if rel_parts else out_dir
        target_dir.mkdir(parents=True, exist_ok=True)

        if fmt == "xml":
            out_path = target_dir / f"{leaf}.xml"
        else:
            out_path = target_dir / leaf

        # Progress (stderr; \r-overwrite if interactive)
        if use_progress:
            line = f"[{i}/{total}] {img_path}"
            pad = max(0, progress_width - len(line))
            sys.stderr.write("\r" + line + " " * pad)
            sys.stderr.flush()
            progress_width = max(progress_width, len(line))
        elif not args.json:
            print(f"[{i}/{total}] {img_path}", file=sys.stderr)

        if fmt == "xml":
            img = wz.image(img_path)
            xml_str = img.to_xml(mode, leaf)
            with open(out_path, "w", encoding="utf-8") as f:
                f.write(xml_str)
            # Drop parsed property tree to keep memory bounded on large WZs.
            wz.evict_image(img_path)
        else:  # img — byte-identical raw slice, no parse / re-serialize
            wz.dump_image_raw(img_path, str(out_path))

    if use_progress:
        sys.stderr.write("\r" + " " * progress_width + "\r")
        sys.stderr.flush()

    if args.json:
        print(json.dumps({
            "status": "ok",
            "format": fmt,
            "exported": total,
            "output": str(out_dir),
        }))
    else:
        print(f"Exported {total} images ({fmt}) to {out_dir}", file=sys.stderr)


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
    parser.add_argument("--version", "-V", default="auto",
                        help="Encryption version: auto, gms, ems, bms (default: auto)")

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

    # build — rebuild a WZ file from a directory tree of .img / .img.xml
    p = sub.add_parser(
        "build",
        help="Build a WZ file from a directory tree of .img / .img.xml files",
        description=(
            "Walk <src_dir> and pack every .img / .img.xml into a complete "
            "WZ archive. Mixed formats are allowed; on same-name conflict "
            "(both .img and .img.xml), .xml wins."
        ),
    )
    p.add_argument("src_dir", help="Source directory tree (mirrors WZ structure)")
    p.add_argument("--output", "-o", required=True, help="Output WZ file path")
    p.add_argument("--encryption", "-e", default=None, choices=["gms", "ems", "bms"],
                   help="Encryption variant for the output file (default: gms)")
    p.add_argument("--patch-version", type=int, default=None,
                   help="Patch version number (default: 83)")
    p.add_argument("--64bit", dest="is_64bit", action="store_true",
                   help="Produce a 64-bit (v770+) WZ file")

    # export — bulk export entire WZ to a directory tree
    p = sub.add_parser(
        "export",
        help="Export entire WZ file to a directory tree (XML or IMG)",
        description=(
            "Walk the WZ directory tree and write each image to a mirrored "
            "path under <output>/. XML mode writes <name>.img.xml files; "
            "IMG mode writes raw .img binaries."
        ),
    )
    p.add_argument("file", help="WZ file path")
    p.add_argument("--output", "-o", required=True, help="Output directory")
    p.add_argument("--format", "-f", choices=["xml", "img"], default="xml",
                   help="Export format: xml (HaRepacker-compatible) or img (raw binary). Default: xml")
    p.add_argument("--mode", "-m", default="client", choices=["server", "client"],
                   help="XML mode: server (metadata only) or client (with base64 binary). Default: client")

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
            "export": cmd_export,
            "build": cmd_build,
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

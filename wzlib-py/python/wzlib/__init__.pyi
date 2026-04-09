"""
wzlib — MapleStory WZ file parser and editor.

Typical usage::

    import wzlib

    # Open a WZ file
    wz = wzlib.WzFile.open("Mob.wz", version="gms")
    for name in wz.list_images():
        img = wz.image(name)
        node = img.get("info/hp")
        if node is not None:
            node.set(9999)
    wz.save("Mob_patched.wz")

    # Open a standalone img file
    img = wzlib.WzImage.open("Data.wz", version="bms")
    rgba, w, h = img.get("stand1/0/body").decode_canvas()
"""

from __future__ import annotations

from typing import Literal, Optional

# Valid encryption version strings (case-insensitive).
WzVersion = Literal["gms", "ems", "msea", "bms", "classic"]

# All possible node type strings returned by WzNode.node_type().
WzNodeType = Literal[
    "Null",
    "Short",
    "Int",
    "Long",
    "Float",
    "Double",
    "String",
    "SubProperty",
    "Canvas",
    "Vector",
    "Convex",
    "Sound",
    "UOL",
    "Lua",
    "RawData",
    "Video",
]

class WzNode:
    """A handle to a single node in a WzImage property tree.

    A :class:`WzNode` is obtained from :meth:`WzImage.get` or :meth:`WzNode.get`.
    It holds an internal reference to the property tree, so mutations through
    :meth:`set` / :meth:`replace_canvas` / :meth:`add` / :meth:`remove` are
    visible immediately to all other handles that share the same image.

    The node's slash-separated path from the image root is available via the
    :attr:`path` property.
    """

    @property
    def path(self) -> str:
        """Slash-joined path from the image root, e.g. ``"info/hp"``."""
        ...

    def node_type(self) -> WzNodeType:
        """Return the WZ property type of this node.

        Possible values: ``"Null"``, ``"Short"``, ``"Int"``, ``"Long"``,
        ``"Float"``, ``"Double"``, ``"String"``, ``"SubProperty"``,
        ``"Canvas"``, ``"Vector"``, ``"Convex"``, ``"Sound"``, ``"UOL"``,
        ``"Lua"``, ``"RawData"``, ``"Video"``.

        :raises KeyError: if the node no longer exists in the tree.
        """
        ...

    def as_int(self) -> Optional[int]:
        """Return the integer value for Short / Int / Long nodes, else ``None``."""
        ...

    def as_float(self) -> Optional[float]:
        """Return the float value for Float / Double nodes, else ``None``."""
        ...

    def as_str(self) -> Optional[str]:
        """Return the string value for String / UOL nodes, else ``None``."""
        ...

    def children(self) -> list[str]:
        """Return the names of direct child nodes.

        Returns an empty list for leaf nodes (Int, String, Canvas, Sound, …).
        Container nodes (SubProperty, Canvas, Convex, Video, RawData) return
        their children's names.

        :returns: List of child node names.
        :rtype: list[str]
        """
        ...

    def get(self, name: str) -> Optional[WzNode]:
        """Get a named child node.

        :param name: The exact child name (not a path — use :meth:`WzImage.get`
            for slash-separated paths).
        :returns: A :class:`WzNode` handle, or ``None`` if no such child exists.
        :rtype: WzNode or None
        """
        ...

    def set(self, value: int | float | str) -> None:
        """Set the scalar value of this node.

        The underlying WZ variant is preserved where possible:
        ``Short`` nodes stay ``Short``, ``UOL`` nodes stay ``UOL``, etc.

        :param value: New value — ``int``, ``float``, or ``str``.
        :raises ValueError: if *value* is not one of the supported scalar types,
            or if an integer value is out of range for the node's type
            (e.g. > 32767 for a ``Short`` node, or > 2147483647 for an ``Int`` node).
        :raises KeyError: if this node no longer exists in the tree.
        """
        ...

    def add(self, name: str, value: int | float | str | bytes) -> None:
        """Add or replace a named child node.

        The parent node must be a container (SubProperty, Canvas, Convex, …).
        If a child with *name* already exists it is replaced in place.

        Integers are stored as ``Int`` (i32); use :meth:`add_typed` with
        ``"long"`` for values outside the i32 range.
        Floats are stored as ``Float`` (f32); use :meth:`add_typed` with
        ``"double"`` for full precision.
        ``bytes`` are stored as a ``Lua`` blob.

        :param name: Name of the child node to add or replace.
        :param value: Value to store — ``int``, ``float``, ``str``, or ``bytes``.
        :raises ValueError: if the parent node is a leaf (Int, String, …),
            or if an integer value is out of the i32 range.
        :raises KeyError: if this node no longer exists in the tree.
        """
        ...

    def add_typed(self, name: str, type_hint: str, value: int | float | str | bytes) -> None:
        """Add or replace a named child node with an explicit WZ type.

        Use this instead of :meth:`add` when you need a specific numeric or
        string variant (e.g. ``Long``, ``Double``, ``UOL``).

        Valid *type_hint* values (case-insensitive):

        - ``"short"``  — 16-bit integer (i16); *value* must be an ``int``
        - ``"int"``    — 32-bit integer (i32); *value* must be an ``int``
        - ``"long"``   — 64-bit integer (i64); *value* must be an ``int``
        - ``"float"``  — 32-bit float (f32); *value* must be a ``float``
        - ``"double"`` — 64-bit float (f64); *value* must be a ``float``
        - ``"string"`` / ``"str"`` — string; *value* must be a ``str``
        - ``"uol"``    — UOL reference string; *value* must be a ``str``
        - ``"lua"`` / ``"bytes"`` — Lua blob; *value* must be ``bytes``

        :param name: Name of the child node to add or replace.
        :param type_hint: WZ property type string (see above).
        :param value: Value matching the requested type.
        :raises ValueError: if *type_hint* is unknown, *value* has the wrong
            Python type, or an integer is out of range for the requested type.
        :raises KeyError: if this node no longer exists in the tree.
        """
        ...

    def remove(self, name: str) -> bool:
        """Remove a named child node.

        :param name: Name of the child node to remove.
        :returns: ``True`` if a child with *name* was found and removed,
            ``False`` otherwise.
        :rtype: bool
        :raises ValueError: if the parent node is a leaf.
        :raises KeyError: if this node no longer exists in the tree.
        """
        ...

    def decode_canvas(self) -> tuple[bytes, int, int]:
        """Decode a Canvas node to raw RGBA8888 pixel data.

        :returns: A tuple ``(rgba_bytes, width, height)`` where *rgba_bytes* is
            a ``bytes`` object of length ``width * height * 4`` with pixels in
            R-G-B-A order.
        :rtype: tuple[bytes, int, int]
        :raises ValueError: if this node is not a Canvas.
        :raises RuntimeError: if decompression or pixel decoding fails.
        """
        ...

    def replace_canvas(self, rgba: bytes, width: int, height: int) -> None:
        """Replace the Canvas pixel data with new RGBA8888 bytes.

        The data is re-encoded as BGRA8888 and zlib-compressed before storage.
        DXT encoding is not supported; BGRA8888 is used regardless of the
        original format.

        :param rgba: Raw RGBA8888 bytes, length must be ``width * height * 4``.
        :param width: New canvas width in pixels.
        :param height: New canvas height in pixels.
        :raises ValueError: if this node is not a Canvas, or if *rgba* is too short.
        :raises RuntimeError: if encoding or compression fails.
        """
        ...

    def extract_sound(self) -> bytes:
        """Extract raw audio bytes from a Sound node.

        :returns: The raw audio payload (MP3/WAV/OGG depending on the WZ file).
        :rtype: bytes
        :raises ValueError: if this node is not a Sound.
        """
        ...

    def __repr__(self) -> str: ...


class WzImage:
    """A parsed WZ image property tree.

    A :class:`WzImage` owns the property tree of a single ``.img`` entry, either
    loaded lazily from a :class:`WzFile` via :meth:`WzFile.image`, or opened
    directly as a standalone hotfix ``Data.wz`` / bare img file.

    All modifications (:meth:`set`, :meth:`add`, :meth:`remove`,
    :meth:`replace_canvas`) are held in memory until :meth:`build` or
    :meth:`save` is called.
    """

    @staticmethod
    def open(path: str, version: WzVersion = "bms") -> WzImage:
        """Open a standalone img file from disk.

        This covers hotfix ``Data.wz`` files (first byte ``0x73``) and any
        raw WZ image binary.

        :param path: Path to the img file.
        :param version: Encryption variant — ``"gms"``, ``"ems"``/``"msea"``,
            or ``"bms"``/``"classic"``. Case-insensitive. Defaults to ``"bms"``.
        :raises IOError: if the file cannot be read.
        :raises RuntimeError: if parsing fails.
        """
        ...

    @staticmethod
    def from_bytes(data: bytes, version: WzVersion = "bms") -> WzImage:
        """Parse a WzImage from raw bytes already loaded in memory.

        :param data: The raw img binary.
        :param version: Encryption variant — ``"gms"``, ``"ems"``/``"msea"``,
            or ``"bms"``/``"classic"``. Case-insensitive. Defaults to ``"bms"``.
        :raises RuntimeError: if parsing fails.
        """
        ...

    def get(self, path: str) -> Optional[WzNode]:
        """Get a node by slash-separated path.

        :param path: Slash-separated path, e.g. ``"info/hp"`` or
            ``"stand1/0/body"``.
        :returns: A :class:`WzNode` handle, or ``None`` if the path does not
            exist.
        :rtype: WzNode or None
        """
        ...

    def children(self) -> list[str]:
        """Return the names of the root-level property nodes in this image.

        :returns: List of root node names.
        :rtype: list[str]
        """
        ...

    def add(self, name: str, value: int | float | str | bytes) -> None:
        """Add or replace a root-level node.

        Integers are stored as ``Int`` (i32); use :meth:`add_typed` with
        ``"long"`` for values outside the i32 range.
        Floats are stored as ``Float`` (f32); use :meth:`add_typed` with
        ``"double"`` for full precision.
        ``bytes`` are stored as a ``Lua`` blob.

        :param name: Name of the root node to add or replace.
        :param value: Value to store — ``int``, ``float``, ``str``, or ``bytes``.
        :raises ValueError: if an integer value is out of the i32 range.
        """
        ...

    def add_typed(self, name: str, type_hint: str, value: int | float | str | bytes) -> None:
        """Add or replace a root-level node with an explicit WZ type.

        Valid *type_hint* values (case-insensitive):
        ``"short"``, ``"int"``, ``"long"``, ``"float"``, ``"double"``,
        ``"string"``/``"str"``, ``"uol"``, ``"lua"``/``"bytes"``.

        :param name: Name of the root node to add or replace.
        :param type_hint: WZ property type string (see :meth:`WzNode.add_typed`).
        :param value: Value matching the requested type.
        :raises ValueError: if *type_hint* is unknown, *value* has the wrong
            Python type, or an integer is out of range.
        """
        ...

    def remove(self, name: str) -> bool:
        """Remove a root-level node by name.

        :param name: Name of the root node to remove.
        :returns: ``True`` if found and removed, ``False`` otherwise.
        :rtype: bool
        """
        ...

    def build(self) -> bytes:
        """Serialize the property tree to WZ image binary.

        The result can be saved standalone (equivalent to a hotfix ``Data.wz``
        file) or passed to external tooling.

        :returns: Serialized WZ image bytes.
        :rtype: bytes
        :raises RuntimeError: if serialization fails.
        """
        ...

    def save(self, path: str) -> None:
        """Serialize and write to a file.

        :param path: Destination file path.
        :raises IOError: if the file cannot be written.
        :raises RuntimeError: if serialization fails.
        """
        ...

    def __repr__(self) -> str: ...


class WzFile:
    """A parsed MapleStory WZ archive (PKG1 format).

    Images are loaded lazily on the first call to :meth:`image`. The parsed
    property tree is cached so subsequent calls for the same image return the
    same object (sharing the same underlying data — modifications are visible
    across all handles).

    Calling :meth:`save` or :meth:`build` re-serializes all images (including
    unmodified ones that have not been accessed yet) so the output is always
    a complete, valid WZ file.
    """

    @staticmethod
    def open(
        path: str,
        version: WzVersion = "bms",
        patch_version: Optional[int] = None,
    ) -> WzFile:
        """Open a WZ file from disk.

        :param path: Path to the ``.wz`` file.
        :param version: Encryption variant — ``"gms"``, ``"ems"``/``"msea"``,
            or ``"bms"``/``"classic"``. Case-insensitive. Defaults to ``"bms"``.
        :param patch_version: Supply the known patch version to skip brute-force
            detection (0–2000). Omit to auto-detect.
        :raises IOError: if the file cannot be read.
        :raises RuntimeError: if parsing or version detection fails.
        """
        ...

    def list_images(self) -> list[str]:
        """Return all image paths in depth-first order.

        Returns paths like ``["0100000.img", "Mob/0100100.img"]``.
        Use these strings directly with :meth:`image`.

        :returns: List of image path strings.
        :rtype: list[str]
        """
        ...

    def image(self, name: str) -> WzImage:
        """Load and return an image by path.

        The image is parsed on first access and cached; repeated calls with
        the same *name* return the same :class:`WzImage` object.

        :param name: Image path as returned by :meth:`list_images`, e.g.
            ``"0100000.img"`` or ``"Mob/0100100.img"``.
        :returns: The parsed :class:`WzImage`.
        :rtype: WzImage
        :raises KeyError: if no image with that path exists.
        :raises RuntimeError: if parsing fails.
        """
        ...

    def build(self) -> bytes:
        """Serialize the entire WZ file to bytes.

        All images — including those not yet accessed — are parsed and
        re-serialized. The result is a valid, self-contained WZ file.

        :returns: Complete serialized WZ file bytes.
        :rtype: bytes
        :raises RuntimeError: if serialization fails.
        """
        ...

    def save(self, path: str) -> None:
        """Serialize the entire WZ file and write it to disk.

        :param path: Destination file path.
        :raises IOError: if the file cannot be written.
        :raises RuntimeError: if serialization fails.
        """
        ...

    def version(self) -> int:
        """Return the detected patch version number (e.g. 83, 176, 220).

        :returns: Patch version integer.
        :rtype: int
        """
        ...

    def is_64bit(self) -> bool:
        """Return whether this file uses the 64-bit WZ format (v770+).

        :returns: ``True`` for 64-bit format, ``False`` otherwise.
        :rtype: bool
        """
        ...

    def __repr__(self) -> str: ...

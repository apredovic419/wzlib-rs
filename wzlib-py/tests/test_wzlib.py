"""Unit tests for wzlib Python bindings."""

import pytest
import tempfile
import os

from wzlib import WzFile, WzImage, WzNode


# ── Test Fixtures ─────────────────────────────────────────────────────────────


@pytest.fixture
def sample_hotfix_bytes():
    """Create a minimal valid hotfix Data.wz bytes (no PKG1 header)."""
    # Hotfix is just a bare WzImage
    data = bytearray()
    data.append(0x73)  # header marker

    # "Property" string (WZ ASCII encoding with zero key)
    name = "Property"
    data.append((-(len(name))) & 0xFF)  # negative length as unsigned byte
    mask = 0xAA
    for c in name:
        data.append(ord(c) ^ mask)
        mask = (mask + 1) & 0xFF

    data.extend((0).to_bytes(2, 'little'))  # val = 0
    data.append(0)  # property count = 0

    return bytes(data)


@pytest.fixture
def sample_image_with_properties():
    """Create WzImage bytes with various property types."""
    data = bytearray()
    data.append(0x73)  # header marker

    # "Property" string
    name = "Property"
    data.append((-(len(name))) & 0xFF)  # negative length as unsigned byte
    mask = 0xAA
    for c in name:
        data.append(ord(c) ^ mask)
        mask = (mask + 1) & 0xFF

    data.extend((0).to_bytes(2, 'little'))  # val = 0
    data.append(3)  # property count = 3 (compressed_int, fits in i8)

    # Property 1: "intVal" = 42 (Int, type 0x03, encoded as compressed_int)
    prop_name = "intVal"
    data.append(0x73)  # string_block type byte: inline string
    data.append((-(len(prop_name))) & 0xFF)
    mask = 0xAA
    for c in prop_name:
        data.append(ord(c) ^ mask)
        mask = (mask + 1) & 0xFF
    data.append(0x03)  # Int type
    data.append(42)    # compressed_int: 42 fits in i8 (single byte)

    # Property 2: "strVal" = "hello" (String, type 0x08, value via read_string_block)
    prop_name = "strVal"
    data.append(0x73)
    data.append((-(len(prop_name))) & 0xFF)
    mask = 0xAA
    for c in prop_name:
        data.append(ord(c) ^ mask)
        mask = (mask + 1) & 0xFF
    data.append(0x08)  # String type
    # String value read via read_string_block: needs type byte first
    data.append(0x73)  # string_block type byte for value
    str_val = "hello"
    data.append((-(len(str_val))) & 0xFF)
    mask = 0xAA
    for c in str_val:
        data.append(ord(c) ^ mask)
        mask = (mask + 1) & 0xFF

    # Property 3: "floatVal" = 3.14 (Float, type 0x04, indicator 0x80 + f32)
    prop_name = "floatVal"
    data.append(0x73)
    data.append((-(len(prop_name))) & 0xFF)
    mask = 0xAA
    for c in prop_name:
        data.append(ord(c) ^ mask)
        mask = (mask + 1) & 0xFF
    data.append(0x04)  # Float type
    data.append(0x80)  # indicator: non-zero → read f32
    import struct
    data.extend(struct.pack('<f', 3.14))

    return bytes(data)


# ── WzImage Tests ─────────────────────────────────────────────────────────────

class TestWzImage:
    """Tests for WzImage class."""

    def test_from_bytes_basic(self, sample_hotfix_bytes):
        """Test parsing a hotfix image from bytes."""
        img = WzImage.from_bytes(sample_hotfix_bytes, "bms")
        assert img is not None
        assert isinstance(img, WzImage)

    def test_from_bytes_default_version(self, sample_hotfix_bytes):
        """Test that default version is 'bms'."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        assert img is not None

    def test_children_empty(self, sample_hotfix_bytes):
        """Test children() on empty image."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        children = img.children()
        assert children == []

    def test_children_with_properties(self, sample_image_with_properties):
        """Test children() returns root property names."""
        img = WzImage.from_bytes(sample_image_with_properties)
        children = img.children()
        assert len(children) == 3
        assert "intVal" in children
        assert "strVal" in children
        assert "floatVal" in children

    def test_get_existing_path(self, sample_image_with_properties):
        """Test get() returns node for existing path."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        assert node is not None
        assert isinstance(node, WzNode)

    def test_get_nonexistent_path(self, sample_image_with_properties):
        """Test get() returns None for nonexistent path."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("nonexistent")
        assert node is None

    def test_get_nested_path(self):
        """Test get() with nested path (SubProperty)."""
        # Create image with nested SubProperty
        data = bytearray()
        data.append(0x73)

        name = "Property"
        data.append((-(len(name))) & 0xFF)
        mask = 0xAA
        for c in name:
            data.append(ord(c) ^ mask)
            mask = (mask + 1) & 0xFF

        data.extend((0).to_bytes(2, 'little'))
        data.append(1)  # count = 1 (compressed_int)

        # SubProperty "outer" with nested "inner" = 42
        prop_name = "outer"
        data.append(0x73)  # string_block type byte
        data.append((-(len(prop_name))) & 0xFF)
        mask = 0xAA
        for c in prop_name:
            data.append(ord(c) ^ mask)
            mask = (mask + 1) & 0xFF

        # Extended property block (type 0x09): block_size u32 + extended data
        inner_data = bytearray()
        inner_data.append(0x73)  # type byte for type string (inline)
        type_name = "Property"
        inner_data.append((-(len(type_name))) & 0xFF)
        mask = 0xAA
        for c in type_name:
            inner_data.append(ord(c) ^ mask)
            mask = (mask + 1) & 0xFF
        inner_data.extend((0).to_bytes(2, 'little'))  # _padding u16 required by SubProperty
        inner_data.append(1)  # count = 1 (compressed_int)

        # Inner property "inner" = 42 (Int, type 0x03)
        inner_name = "inner"
        inner_data.append(0x73)   # string_block type byte
        inner_data.append((-(len(inner_name))) & 0xFF)
        mask = 0xAA
        for c in inner_name:
            inner_data.append(ord(c) ^ mask)
            mask = (mask + 1) & 0xFF
        inner_data.append(0x03)  # Int type
        inner_data.append(42)    # compressed_int: single byte

        data.append(0x09)  # extended marker
        data.extend(len(inner_data).to_bytes(4, 'little'))
        data.extend(inner_data)

        img = WzImage.from_bytes(bytes(data))
        node = img.get("outer/inner")
        assert node is not None
        assert node.as_int() == 42


class TestWzNode:
    """Tests for WzNode class."""

    def test_node_type(self, sample_image_with_properties):
        """Test node_type() returns correct type name."""
        img = WzImage.from_bytes(sample_image_with_properties)

        node = img.get("intVal")
        assert node.node_type() == "Int"

        node = img.get("strVal")
        assert node.node_type() == "String"

    def test_as_int(self, sample_image_with_properties):
        """Test as_int() returns correct value."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        assert node.as_int() == 42

    def test_as_int_wrong_type(self, sample_image_with_properties):
        """Test as_int() returns None for non-int node."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("strVal")
        assert node.as_int() is None

    def test_as_float(self, sample_image_with_properties):
        """Test as_float() returns correct value."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("floatVal")
        result = node.as_float()
        assert result is not None
        assert abs(result - 3.14) < 0.01

    def test_as_str(self, sample_image_with_properties):
        """Test as_str() returns correct value."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("strVal")
        assert node.as_str() == "hello"

    def test_path_property(self, sample_image_with_properties):
        """Test path property returns correct path string."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        assert node.path == "intVal"

    def test_repr(self, sample_image_with_properties):
        """Test __repr__ returns useful string."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        repr_str = repr(node)
        assert "intVal" in repr_str
        assert "Int" in repr_str


class TestWzImageEdit:
    """Tests for WzImage editing functionality."""

    def test_add_int_property(self, sample_hotfix_bytes):
        """Test add() creates new int property."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add("newInt", 100)

        node = img.get("newInt")
        assert node is not None
        assert node.as_int() == 100

    def test_add_string_property(self, sample_hotfix_bytes):
        """Test add() creates new string property."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add("newStr", "test string")

        node = img.get("newStr")
        assert node is not None
        assert node.as_str() == "test string"

    def test_add_float_property(self, sample_hotfix_bytes):
        """Test add() creates new float property."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add("newFloat", 2.5)

        node = img.get("newFloat")
        assert node is not None
        result = node.as_float()
        assert result is not None
        assert abs(result - 2.5) < 0.01

    def test_add_bytes_property(self, sample_hotfix_bytes):
        """Test add() creates new Lua property from bytes."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add("newLua", b"\x01\x02\x03")

        node = img.get("newLua")
        assert node is not None
        assert node.node_type() == "Lua"

    def test_add_replace_existing(self, sample_hotfix_bytes):
        """Test add() replaces existing property."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add("value", 10)
        img.add("value", 20)

        node = img.get("value")
        assert node.as_int() == 20

    def test_remove_existing(self, sample_hotfix_bytes):
        """Test remove() removes existing property."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add("toRemove", 5)

        result = img.remove("toRemove")
        assert result is True

        node = img.get("toRemove")
        assert node is None

    def test_remove_nonexistent(self, sample_hotfix_bytes):
        """Test remove() returns False for nonexistent property."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        result = img.remove("nonexistent")
        assert result is False

    def test_set_int_value(self, sample_image_with_properties):
        """Test set() changes int value."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        node.set(999)

        node = img.get("intVal")
        assert node.as_int() == 999

    def test_set_int_overflow_raises_error(self, sample_image_with_properties):
        """Test set() raises ValueError for int value out of i32 range."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        with pytest.raises(ValueError, match="out of range"):
            node.set(2**31)  # one past i32 max

    def test_set_int_max_does_not_raise(self, sample_image_with_properties):
        """Test set() accepts i32 boundary values."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        node.set(2**31 - 1)
        assert node.as_int() == 2**31 - 1
        node.set(-(2**31))
        assert node.as_int() == -(2**31)

    def test_set_string_value(self, sample_image_with_properties):
        """Test set() changes string value."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("strVal")
        node.set("new string")

        node = img.get("strVal")
        assert node.as_str() == "new string"


class TestAddTyped:
    """Tests for add_typed() on WzImage and WzNode."""

    def test_add_typed_long(self, sample_hotfix_bytes):
        """Test add_typed() creates a Long node for large integers."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add_typed("bigVal", "long", 10_000_000_000)

        node = img.get("bigVal")
        assert node is not None
        assert node.node_type() == "Long"
        assert node.as_int() == 10_000_000_000

    def test_add_typed_short(self, sample_hotfix_bytes):
        """Test add_typed() creates a Short node."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add_typed("s", "short", 100)

        node = img.get("s")
        assert node.node_type() == "Short"
        assert node.as_int() == 100

    def test_add_typed_short_overflow_raises(self, sample_hotfix_bytes):
        """Test add_typed('short') raises ValueError for out-of-range value."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        with pytest.raises(ValueError, match="out of range"):
            img.add_typed("s", "short", 40000)

    def test_add_typed_double(self, sample_hotfix_bytes):
        """Test add_typed() creates a Double node."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add_typed("d", "double", 1.23456789012345)

        node = img.get("d")
        assert node.node_type() == "Double"
        assert abs(node.as_float() - 1.23456789012345) < 1e-10

    def test_add_typed_uol(self, sample_hotfix_bytes):
        """Test add_typed() creates a UOL node."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add_typed("link", "uol", "../other")

        node = img.get("link")
        assert node.node_type() == "UOL"
        assert node.as_str() == "../other"

    def test_add_typed_int_overflow_raises(self, sample_hotfix_bytes):
        """Test add_typed('int') raises ValueError for out-of-range value."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        with pytest.raises(ValueError, match="out of range"):
            img.add_typed("v", "int", 2**31)

    def test_add_typed_unknown_type_raises(self, sample_hotfix_bytes):
        """Test add_typed() raises ValueError for unknown type hint."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        with pytest.raises(ValueError, match="Unknown type hint"):
            img.add_typed("v", "uint64", 1)

    def test_image_add_int_overflow_raises(self, sample_hotfix_bytes):
        """Test WzImage.add() raises ValueError for int out of i32 range."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        with pytest.raises(ValueError, match="out of range"):
            img.add("bigVal", 2**40)


class TestWzImageBuild:
    """Tests for WzImage build/save functionality."""

    def test_build_returns_bytes(self, sample_hotfix_bytes):
        """Test build() returns bytes."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        result = img.build()
        assert isinstance(result, bytes)
        assert len(result) > 0

    def test_build_roundtrip(self, sample_image_with_properties):
        """Test build -> from_bytes roundtrip preserves data."""
        img1 = WzImage.from_bytes(sample_image_with_properties)

        # Get original values
        int_node = img1.get("intVal")
        str_node = img1.get("strVal")
        orig_int = int_node.as_int()
        orig_str = str_node.as_str()

        # Build and re-parse
        built = img1.build()
        img2 = WzImage.from_bytes(built)

        # Verify values preserved
        int_node2 = img2.get("intVal")
        str_node2 = img2.get("strVal")
        assert int_node2.as_int() == orig_int
        assert str_node2.as_str() == orig_str

    def test_save_to_file(self, sample_hotfix_bytes):
        """Test save() writes to file."""
        img = WzImage.from_bytes(sample_hotfix_bytes)

        with tempfile.NamedTemporaryFile(delete=False, suffix='.img') as f:
            temp_path = f.name

        try:
            img.save(temp_path)
            assert os.path.exists(temp_path)
            assert os.path.getsize(temp_path) > 0
        finally:
            os.unlink(temp_path)


class TestWzNodeChildren:
    """Tests for WzNode children operations."""

    def test_children_of_leaf_node(self, sample_image_with_properties):
        """Test children() returns empty list for leaf node."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        children = node.children()
        assert children == []

    def test_get_child(self, sample_image_with_properties):
        """Test get() on node returns child."""
        img = WzImage.from_bytes(sample_image_with_properties)
        # intVal is a leaf, so get() should return None
        node = img.get("intVal")
        child = node.get("nonexistent")
        assert child is None

    def test_repr_on_deleted_node(self, sample_image_with_properties):
        """Test __repr__ handles missing node gracefully."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        # Even after getting, repr should work
        repr_str = repr(node)
        assert "intVal" in repr_str


class TestVersionStrings:
    """Tests for version string handling."""

    @pytest.mark.parametrize("version", ["gms", "GMS", "Gms"])
    def test_version_gms(self, version, sample_hotfix_bytes):
        """Test GMS version is accepted."""
        img = WzImage.from_bytes(sample_hotfix_bytes, version)
        assert img is not None

    @pytest.mark.parametrize("version", ["ems", "EMS", "msea", "MSEA"])
    def test_version_ems(self, version, sample_hotfix_bytes):
        """Test EMS/MSEA version is accepted."""
        img = WzImage.from_bytes(sample_hotfix_bytes, version)
        assert img is not None

    @pytest.mark.parametrize("version", ["bms", "BMS", "classic", "Classic"])
    def test_version_bms(self, version, sample_hotfix_bytes):
        """Test BMS/Classic version is accepted."""
        img = WzImage.from_bytes(sample_hotfix_bytes, version)
        assert img is not None

    def test_invalid_version_raises_error(self, sample_hotfix_bytes):
        """Test invalid version raises ValueError."""
        with pytest.raises(ValueError, match="Unknown version"):
            WzImage.from_bytes(sample_hotfix_bytes, "invalid")


class TestWzFile:
    """Tests for WzFile class."""

    def test_open_nonexistent_raises_ioerror(self):
        """Test open() raises IOError for a non-existent file."""
        with pytest.raises(IOError):
            WzFile.open("/nonexistent/path/to/file.wz")

    def test_open_invalid_bytes_raises_runtime_error(self):
        """Test open() raises RuntimeError for invalid WZ bytes."""
        with tempfile.NamedTemporaryFile(delete=False, suffix='.wz') as f:
            f.write(b"not a wz file")
            temp_path = f.name
        try:
            with pytest.raises(RuntimeError):
                WzFile.open(temp_path)
        finally:
            os.unlink(temp_path)

    def test_open_invalid_version_raises_value_error(self):
        """Test open() raises ValueError for invalid version string."""
        with tempfile.NamedTemporaryFile(delete=False, suffix='.wz') as f:
            f.write(b"PKG1" + b"\x00" * 100)
            temp_path = f.name
        try:
            with pytest.raises(ValueError, match="Unknown version"):
                WzFile.open(temp_path, version="invalid")
        finally:
            os.unlink(temp_path)


class TestErrorHandling:
    """Tests for error handling."""

    def test_get_missing_key_error(self, sample_image_with_properties):
        """Test get() on missing path returns None (no exception)."""
        img = WzImage.from_bytes(sample_image_with_properties)
        result = img.get("does/not/exist")
        assert result is None

    def test_node_type_missing_raises_error(self, sample_image_with_properties):
        """Test node_type() on missing node raises KeyError."""
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        # This should work
        assert node.node_type() == "Int"


# ── XML export / import ───────────────────────────────────────────────────────


class TestXmlExport:
    """Tests for WzImage.to_xml() and WzImage.from_xml()."""

    def test_to_xml_server_mode(self, sample_image_with_properties):
        """Server mode XML has no basedata attributes."""
        img = WzImage.from_bytes(sample_image_with_properties)
        xml = img.to_xml(mode="server")
        assert "<?xml" in xml
        assert '<imgdir name="root">' in xml
        assert 'name="intVal"' in xml
        assert "basedata" not in xml

    def test_to_xml_client_mode_matches_server_for_scalars(self, sample_image_with_properties):
        """Client mode for scalar-only images matches server mode (no binary)."""
        img = WzImage.from_bytes(sample_image_with_properties)
        xml_server = img.to_xml(mode="server")
        xml_client = img.to_xml(mode="client")
        # Both should have the same int/string/float nodes
        assert 'name="intVal"' in xml_client
        assert 'name="strVal"' in xml_client

    def test_to_xml_default_mode_is_server(self, sample_image_with_properties):
        """Default mode should be server (no basedata)."""
        img = WzImage.from_bytes(sample_image_with_properties)
        xml = img.to_xml()
        assert "basedata" not in xml

    def test_roundtrip_server_mode(self, sample_image_with_properties):
        """Export to XML and import back — scalar values preserved."""
        img = WzImage.from_bytes(sample_image_with_properties)
        xml = img.to_xml(mode="server", name="test.img")
        img2 = WzImage.from_xml(xml)
        assert img2 is not None
        node = img2.get("intVal")
        assert node is not None
        assert node.as_int() == 42

    def test_from_xml_parses_string(self, sample_image_with_properties):
        """from_xml correctly parses string properties."""
        img = WzImage.from_bytes(sample_image_with_properties)
        xml = img.to_xml(mode="server", name="test.img")
        img2 = WzImage.from_xml(xml)
        node = img2.get("strVal")
        assert node is not None
        assert node.as_str() == "hello"

    def test_from_xml_invalid_raises(self):
        """from_xml raises RuntimeError on malformed XML."""
        with pytest.raises(RuntimeError):
            WzImage.from_xml("not xml at all <<<")

    def test_to_xml_named(self, sample_image_with_properties):
        """to_xml with name argument uses correct root name."""
        img = WzImage.from_bytes(sample_image_with_properties)
        xml = img.to_xml(name="myimg.img")
        assert 'name="myimg.img"' in xml

    def test_to_xml_escape(self, sample_hotfix_bytes):
        """XML special characters in property names/values are escaped."""
        img = WzImage.from_bytes(sample_hotfix_bytes)
        img.add("a&b", '<val">')
        xml = img.to_xml()
        assert "a&amp;b" in xml
        assert "&lt;val&quot;&gt;" in xml


# ── JSON base64 export ────────────────────────────────────────────────────────


class TestJsonBase64:
    """Tests for to_json_base64() on WzImage and WzNode."""

    def test_image_to_json_base64_returns_string(self, sample_image_with_properties):
        """to_json_base64() returns a JSON object keyed by property name."""
        import json
        img = WzImage.from_bytes(sample_image_with_properties)
        j = img.to_json_base64()
        parsed = json.loads(j)
        assert isinstance(parsed, dict)

    def test_image_to_json_base64_contains_scalars(self, sample_image_with_properties):
        """to_json_base64() includes scalar properties keyed by name."""
        import json
        img = WzImage.from_bytes(sample_image_with_properties)
        j = img.to_json_base64()
        parsed = json.loads(j)
        assert "intVal" in parsed
        assert "strVal" in parsed
        assert parsed["intVal"]["value"] == 42

    def test_node_to_json_base64(self, sample_image_with_properties):
        """WzNode.to_json_base64() returns a single-node JSON object."""
        import json
        img = WzImage.from_bytes(sample_image_with_properties)
        node = img.get("intVal")
        j = node.to_json_base64()
        parsed = json.loads(j)
        assert parsed["type"] == "Int"
        assert parsed["value"] == 42

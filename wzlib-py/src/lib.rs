//! Python bindings for wzlib-rs via PyO3.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use pyo3::exceptions::{PyIOError, PyKeyError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use wzlib_rs::crypto::aes_encryption::generate_wz_key;
use wzlib_rs::wz::directory::{WzDirectoryEntry, WzImageEntry};
use wzlib_rs::wz::properties::WzProperty;
use wzlib_rs::wz::types::{WzMapleVersion, WzPngFormat};
use wzlib_rs::{
    WzBinaryReader, WzFile, WzHeader,
    compress_png_data, decode_pixels, decompress_png_data, encode_pixels,
    parse_hotfix_data_wz, parse_wz_image, save_hotfix_data_wz,
};

// ── Helpers ──────────────────────────────────────────────────────────

fn version_to_maple(version: &str) -> PyResult<WzMapleVersion> {
    match version.to_lowercase().as_str() {
        "gms" => Ok(WzMapleVersion::Gms),
        "ems" | "msea" => Ok(WzMapleVersion::Ems),
        "bms" | "classic" => Ok(WzMapleVersion::Bms),
        other => Err(PyValueError::new_err(format!(
            "Unknown version '{}'. Use 'gms', 'ems'/'msea', or 'bms'/'classic'.",
            other
        ))),
    }
}

fn version_to_iv(version: &str) -> PyResult<[u8; 4]> {
    Ok(version_to_maple(version)?.iv())
}

fn prop_type_name(prop: &WzProperty) -> &'static str {
    match prop {
        WzProperty::Null => "Null",
        WzProperty::Short(_) => "Short",
        WzProperty::Int(_) => "Int",
        WzProperty::Long(_) => "Long",
        WzProperty::Float(_) => "Float",
        WzProperty::Double(_) => "Double",
        WzProperty::String(_) => "String",
        WzProperty::SubProperty { .. } => "SubProperty",
        WzProperty::Canvas { .. } => "Canvas",
        WzProperty::Vector { .. } => "Vector",
        WzProperty::Convex { .. } => "Convex",
        WzProperty::Sound { .. } => "Sound",
        WzProperty::Uol(_) => "UOL",
        WzProperty::Lua(_) => "Lua",
        WzProperty::RawData { .. } => "RawData",
        WzProperty::Video { .. } => "Video",
    }
}

fn prop_children(prop: &WzProperty) -> Option<&[(String, WzProperty)]> {
    match prop {
        WzProperty::SubProperty { properties } => Some(properties),
        WzProperty::Canvas { properties, .. } => Some(properties),
        WzProperty::Video { properties, .. } => Some(properties),
        WzProperty::Convex { points } => Some(points),
        WzProperty::RawData { properties, .. } => Some(properties),
        _ => None,
    }
}

fn prop_children_mut(prop: &mut WzProperty) -> Option<&mut Vec<(String, WzProperty)>> {
    match prop {
        WzProperty::SubProperty { properties } => Some(properties),
        WzProperty::Canvas { properties, .. } => Some(properties),
        WzProperty::Video { properties, .. } => Some(properties),
        WzProperty::Convex { points } => Some(points),
        WzProperty::RawData { properties, .. } => Some(properties),
        _ => None,
    }
}

fn get_prop<'a>(props: &'a [(String, WzProperty)], path: &[&str]) -> Option<&'a WzProperty> {
    let (head, rest) = path.split_first()?;
    for (name, prop) in props {
        if name == *head {
            return if rest.is_empty() {
                Some(prop)
            } else {
                get_prop(prop_children(prop)?, rest)
            };
        }
    }
    None
}

fn get_prop_mut<'a>(
    props: &'a mut Vec<(String, WzProperty)>,
    path: &[&str],
) -> Option<&'a mut WzProperty> {
    let (head, rest) = path.split_first()?;
    for (name, prop) in props.iter_mut() {
        if name == *head {
            return if rest.is_empty() {
                Some(prop)
            } else {
                get_prop_mut(prop_children_mut(prop)?, rest)
            };
        }
    }
    None
}

fn path_parts(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

fn lock_err() -> PyErr {
    PyRuntimeError::new_err("internal lock poisoned")
}

// ── JSON / tree-string helpers ───────────────────────────────────────

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// Recursively serialise a WzProperty into a JSON object matching the
// _node_to_dict format: {path, type, value?, children? | children_count?}.
fn prop_to_json(
    prop: &WzProperty,
    path_str: &str,
    max_depth: i32,
    depth: i32,
    out: &mut String,
) {
    let type_name = prop_type_name(prop);
    out.push('{');
    out.push_str("\"path\":");
    out.push_str(&json_string(path_str));
    out.push_str(",\"type\":");
    out.push_str(&json_string(type_name));

    match prop {
        WzProperty::Short(v) => { out.push_str(",\"value\":"); out.push_str(&v.to_string()); }
        WzProperty::Int(v)   => { out.push_str(",\"value\":"); out.push_str(&v.to_string()); }
        WzProperty::Long(v)  => { out.push_str(",\"value\":"); out.push_str(&v.to_string()); }
        WzProperty::Float(v) => {
            out.push_str(",\"value\":");
            // NaN/inf are not valid JSON; map them to null.
            if v.is_finite() { out.push_str(&v.to_string()); } else { out.push_str("null"); }
        }
        WzProperty::Double(v) => {
            out.push_str(",\"value\":");
            if v.is_finite() { out.push_str(&v.to_string()); } else { out.push_str("null"); }
        }
        WzProperty::String(s) | WzProperty::Uol(s) => {
            out.push_str(",\"value\":");
            out.push_str(&json_string(s));
        }
        WzProperty::Null => { out.push_str(",\"value\":null"); }
        _ => {}
    }

    if let Some(children) = prop_children(prop) {
        if !children.is_empty() {
            if max_depth < 0 || depth < max_depth {
                out.push_str(",\"children\":{");
                for (i, (name, child)) in children.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    out.push_str(&json_string(name));
                    out.push(':');
                    let child_path = if path_str.is_empty() {
                        name.clone()
                    } else {
                        format!("{}/{}", path_str, name)
                    };
                    prop_to_json(child, &child_path, max_depth, depth + 1, out);
                }
                out.push('}');
            } else {
                out.push_str(",\"children_count\":");
                out.push_str(&children.len().to_string());
            }
        }
    }

    out.push('}');
}

// Recursively format a WzProperty as a human-readable tree matching
// the _format_node_tree layout.
fn prop_to_tree(
    prop: &WzProperty,
    label: &str,
    line_prefix: &str,  // prepended before `label` on this node's own line
    child_indent: &str, // prepended before connector / content of every child line
    max_depth: i32,
    depth: i32,
    out: &mut String,
) {
    let type_name = prop_type_name(prop);
    let val = match prop {
        WzProperty::Short(v)  => format!(" = {}", v),
        WzProperty::Int(v)    => format!(" = {}", v),
        WzProperty::Long(v)   => format!(" = {}", v),
        WzProperty::Float(v)  => format!(" = {}", v),
        WzProperty::Double(v) => format!(" = {}", v),
        WzProperty::String(s) | WzProperty::Uol(s) => {
            let truncated: String = s.chars().take(60).collect();
            let display = if s.chars().count() > 60 {
                format!("{}...", truncated)
            } else {
                truncated
            };
            format!(" = \"{}\"", display)
        }
        WzProperty::Null => " = null".to_string(),
        _ => String::new(),
    };

    out.push_str(line_prefix);
    out.push_str(label);
    out.push_str(" [");
    out.push_str(type_name);
    out.push(']');
    out.push_str(&val);

    if let Some(children) = prop_children(prop) {
        if !children.is_empty() && (max_depth < 0 || depth < max_depth) {
            for (i, (name, child)) in children.iter().enumerate() {
                out.push('\n');
                let is_last = i == children.len() - 1;
                let connector   = if is_last { "└─" } else { "├─" };
                let next_indent = if is_last { "  " } else { "│ " };
                let next_line_prefix  = format!("{}{}", child_indent, connector);
                let next_child_indent = format!("{}{}", child_indent, next_indent);
                prop_to_tree(child, name, &next_line_prefix, &next_child_indent, max_depth, depth + 1, out);
            }
        } else if !children.is_empty() {
            out.push('\n');
            out.push_str(child_indent);
            out.push_str(&format!("  ... ({} children)", children.len()));
        }
    }
}

// ── Directory helpers ────────────────────────────────────────────────

fn find_image_entry<'a>(dir: &'a WzDirectoryEntry, path: &str) -> Option<&'a WzImageEntry> {
    if let Some(slash) = path.find('/') {
        let head = &path[..slash];
        let rest = &path[slash + 1..];
        for sub in &dir.subdirectories {
            if sub.name == head {
                return find_image_entry(sub, rest);
            }
        }
    } else {
        for img in &dir.images {
            if img.name == path {
                return Some(img);
            }
        }
    }
    None
}

fn collect_image_paths(dir: &WzDirectoryEntry, prefix: &str, out: &mut Vec<String>) {
    for img in &dir.images {
        out.push(if prefix.is_empty() {
            img.name.clone()
        } else {
            format!("{}/{}", prefix, img.name)
        });
    }
    for sub in &dir.subdirectories {
        let sub_prefix = if prefix.is_empty() {
            sub.name.clone()
        } else {
            format!("{}/{}", prefix, sub.name)
        };
        collect_image_paths(sub, &sub_prefix, out);
    }
}

// ── Image parsing ────────────────────────────────────────────────────

fn parse_image_at(
    raw: &[u8],
    iv: [u8; 4],
    version_hash: u32,
    offset: u64,
    header: &WzHeader,
) -> PyResult<Vec<(String, WzProperty)>> {
    use std::io::Cursor;
    let cursor = Cursor::new(raw);
    let mut reader = WzBinaryReader::new(cursor, iv, header.clone(), 0);
    reader.hash = version_hash;
    reader.seek(offset).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    parse_wz_image(&mut reader).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

// Prepare all images for save.
//
// Modified images (present in `cache`) get their properties set so `generate_data`
// re-serialises them. Unmodified images get `raw_data` set to the original bytes
// from `raw`, letting `generate_data` copy them verbatim — no re-parse, no
// re-serialise.
fn populate_directory_fast(
    dir: &mut WzDirectoryEntry,
    raw: &[u8],
    cache: &HashMap<String, Arc<RwLock<Vec<(String, WzProperty)>>>>,
    prefix: &str,
) -> PyResult<()> {
    for img in dir.images.iter_mut() {
        let full_path = if prefix.is_empty() {
            img.name.clone()
        } else {
            format!("{}/{}", prefix, img.name)
        };

        if let Some(cached) = cache.get(&full_path) {
            img.properties = Some(cached.read().map_err(|_| lock_err())?.clone());
            img.raw_data = None;
        } else {
            // Unmodified: slice original bytes directly (O(size) copy, no parsing).
            let start = img.offset as usize;
            let size  = img.size.max(0) as usize;
            let end   = start.saturating_add(size);
            if end > raw.len() {
                return Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                    "Image '{}': offset {} + size {} exceeds file length {}",
                    img.name, start, size, raw.len()
                )));
            }
            img.properties = None;
            img.raw_data = Some(raw[start..end].to_vec());
        }
    }

    for sub in dir.subdirectories.iter_mut() {
        let sub_prefix = if prefix.is_empty() {
            sub.name.clone()
        } else {
            format!("{}/{}", prefix, sub.name)
        };
        populate_directory_fast(sub, raw, cache, &sub_prefix)?;
    }

    Ok(())
}

// ── WzNode ───────────────────────────────────────────────────────────

#[pyclass(name = "WzNode", module = "wzlib")]
pub struct WzNode {
    root: Arc<RwLock<Vec<(String, WzProperty)>>>,
    path: Vec<String>,
    iv: [u8; 4],
}

#[pymethods]
impl WzNode {
    /// The property type name: "Int", "String", "Canvas", "SubProperty", etc.
    fn node_type(&self) -> PyResult<&'static str> {
        let root = self.root.read().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        get_prop(&root, &parts)
            .map(prop_type_name)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))
    }

    /// Return the integer value (Short/Int/Long), or None if not an integer type.
    fn as_int(&self) -> PyResult<Option<i64>> {
        let root = self.root.read().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        Ok(get_prop(&root, &parts).and_then(|p| p.as_int()))
    }

    /// Return the float value (Float/Double), or None if not a float type.
    fn as_float(&self) -> PyResult<Option<f64>> {
        let root = self.root.read().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        Ok(get_prop(&root, &parts).and_then(|p| p.as_float()))
    }

    /// Return the string value (String/UOL), or None if not a string type.
    fn as_str(&self) -> PyResult<Option<String>> {
        let root = self.root.read().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        Ok(get_prop(&root, &parts)
            .and_then(|p| p.as_str().map(|s| s.to_string())))
    }

    /// Names of direct child nodes. Returns [] for leaf nodes.
    fn children(&self) -> PyResult<Vec<String>> {
        let root = self.root.read().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        let prop = get_prop(&root, &parts)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))?;
        Ok(prop_children(prop)
            .map(|c| c.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default())
    }

    /// Get a child node by name. Returns None if not found.
    fn get(&self, name: &str) -> PyResult<Option<WzNode>> {
        let mut new_path = self.path.clone();
        new_path.push(name.to_string());
        let root = self.root.read().map_err(|_| lock_err())?;
        let parts: Vec<&str> = new_path.iter().map(String::as_str).collect();
        if get_prop(&root, &parts).is_some() {
            Ok(Some(WzNode {
                root: Arc::clone(&self.root),
                path: new_path,
                iv: self.iv,
            }))
        } else {
            Ok(None)
        }
    }

    /// Set the scalar value of this node (int, float, or str).
    /// The Rust variant is preserved: Short stays Short, UOL stays UOL, etc.
    /// Raises ValueError if an integer value is out of range for the node's type.
    fn set(&self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut root = self.root.write().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        let prop = get_prop_mut(&mut root, &parts)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))?;

        if let Ok(v) = value.extract::<i64>() {
            *prop = match prop {
                WzProperty::Short(_) => {
                    let s = i16::try_from(v).map_err(|_| PyValueError::new_err(format!(
                        "value {} out of range for Short (-32768..=32767)", v
                    )))?;
                    WzProperty::Short(s)
                }
                WzProperty::Long(_) => WzProperty::Long(v),
                _ => {
                    let i = i32::try_from(v).map_err(|_| PyValueError::new_err(format!(
                        "value {} out of range for Int (-2147483648..=2147483647)", v
                    )))?;
                    WzProperty::Int(i)
                }
            };
        } else if let Ok(v) = value.extract::<f64>() {
            *prop = match prop {
                WzProperty::Double(_) => WzProperty::Double(v),
                _ => WzProperty::Float(v as f32),
            };
        } else if let Ok(v) = value.extract::<String>() {
            *prop = match prop {
                WzProperty::Uol(_) => WzProperty::Uol(v),
                _ => WzProperty::String(v),
            };
        } else {
            return Err(PyValueError::new_err("value must be int, float, or str"));
        }
        Ok(())
    }

    /// Add a child node. The parent must be a container (SubProperty, Canvas, etc.).
    /// `value` may be int, float, str, or bytes (stored as Lua blob).
    fn add(&self, name: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut root = self.root.write().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        let prop = get_prop_mut(&mut root, &parts)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))?;
        let type_name = prop_type_name(prop);
        let children = prop_children_mut(prop)
            .ok_or_else(|| PyValueError::new_err(format!(
                "'{}' is a leaf node ({}), cannot add children",
                self.path.join("/"), type_name
            )))?;

        let new_prop = py_value_to_property(value)?;
        // Replace if name already exists, otherwise push
        if let Some(entry) = children.iter_mut().find(|(n, _)| n == name) {
            entry.1 = new_prop;
        } else {
            children.push((name.to_string(), new_prop));
        }
        Ok(())
    }

    /// Add a child node with an explicit type. The parent must be a container.
    /// `type_hint`: "short", "int", "long", "float", "double", "string", "uol", "lua"
    fn add_typed(&self, name: &str, type_hint: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut root = self.root.write().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        let prop = get_prop_mut(&mut root, &parts)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))?;
        let type_name = prop_type_name(prop);
        let children = prop_children_mut(prop)
            .ok_or_else(|| PyValueError::new_err(format!(
                "'{}' is a leaf node ({}), cannot add children",
                self.path.join("/"), type_name
            )))?;

        let new_prop = py_value_to_typed_property(type_hint, value)?;
        if let Some(entry) = children.iter_mut().find(|(n, _)| n == name) {
            entry.1 = new_prop;
        } else {
            children.push((name.to_string(), new_prop));
        }
        Ok(())
    }

    /// Remove a named child node. Returns True if removed, False if not found.
    fn remove(&self, name: &str) -> PyResult<bool> {
        let mut root = self.root.write().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        let prop = get_prop_mut(&mut root, &parts)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))?;
        let children = prop_children_mut(prop)
            .ok_or_else(|| PyValueError::new_err(format!(
                "'{}' is a leaf node, cannot remove children",
                self.path.join("/")
            )))?;
        let before = children.len();
        children.retain(|(n, _)| n != name);
        Ok(children.len() < before)
    }

    /// Decode a Canvas node to raw RGBA8888 bytes.
    /// Returns (rgba_bytes, width, height).
    fn decode_canvas<'py>(&self, py: Python<'py>) -> PyResult<(Bound<'py, PyBytes>, u32, u32)> {
        let root = self.root.read().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        let prop = get_prop(&root, &parts)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))?;
        match prop {
            WzProperty::Canvas { width, height, format, png_data, .. } => {
                let wz_key = generate_wz_key(&self.iv, 0x10000, None);
                let raw = decompress_png_data(png_data, Some(&wz_key))
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
                let rgba = decode_pixels(&raw, *width as u32, *height as u32, *format)
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
                Ok((PyBytes::new(py, &rgba), *width as u32, *height as u32))
            }
            other => Err(PyValueError::new_err(format!(
                "Node '{}' is {}, not Canvas",
                self.path.join("/"),
                prop_type_name(other)
            ))),
        }
    }

    /// Replace the Canvas pixel data with new RGBA8888 bytes.
    /// Always encodes as BGRA8888 (lossless; DXT encoding is unsupported).
    fn replace_canvas(&self, rgba: &[u8], width: u32, height: u32) -> PyResult<()> {
        let mut root = self.root.write().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        let prop = get_prop_mut(&mut root, &parts)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))?;
        match prop {
            WzProperty::Canvas { width: w, height: h, format, png_data, .. } => {
                let encoded = encode_pixels(rgba, width, height, WzPngFormat::Bgra8888)
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
                let compressed = compress_png_data(&encoded)
                    .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
                *w = width as i32;
                *h = height as i32;
                *format = WzPngFormat::Bgra8888;
                *png_data = compressed;
                Ok(())
            }
            other => Err(PyValueError::new_err(format!(
                "Node '{}' is {}, not Canvas",
                self.path.join("/"),
                prop_type_name(other)
            ))),
        }
    }

    /// Extract raw audio bytes from a Sound node.
    fn extract_sound<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let root = self.root.read().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        let prop = get_prop(&root, &parts)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))?;
        match prop {
            WzProperty::Sound { data, .. } => Ok(PyBytes::new(py, data)),
            other => Err(PyValueError::new_err(format!(
                "Node '{}' is {}, not Sound",
                self.path.join("/"),
                prop_type_name(other)
            ))),
        }
    }

    /// Serialise this node and its subtree as a compact JSON string.
    /// Matches the _node_to_dict format: {path, type, value?, children?}.
    /// `max_depth`: -1 = unlimited, 0 = this node only (no children).
    /// One read-lock for the entire traversal — much faster than per-node Python calls.
    #[pyo3(signature = (max_depth = -1))]
    fn to_json(&self, max_depth: i32) -> PyResult<String> {
        let root = self.root.read().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        let prop = get_prop(&root, &parts)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))?;
        let path_str = self.path.join("/");
        let mut out = String::new();
        prop_to_json(prop, &path_str, max_depth, 0, &mut out);
        Ok(out)
    }

    /// Format this node and its subtree as a human-readable tree string.
    /// `max_depth`: -1 = unlimited, 0 = this node only.
    /// One read-lock for the entire traversal.
    #[pyo3(signature = (max_depth = -1))]
    fn to_tree_str(&self, max_depth: i32) -> PyResult<String> {
        let root = self.root.read().map_err(|_| lock_err())?;
        let parts = self.path_parts_ref();
        let prop = get_prop(&root, &parts)
            .ok_or_else(|| PyKeyError::new_err(self.path.join("/")))?;
        let label = self.path.last().map(|s| s.as_str()).unwrap_or("root");
        let mut out = String::new();
        prop_to_tree(prop, label, "", "", max_depth, 0, &mut out);
        Ok(out)
    }

    /// The slash-joined path from the image root, e.g. "info/hp".
    #[getter]
    fn path(&self) -> String {
        self.path.join("/")
    }

    fn __repr__(&self) -> PyResult<String> {
        let path_str = self.path.join("/");
        let type_name = self.root.read().ok()
            .and_then(|r| {
                let parts: Vec<&str> = self.path.iter().map(String::as_str).collect();
                get_prop(&r, &parts).map(prop_type_name)
            })
            .unwrap_or("?");
        Ok(format!("WzNode('{}', type={})", path_str, type_name))
    }
}

impl WzNode {
    fn path_parts_ref(&self) -> Vec<&str> {
        self.path.iter().map(String::as_str).collect()
    }
}

fn py_value_to_property(value: &Bound<'_, PyAny>) -> PyResult<WzProperty> {
    if let Ok(v) = value.extract::<i64>() {
        let i = i32::try_from(v).map_err(|_| PyValueError::new_err(format!(
            "value {} out of range for Int. Use add_typed() with \"long\" for large integers.", v
        )))?;
        Ok(WzProperty::Int(i))
    } else if let Ok(v) = value.extract::<f64>() {
        Ok(WzProperty::Float(v as f32))
    } else if let Ok(v) = value.extract::<String>() {
        Ok(WzProperty::String(v))
    } else if let Ok(v) = value.extract::<Vec<u8>>() {
        Ok(WzProperty::Lua(v))
    } else {
        Err(PyValueError::new_err("value must be int, float, str, or bytes"))
    }
}

fn py_value_to_typed_property(type_hint: &str, value: &Bound<'_, PyAny>) -> PyResult<WzProperty> {
    match type_hint.to_lowercase().as_str() {
        "short" => {
            let v = value.extract::<i64>()
                .map_err(|_| PyValueError::new_err("'short' requires an int value"))?;
            let s = i16::try_from(v).map_err(|_| PyValueError::new_err(format!(
                "value {} out of range for Short (-32768..=32767)", v
            )))?;
            Ok(WzProperty::Short(s))
        }
        "int" => {
            let v = value.extract::<i64>()
                .map_err(|_| PyValueError::new_err("'int' requires an int value"))?;
            let i = i32::try_from(v).map_err(|_| PyValueError::new_err(format!(
                "value {} out of range for Int (-2147483648..=2147483647)", v
            )))?;
            Ok(WzProperty::Int(i))
        }
        "long" => {
            let v = value.extract::<i64>()
                .map_err(|_| PyValueError::new_err("'long' requires an int value"))?;
            Ok(WzProperty::Long(v))
        }
        "float" => {
            let v = value.extract::<f64>()
                .map_err(|_| PyValueError::new_err("'float' requires a float value"))?;
            Ok(WzProperty::Float(v as f32))
        }
        "double" => {
            let v = value.extract::<f64>()
                .map_err(|_| PyValueError::new_err("'double' requires a float value"))?;
            Ok(WzProperty::Double(v))
        }
        "string" | "str" => {
            let v = value.extract::<String>()
                .map_err(|_| PyValueError::new_err("'string' requires a str value"))?;
            Ok(WzProperty::String(v))
        }
        "uol" => {
            let v = value.extract::<String>()
                .map_err(|_| PyValueError::new_err("'uol' requires a str value"))?;
            Ok(WzProperty::Uol(v))
        }
        "lua" | "bytes" => {
            let v = value.extract::<Vec<u8>>()
                .map_err(|_| PyValueError::new_err("'lua' requires a bytes value"))?;
            Ok(WzProperty::Lua(v))
        }
        other => Err(PyValueError::new_err(format!(
            "Unknown type hint '{}'. Use: short, int, long, float, double, string, uol, lua",
            other
        ))),
    }
}

// ── WzImage ──────────────────────────────────────────────────────────

#[pyclass(name = "WzImage", module = "wzlib")]
pub struct WzImage {
    props: Arc<RwLock<Vec<(String, WzProperty)>>>,
    iv: [u8; 4],
}

#[pymethods]
impl WzImage {
    /// Open a standalone img file (hotfix Data.wz or any bare WzImage binary).
    #[staticmethod]
    #[pyo3(signature = (path, version = "bms"))]
    fn open(path: &str, version: &str) -> PyResult<Self> {
        let iv = version_to_iv(version)?;
        let raw = std::fs::read(path).map_err(|e| PyIOError::new_err(e.to_string()))?;
        let props = parse_hotfix_data_wz(&raw, iv)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(Self { props: Arc::new(RwLock::new(props)), iv })
    }

    /// Parse a WzImage from raw bytes.
    #[staticmethod]
    #[pyo3(signature = (data, version = "bms"))]
    fn from_bytes(data: &[u8], version: &str) -> PyResult<Self> {
        let iv = version_to_iv(version)?;
        let props = parse_hotfix_data_wz(data, iv)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(Self { props: Arc::new(RwLock::new(props)), iv })
    }

    /// Get a node by slash-separated path, e.g. "info/hp". Returns None if not found.
    fn get(&self, path: &str) -> PyResult<Option<WzNode>> {
        let parts = path_parts(path);
        let root = self.props.read().map_err(|_| lock_err())?;
        if get_prop(&root, &parts).is_some() {
            Ok(Some(WzNode {
                root: Arc::clone(&self.props),
                path: parts.into_iter().map(|s| s.to_string()).collect(),
                iv: self.iv,
            }))
        } else {
            Ok(None)
        }
    }

    /// Names of the root-level child nodes.
    fn children(&self) -> PyResult<Vec<String>> {
        let root = self.props.read().map_err(|_| lock_err())?;
        Ok(root.iter().map(|(n, _)| n.clone()).collect())
    }

    /// Add or replace a root-level node. `value` may be int, float, str, or bytes.
    fn add(&self, name: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let new_prop = py_value_to_property(value)?;
        let mut root = self.props.write().map_err(|_| lock_err())?;
        if let Some(entry) = root.iter_mut().find(|(n, _)| n == name) {
            entry.1 = new_prop;
        } else {
            root.push((name.to_string(), new_prop));
        }
        Ok(())
    }

    /// Add or replace a root-level node with an explicit type.
    /// `type_hint`: "short", "int", "long", "float", "double", "string", "uol", "lua"
    fn add_typed(&self, name: &str, type_hint: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let new_prop = py_value_to_typed_property(type_hint, value)?;
        let mut root = self.props.write().map_err(|_| lock_err())?;
        if let Some(entry) = root.iter_mut().find(|(n, _)| n == name) {
            entry.1 = new_prop;
        } else {
            root.push((name.to_string(), new_prop));
        }
        Ok(())
    }

    /// Remove a root-level node by name. Returns True if removed.
    fn remove(&self, name: &str) -> PyResult<bool> {
        let mut root = self.props.write().map_err(|_| lock_err())?;
        let before = root.len();
        root.retain(|(n, _)| n != name);
        Ok(root.len() < before)
    }

    /// Serialize to WZ image binary (can be used with WzFile.build or saved standalone).
    fn build<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(py, &self.build_bytes_internal()?))
    }

    /// Serialize and write to a file.
    fn save(&self, path: &str) -> PyResult<()> {
        std::fs::write(path, self.build_bytes_internal()?)
            .map_err(|e| PyIOError::new_err(e.to_string()))
    }

    /// Serialise all root nodes as a compact JSON object `{name: {path,type,...}}`.
    /// One read-lock for the entire traversal.
    #[pyo3(signature = (max_depth = -1))]
    fn to_json(&self, max_depth: i32) -> PyResult<String> {
        let root = self.props.read().map_err(|_| lock_err())?;
        let mut out = String::from("{");
        for (i, (name, prop)) in root.iter().enumerate() {
            if i > 0 { out.push(','); }
            out.push_str(&json_string(name));
            out.push(':');
            prop_to_json(prop, name, max_depth, 0, &mut out);
        }
        out.push('}');
        Ok(out)
    }

    /// Format all root nodes as a human-readable tree string.
    /// One read-lock for the entire traversal.
    #[pyo3(signature = (max_depth = -1))]
    fn to_tree_str(&self, max_depth: i32) -> PyResult<String> {
        let root = self.props.read().map_err(|_| lock_err())?;
        let mut out = String::new();
        for (i, (name, prop)) in root.iter().enumerate() {
            if i > 0 { out.push('\n'); }
            prop_to_tree(prop, name, "", "", max_depth, 0, &mut out);
        }
        Ok(out)
    }

    fn __repr__(&self) -> String {
        let count = self.props.read().map(|r| r.len()).unwrap_or(0);
        format!("WzImage({} root nodes)", count)
    }
}

impl WzImage {
    fn build_bytes_internal(&self) -> PyResult<Vec<u8>> {
        let root = self.props.read().map_err(|_| lock_err())?;
        save_hotfix_data_wz(&root, self.iv)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }
}

// ── WzFile internals ─────────────────────────────────────────────────

struct WzFileInner {
    wz: WzFile,
    // Arc so we can clone the pointer (O(1)) to parse images without holding the lock.
    raw: Arc<Vec<u8>>,
    image_cache: HashMap<String, Arc<RwLock<Vec<(String, WzProperty)>>>>,
}

// ── WzFile ───────────────────────────────────────────────────────────

#[pyclass(name = "WzFile", module = "wzlib")]
pub struct PyWzFile {
    inner: Arc<Mutex<WzFileInner>>,
}

#[pymethods]
impl PyWzFile {
    /// Open a standard PKG1 WZ file.
    /// `version`: "gms", "ems"/"msea", or "bms"/"classic" (default "bms").
    /// `patch_version`: supply the known patch version to skip brute-force detection.
    #[staticmethod]
    #[pyo3(signature = (path, version = "bms", patch_version = None))]
    fn open(path: &str, version: &str, patch_version: Option<i16>) -> PyResult<Self> {
        let maple = version_to_maple(version)?;
        let raw = std::fs::read(path).map_err(|e| PyIOError::new_err(e.to_string()))?;
        let wz = WzFile::parse(&raw, maple, patch_version)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(WzFileInner {
                wz,
                raw: Arc::new(raw),
                image_cache: HashMap::new(),
            })),
        })
    }

    /// All image paths in depth-first order, e.g. ["0100000.img", "Mob/0100100.img"].
    fn list_images(&self) -> PyResult<Vec<String>> {
        let guard = self.inner.lock().map_err(|_| lock_err())?;
        let mut paths = Vec::new();
        collect_image_paths(&guard.wz.directory, "", &mut paths);
        Ok(paths)
    }

    /// Get an image by path (e.g. "0100000.img" or "Mob/0100100.img").
    /// The result is cached; calling image() again returns the same object (same Arc).
    fn image(&self, name: &str) -> PyResult<WzImage> {
        let guard = self.inner.lock().map_err(|_| lock_err())?;

        if let Some(cached) = guard.image_cache.get(name) {
            return Ok(WzImage { props: Arc::clone(cached), iv: guard.wz.iv });
        }

        let (offset, iv) = {
            let entry = find_image_entry(&guard.wz.directory, name)
                .ok_or_else(|| PyKeyError::new_err(format!("Image not found: '{}'", name)))?;
            (entry.offset, entry.iv.unwrap_or(guard.wz.iv))
        };
        let version_hash = guard.wz.version_hash;
        let header = guard.wz.header.clone();
        let raw = Arc::clone(&guard.raw); // O(1) — just increment refcount

        // Release lock while parsing (parsing can be slow for large images).
        drop(guard);

        let props = parse_image_at(&raw, iv, version_hash, offset, &header)?;
        let props_arc = Arc::new(RwLock::new(props));

        let mut guard = self.inner.lock().map_err(|_| lock_err())?;
        // Use entry() in case another caller parsed the same image concurrently.
        let cached = guard
            .image_cache
            .entry(name.to_string())
            .or_insert_with(|| Arc::clone(&props_arc));
        Ok(WzImage { props: Arc::clone(cached), iv })
    }

    /// All images (including unmodified ones) are read and re-serialized.
    fn build<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(py, &self.build_bytes_internal()?))
    }

    /// Serialize and write to a file.
    fn save(&self, path: &str) -> PyResult<()> {
        std::fs::write(path, self.build_bytes_internal()?)
            .map_err(|e| PyIOError::new_err(e.to_string()))
    }

    /// The detected patch version number.
    fn version(&self) -> PyResult<i16> {
        Ok(self.inner.lock().map_err(|_| lock_err())?.wz.version)
    }

    /// Whether this is a 64-bit WZ format (v770+).
    fn is_64bit(&self) -> PyResult<bool> {
        Ok(self.inner.lock().map_err(|_| lock_err())?.wz.is_64bit)
    }

    fn __repr__(&self) -> PyResult<String> {
        let guard = self.inner.lock().map_err(|_| lock_err())?;
        let mut count = 0usize;
        fn count_imgs(dir: &WzDirectoryEntry, n: &mut usize) {
            *n += dir.images.len();
            for s in &dir.subdirectories { count_imgs(s, n); }
        }
        count_imgs(&guard.wz.directory, &mut count);
        Ok(format!(
            "WzFile(version={}, is_64bit={}, images={})",
            guard.wz.version, guard.wz.is_64bit, count
        ))
    }
}

impl PyWzFile {
    fn build_bytes_internal(&self) -> PyResult<Vec<u8>> {
        let mut guard = self.inner.lock().map_err(|_| lock_err())?;
        let raw = Arc::clone(&guard.raw);

        // Borrow split: &mut wz.directory and &image_cache are different fields.
        let inner = &mut *guard;
        populate_directory_fast(
            &mut inner.wz.directory,
            &raw,
            &inner.image_cache,
            "",
        )?;

        inner.wz.save().map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }
}

// ── Module ────────────────────────────────────────────────────────────

#[pymodule]
fn _wzlib(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyWzFile>()?;
    m.add_class::<WzImage>()?;
    m.add_class::<WzNode>()?;
    Ok(())
}

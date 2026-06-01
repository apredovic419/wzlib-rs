//! WZ XML export and import — server/client mode.
//!
//! Matches the WzLib/HaRepacker XML convention used by MapleStory server tools.
//! Server (MetadataOnly): no binary data — canvas/sound/lua are metadata stubs.
//! Client (WithBinaryData): binary data base64-encoded in `basedata` attributes.

use std::collections::HashMap;

use base64::Engine as _;

use crate::image;
use crate::image::encode as image_encode;
use crate::wz::error::{WzError, WzResult};
use crate::wz::properties::WzProperty;
use crate::wz::types::WzPngFormat;

// ── Public types ─────────────────────────────────────────────────────

pub enum XmlMode {
    /// Server mode: no binary data. Canvas/Sound/Lua/RawData are metadata-only stubs.
    MetadataOnly,
    /// Client mode: binary data base64-encoded in `basedata` attributes.
    WithBinaryData,
}

// ── Export ───────────────────────────────────────────────────────────

/// Serialize an image property list to WZ XML format.
///
/// `img_name` becomes the root `<imgdir name="...">` element's name attribute
/// (typically the .img filename, e.g. `"00002000.img"`).
pub fn export_wz_xml(img_name: &str, props: &[(String, WzProperty)], mode: &XmlMode) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    out.push_str("<imgdir name=\"");
    xml_escape_into(&mut out, img_name);
    out.push_str("\">\n");
    for (name, prop) in props {
        write_prop(&mut out, name, prop, mode, 1);
    }
    out.push_str("</imgdir>\n");
    out
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn xml_escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

fn write_prop(out: &mut String, name: &str, prop: &WzProperty, mode: &XmlMode, depth: usize) {
    match prop {
        WzProperty::Null => {
            indent(out, depth);
            out.push_str("<null name=\"");
            xml_escape_into(out, name);
            out.push_str("\"/>\n");
        }
        WzProperty::Short(v) => {
            indent(out, depth);
            out.push_str("<short name=\"");
            xml_escape_into(out, name);
            out.push_str("\" value=\"");
            out.push_str(&v.to_string());
            out.push_str("\"/>\n");
        }
        WzProperty::Int(v) => {
            indent(out, depth);
            out.push_str("<int name=\"");
            xml_escape_into(out, name);
            out.push_str("\" value=\"");
            out.push_str(&v.to_string());
            out.push_str("\"/>\n");
        }
        WzProperty::Long(v) => {
            indent(out, depth);
            out.push_str("<long name=\"");
            xml_escape_into(out, name);
            out.push_str("\" value=\"");
            out.push_str(&v.to_string());
            out.push_str("\"/>\n");
        }
        WzProperty::Float(v) => {
            indent(out, depth);
            out.push_str("<float name=\"");
            xml_escape_into(out, name);
            out.push_str("\" value=\"");
            let s = if v.is_finite() {
                v.to_string()
            } else {
                "0".to_string()
            };
            out.push_str(&s);
            out.push_str("\"/>\n");
        }
        WzProperty::Double(v) => {
            indent(out, depth);
            out.push_str("<double name=\"");
            xml_escape_into(out, name);
            out.push_str("\" value=\"");
            let s = if v.is_finite() {
                v.to_string()
            } else {
                "0".to_string()
            };
            out.push_str(&s);
            out.push_str("\"/>\n");
        }
        WzProperty::String(v) => {
            indent(out, depth);
            out.push_str("<string name=\"");
            xml_escape_into(out, name);
            out.push_str("\" value=\"");
            xml_escape_into(out, v);
            out.push_str("\"/>\n");
        }
        WzProperty::Uol(v) => {
            indent(out, depth);
            out.push_str("<uol name=\"");
            xml_escape_into(out, name);
            out.push_str("\" value=\"");
            xml_escape_into(out, v);
            out.push_str("\"/>\n");
        }
        WzProperty::Vector { x, y } => {
            indent(out, depth);
            out.push_str("<vector name=\"");
            xml_escape_into(out, name);
            out.push_str("\" x=\"");
            out.push_str(&x.to_string());
            out.push_str("\" y=\"");
            out.push_str(&y.to_string());
            out.push_str("\"/>\n");
        }
        WzProperty::SubProperty { properties } => {
            indent(out, depth);
            out.push_str("<imgdir name=\"");
            xml_escape_into(out, name);
            if properties.is_empty() {
                out.push_str("\"/>\n");
            } else {
                out.push_str("\">\n");
                for (child_name, child_prop) in properties {
                    write_prop(out, child_name, child_prop, mode, depth + 1);
                }
                indent(out, depth);
                out.push_str("</imgdir>\n");
            }
        }
        WzProperty::Canvas {
            width,
            height,
            format,
            scale,
            properties,
            png_data,
        } => {
            indent(out, depth);
            out.push_str("<canvas name=\"");
            xml_escape_into(out, name);
            out.push_str("\" width=\"");
            out.push_str(&width.to_string());
            out.push_str("\" height=\"");
            out.push_str(&height.to_string());
            out.push_str("\"");
            if let XmlMode::WithBinaryData = mode {
                // Decode WZ pixels → RGBA8888 → standard PNG (HaRepacker-compatible).
                // The exported PNG is full-resolution (scale already applied), so
                // re-import yields a scale-0 canvas carrying the same image.
                if let Ok(png_bytes) =
                    canvas_to_standard_png(*width, *height, *scale, *format, png_data)
                {
                    out.push_str(" basedata=\"");
                    out.push_str(&base64::engine::general_purpose::STANDARD.encode(&png_bytes));
                    out.push_str("\"");
                }
            }
            if properties.is_empty() {
                out.push_str("/>\n");
            } else {
                out.push_str(">\n");
                for (child_name, child_prop) in properties {
                    write_prop(out, child_name, child_prop, mode, depth + 1);
                }
                indent(out, depth);
                out.push_str("</canvas>\n");
            }
        }
        WzProperty::Convex { points } => {
            indent(out, depth);
            out.push_str("<convex name=\"");
            xml_escape_into(out, name);
            if points.is_empty() {
                out.push_str("\"/>\n");
            } else {
                out.push_str("\">\n");
                for (child_name, child_prop) in points {
                    write_prop(out, child_name, child_prop, mode, depth + 1);
                }
                indent(out, depth);
                out.push_str("</convex>\n");
            }
        }
        WzProperty::Sound {
            duration_ms,
            header,
            data,
        } => {
            indent(out, depth);
            out.push_str("<sound name=\"");
            xml_escape_into(out, name);
            out.push_str("\"");
            if let XmlMode::WithBinaryData = mode {
                let blob = pack_sound_blob(header, data);
                out.push_str(" duration=\"");
                out.push_str(&duration_ms.to_string());
                out.push_str("\" basedata=\"");
                out.push_str(&base64::engine::general_purpose::STANDARD.encode(&blob));
                out.push_str("\"");
            }
            out.push_str("/>\n");
        }
        WzProperty::Lua(data) => {
            indent(out, depth);
            out.push_str("<lua name=\"");
            xml_escape_into(out, name);
            out.push_str("\"");
            if let XmlMode::WithBinaryData = mode {
                out.push_str(" basedata=\"");
                out.push_str(&base64::engine::general_purpose::STANDARD.encode(data));
                out.push_str("\"");
            }
            out.push_str("/>\n");
        }
        WzProperty::RawData {
            raw_type,
            properties,
            data,
        } => {
            indent(out, depth);
            out.push_str("<rawdata name=\"");
            xml_escape_into(out, name);
            out.push_str("\" type=\"");
            out.push_str(&raw_type.to_string());
            out.push_str("\"");
            if let XmlMode::WithBinaryData = mode {
                out.push_str(" basedata=\"");
                out.push_str(&base64::engine::general_purpose::STANDARD.encode(data));
                out.push_str("\"");
            }
            if properties.is_empty() {
                out.push_str("/>\n");
            } else {
                out.push_str(">\n");
                for (child_name, child_prop) in properties {
                    write_prop(out, child_name, child_prop, mode, depth + 1);
                }
                indent(out, depth);
                out.push_str("</rawdata>\n");
            }
        }
        WzProperty::Video {
            video_type,
            properties,
            data_length,
            ..
        } => {
            // Video binary data is always omitted — it's too large and rarely useful
            indent(out, depth);
            out.push_str("<video name=\"");
            xml_escape_into(out, name);
            out.push_str("\" type=\"");
            out.push_str(&video_type.to_string());
            out.push_str("\" dataLength=\"");
            out.push_str(&data_length.to_string());
            out.push_str("\"");
            if properties.is_empty() {
                out.push_str("/>\n");
            } else {
                out.push_str(">\n");
                for (child_name, child_prop) in properties {
                    write_prop(out, child_name, child_prop, mode, depth + 1);
                }
                indent(out, depth);
                out.push_str("</video>\n");
            }
        }
    }
}

fn pack_sound_blob(header: &[u8], data: &[u8]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(4 + header.len() + data.len());
    blob.extend_from_slice(&(header.len() as u32).to_le_bytes());
    blob.extend_from_slice(header);
    blob.extend_from_slice(data);
    blob
}

// ── Import ───────────────────────────────────────────────────────────

/// Parse a WZ XML string back to `(img_name, properties)`.
///
/// Accepts both server-mode (no basedata) and client-mode (with basedata) XML.
/// Float/Double values accept both `.` and `,` as decimal separators.
pub fn import_wz_xml(xml: &str) -> WzResult<(String, Vec<(String, WzProperty)>)> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    // Parse attributes from a BytesStart element into a HashMap<String, String>.
    fn elem_attrs(e: &quick_xml::events::BytesStart) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for attr_res in e.attributes() {
            if let Ok(a) = attr_res {
                let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                let raw_val = String::from_utf8_lossy(a.value.as_ref()).into_owned();
                map.insert(key, unescape_xml_attr(&raw_val));
            }
        }
        map
    }

    fn elem_name(e: &quick_xml::events::BytesStart) -> String {
        String::from_utf8_lossy(e.name().as_ref()).into_owned()
    }

    fn end_name(e: &quick_xml::events::BytesEnd) -> String {
        String::from_utf8_lossy(e.name().as_ref()).into_owned()
    }

    // Build a leaf WzProperty from a self-closing (Empty) element.
    fn build_leaf(tag: &str, _name: &str, attrs: &HashMap<String, String>) -> Option<WzProperty> {
        match tag {
            "null" => Some(WzProperty::Null),
            "short" => Some(WzProperty::Short(
                attrs.get("value").and_then(|s| s.parse().ok()).unwrap_or(0),
            )),
            "int" => Some(WzProperty::Int(
                attrs.get("value").and_then(|s| s.parse().ok()).unwrap_or(0),
            )),
            "long" => Some(WzProperty::Long(
                attrs.get("value").and_then(|s| s.parse().ok()).unwrap_or(0),
            )),
            "float" => Some(WzProperty::Float(parse_float(
                attrs.get("value").map(|s| s.as_str()).unwrap_or("0"),
            ) as f32)),
            "double" => Some(WzProperty::Double(parse_float(
                attrs.get("value").map(|s| s.as_str()).unwrap_or("0"),
            ))),
            "string" => Some(WzProperty::String(
                attrs.get("value").cloned().unwrap_or_default(),
            )),
            "uol" => Some(WzProperty::Uol(
                attrs.get("value").cloned().unwrap_or_default(),
            )),
            "vector" => Some(WzProperty::Vector {
                x: attrs.get("x").and_then(|s| s.parse().ok()).unwrap_or(0),
                y: attrs.get("y").and_then(|s| s.parse().ok()).unwrap_or(0),
            }),
            "imgdir" => Some(WzProperty::SubProperty {
                properties: Vec::new(),
            }),
            "canvas" => {
                let width = attrs.get("width").and_then(|s| s.parse().ok()).unwrap_or(0);
                let height = attrs
                    .get("height")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let (format, png_data) = canvas_from_basedata(attrs, width, height);
                Some(WzProperty::Canvas {
                    width,
                    height,
                    format,
                    // XML basedata is a full-resolution PNG; no scale needed.
                    scale: 0,
                    properties: Vec::new(),
                    png_data,
                })
            }
            "convex" => Some(WzProperty::Convex { points: Vec::new() }),
            "sound" => {
                let duration_ms = attrs
                    .get("duration")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let (header, data) = unpack_sound_from_attrs(attrs);
                Some(WzProperty::Sound {
                    duration_ms,
                    header,
                    data,
                })
            }
            "lua" => Some(WzProperty::Lua(decode_b64_attr(attrs, "basedata"))),
            "rawdata" => {
                let raw_type = attrs.get("type").and_then(|s| s.parse().ok()).unwrap_or(0);
                let data = decode_b64_attr(attrs, "basedata");
                Some(WzProperty::RawData {
                    raw_type,
                    properties: Vec::new(),
                    data,
                })
            }
            "video" => {
                let video_type = attrs.get("type").and_then(|s| s.parse().ok()).unwrap_or(0);
                let data_length = attrs
                    .get("dataLength")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                Some(WzProperty::Video {
                    video_type,
                    properties: Vec::new(),
                    data_offset: 0,
                    data_length,
                    mcv_header: None,
                    video_data: None,
                })
            }
            _ => None,
        }
    }

    // Stack entries: what's currently open on the XML element stack.
    enum Frame {
        ImgDir {
            name: String,
        },
        Canvas {
            name: String,
            width: i32,
            height: i32,
            format: WzPngFormat,
            png_data: Vec<u8>,
        },
        Convex {
            name: String,
        },
        RawData {
            name: String,
            raw_type: u8,
            data: Vec<u8>,
        },
        Video {
            name: String,
            video_type: u8,
            data_length: u32,
        },
    }

    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<(Frame, Vec<(String, WzProperty)>)> = Vec::new();
    let mut root_name = String::from("root");
    let mut result: Vec<(String, WzProperty)> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let tag = elem_name(e);
                let attrs = elem_attrs(e);
                let name = attrs.get("name").cloned().unwrap_or_default();

                match tag.as_str() {
                    "imgdir" => {
                        if stack.is_empty() {
                            root_name = name.clone();
                        }
                        stack.push((Frame::ImgDir { name }, Vec::new()));
                    }
                    "canvas" => {
                        let width = attrs.get("width").and_then(|s| s.parse().ok()).unwrap_or(0);
                        let height = attrs
                            .get("height")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        let (fmt, png_data) = canvas_from_basedata(&attrs, width, height);
                        stack.push((
                            Frame::Canvas {
                                name,
                                width,
                                height,
                                format: fmt,
                                png_data,
                            },
                            Vec::new(),
                        ));
                    }
                    "convex" => {
                        stack.push((Frame::Convex { name }, Vec::new()));
                    }
                    "rawdata" => {
                        let raw_type = attrs.get("type").and_then(|s| s.parse().ok()).unwrap_or(0);
                        let data = decode_b64_attr(&attrs, "basedata");
                        stack.push((
                            Frame::RawData {
                                name,
                                raw_type,
                                data,
                            },
                            Vec::new(),
                        ));
                    }
                    "video" => {
                        let video_type =
                            attrs.get("type").and_then(|s| s.parse().ok()).unwrap_or(0);
                        let data_length = attrs
                            .get("dataLength")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        stack.push((
                            Frame::Video {
                                name,
                                video_type,
                                data_length,
                            },
                            Vec::new(),
                        ));
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let tag = elem_name(e);
                let attrs = elem_attrs(e);
                let name = attrs.get("name").cloned().unwrap_or_default();

                if tag.as_str() == "imgdir" && stack.is_empty() {
                    // Root element with no children
                    root_name = name.clone();
                    continue;
                }

                if let Some(prop) = build_leaf(&tag, &name, &attrs) {
                    if let Some((_, children)) = stack.last_mut() {
                        children.push((name, prop));
                    } else {
                        result.push((name, prop));
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let closing_tag = end_name(e);
                let _ = closing_tag; // tag matched by stack order

                if let Some((frame, children)) = stack.pop() {
                    let (name, prop) = match frame {
                        Frame::ImgDir { name } => {
                            if stack.is_empty() {
                                result = children;
                                continue;
                            }
                            (
                                name,
                                WzProperty::SubProperty {
                                    properties: children,
                                },
                            )
                        }
                        Frame::Canvas {
                            name,
                            width,
                            height,
                            format,
                            png_data,
                        } => (
                            name,
                            WzProperty::Canvas {
                                width,
                                height,
                                format,
                                scale: 0,
                                properties: children,
                                png_data,
                            },
                        ),
                        Frame::Convex { name } => (name, WzProperty::Convex { points: children }),
                        Frame::RawData {
                            name,
                            raw_type,
                            data,
                        } => (
                            name,
                            WzProperty::RawData {
                                raw_type,
                                properties: children,
                                data,
                            },
                        ),
                        Frame::Video {
                            name,
                            video_type,
                            data_length,
                        } => (
                            name,
                            WzProperty::Video {
                                video_type,
                                properties: children,
                                data_offset: 0,
                                data_length,
                                mcv_header: None,
                                video_data: None,
                            },
                        ),
                    };

                    if let Some((_, parent_children)) = stack.last_mut() {
                        parent_children.push((name, prop));
                    } else {
                        // Closed a top-level element (not root imgdir) — shouldn't happen
                        result.push((name, prop));
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(WzError::Custom(format!("XML parse error: {}", e)));
            }
            _ => {}
        }
    }

    Ok((root_name, result))
}

// ── Internal helpers ─────────────────────────────────────────────────

/// Accept both `.` and `,` as decimal separator (HaRepacker compat).
fn parse_float(s: &str) -> f64 {
    let normalized = s.replace(',', ".");
    normalized.parse().unwrap_or(0.0)
}

fn decode_b64_attr(attrs: &HashMap<String, String>, key: &str) -> Vec<u8> {
    attrs
        .get(key)
        .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
        .unwrap_or_default()
}

fn unpack_sound_from_attrs(attrs: &HashMap<String, String>) -> (Vec<u8>, Vec<u8>) {
    let blob = decode_b64_attr(attrs, "basedata");
    if blob.len() < 4 {
        return (Vec::new(), Vec::new());
    }
    let header_len = u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]]) as usize;
    if 4 + header_len > blob.len() {
        return (Vec::new(), Vec::new());
    }
    let header = blob[4..4 + header_len].to_vec();
    let data = blob[4 + header_len..].to_vec();
    (header, data)
}

/// Decode a Canvas WZ pixel blob to a standard PNG file.
///
/// Pipeline: zlib-decompress → decode WZ pixels → RGBA8888 → encode as PNG.
fn canvas_to_standard_png(
    width: i32,
    height: i32,
    scale: u8,
    format: WzPngFormat,
    png_data: &[u8],
) -> WzResult<Vec<u8>> {
    let raw = image::decompress_png_data(png_data, None)?;
    let rgba = image::decode_canvas_pixels(&raw, width as u32, height as u32, scale, format)?;
    rgba_to_png_bytes(&rgba, width as u32, height as u32)
}

/// Encode RGBA8888 bytes as a standard PNG file.
fn rgba_to_png_bytes(rgba: &[u8], width: u32, height: u32) -> WzResult<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| WzError::Custom(format!("PNG encode: {}", e)))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| WzError::Custom(format!("PNG encode: {}", e)))?;
    drop(writer);
    Ok(out)
}

/// Decode a standard PNG file to RGBA8888 bytes.
fn png_bytes_to_rgba(png_bytes: &[u8]) -> WzResult<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|e| WzError::Custom(format!("PNG decode: {}", e)))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| WzError::Custom(format!("PNG decode: {}", e)))?;
    let w = frame.width;
    let h = frame.height;
    let raw = &buf[..frame.buffer_size()];

    // Normalise to RGBA8888.
    let rgba = match frame.color_type {
        png::ColorType::Rgba => raw.to_vec(),
        png::ColorType::Rgb => {
            let mut v = Vec::with_capacity(w as usize * h as usize * 4);
            for px in raw.chunks(3) {
                v.extend_from_slice(px);
                v.push(255);
            }
            v
        }
        png::ColorType::GrayscaleAlpha => {
            let mut v = Vec::with_capacity(w as usize * h as usize * 4);
            for px in raw.chunks(2) {
                v.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            v
        }
        png::ColorType::Grayscale => {
            let mut v = Vec::with_capacity(w as usize * h as usize * 4);
            for &g in raw {
                v.extend_from_slice(&[g, g, g, 255]);
            }
            v
        }
        // Indexed / other — return raw and let the caller decide
        _ => raw.to_vec(),
    };

    Ok((w, h, rgba))
}

/// Parse Canvas `basedata` attribute: standard PNG → WZ png_data (BGRA8888, zlib-compressed).
/// Returns `(format, png_data)`. If `basedata` is absent or invalid, returns empty data.
fn canvas_from_basedata(
    attrs: &HashMap<String, String>,
    _hint_width: i32,
    _hint_height: i32,
) -> (WzPngFormat, Vec<u8>) {
    let b64 = match attrs.get("basedata") {
        Some(v) if !v.is_empty() => v,
        _ => return (WzPngFormat::Bgra8888, Vec::new()),
    };
    let png_bytes = match base64::engine::general_purpose::STANDARD.decode(b64) {
        Ok(b) => b,
        Err(_) => return (WzPngFormat::Bgra8888, Vec::new()),
    };
    match png_bytes_to_rgba(&png_bytes) {
        Ok((w, h, rgba)) => {
            let wz_pixels = match image_encode::encode_pixels(&rgba, w, h, WzPngFormat::Bgra8888) {
                Ok(p) => p,
                Err(_) => return (WzPngFormat::Bgra8888, Vec::new()),
            };
            let compressed = match image_encode::compress_png_data(&wz_pixels) {
                Ok(c) => c,
                Err(_) => return (WzPngFormat::Bgra8888, Vec::new()),
            };
            (WzPngFormat::Bgra8888, compressed)
        }
        Err(_) => (WzPngFormat::Bgra8888, Vec::new()),
    }
}

/// Unescape XML attribute value entities.
fn unescape_xml_attr(s: &str) -> String {
    // Fast path: no entities present
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wz::properties::WzProperty;
    use crate::wz::types::WzPngFormat;

    fn make_props() -> Vec<(String, WzProperty)> {
        vec![
            (
                "info".to_string(),
                WzProperty::SubProperty {
                    properties: vec![
                        ("islot".to_string(), WzProperty::String("Bd".to_string())),
                        ("cash".to_string(), WzProperty::Int(0)),
                    ],
                },
            ),
            ("delay".to_string(), WzProperty::Int(120)),
            ("origin".to_string(), WzProperty::Vector { x: -36, y: 100 }),
            ("face".to_string(), WzProperty::Short(1)),
        ]
    }

    #[test]
    fn test_export_server_mode() {
        let props = make_props();
        let xml = export_wz_xml("test.img", &props, &XmlMode::MetadataOnly);
        assert!(xml.starts_with("<?xml version=\"1.0\""));
        assert!(xml.contains("<imgdir name=\"test.img\">"));
        assert!(xml.contains("<imgdir name=\"info\">"));
        assert!(xml.contains("<string name=\"islot\" value=\"Bd\"/>"));
        assert!(xml.contains("<int name=\"cash\" value=\"0\"/>"));
        assert!(xml.contains("</imgdir>"));
        assert!(xml.contains("<int name=\"delay\" value=\"120\"/>"));
        assert!(xml.contains("<vector name=\"origin\" x=\"-36\" y=\"100\"/>"));
        assert!(xml.contains("<short name=\"face\" value=\"1\"/>"));
        // No binary data markers
        assert!(!xml.contains("basedata"));
    }

    #[test]
    fn test_export_empty_subproperty() {
        let props = vec![(
            "stop".to_string(),
            WzProperty::SubProperty {
                properties: vec![(
                    "0".to_string(),
                    WzProperty::SubProperty { properties: vec![] },
                )],
            },
        )];
        let xml = export_wz_xml("test.img", &props, &XmlMode::MetadataOnly);
        assert!(xml.contains("<imgdir name=\"0\"/>"));
    }

    #[test]
    fn test_export_xml_escape() {
        let props = vec![("a&b".to_string(), WzProperty::String("val<>\"".to_string()))];
        let xml = export_wz_xml("test.img", &props, &XmlMode::MetadataOnly);
        assert!(xml.contains("name=\"a&amp;b\""));
        assert!(xml.contains("value=\"val&lt;&gt;&quot;\""));
    }

    #[test]
    fn test_import_basic() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<imgdir name="00002000.img">
  <imgdir name="info">
    <string name="islot" value="Bd"/>
    <int name="cash" value="0"/>
  </imgdir>
  <short name="face" value="1"/>
  <vector name="origin" x="19" y="32"/>
</imgdir>"#;

        let (name, props) = import_wz_xml(xml).unwrap();
        assert_eq!(name, "00002000.img");
        assert_eq!(props.len(), 3);

        // First child: info SubProperty
        let (n, p) = &props[0];
        assert_eq!(n, "info");
        if let WzProperty::SubProperty { properties } = p {
            assert_eq!(properties.len(), 2);
            assert_eq!(properties[0].0, "islot");
            assert!(matches!(&properties[0].1, WzProperty::String(s) if s == "Bd"));
            assert!(matches!(&properties[1].1, WzProperty::Int(0)));
        } else {
            panic!("Expected SubProperty");
        }

        // Short
        assert!(matches!(&props[1].1, WzProperty::Short(1)));
        // Vector
        assert!(matches!(&props[2].1, WzProperty::Vector { x: 19, y: 32 }));
    }

    #[test]
    fn test_import_double_comma() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<imgdir name="root">
  <double name="speed" value="1,5"/>
  <float name="rate" value="2.5"/>
</imgdir>"#;
        let (_, props) = import_wz_xml(xml).unwrap();
        assert!(matches!(&props[0].1, WzProperty::Double(v) if (*v - 1.5).abs() < 1e-9));
        assert!(matches!(&props[1].1, WzProperty::Float(v) if (*v - 2.5f32).abs() < 1e-6));
    }

    #[test]
    fn test_roundtrip_server_mode() {
        let props = make_props();
        let xml = export_wz_xml("test.img", &props, &XmlMode::MetadataOnly);
        let (name, imported) = import_wz_xml(&xml).unwrap();
        assert_eq!(name, "test.img");
        assert_eq!(imported.len(), props.len());

        // Verify info/islot
        if let WzProperty::SubProperty { properties } = &imported[0].1 {
            assert!(matches!(&properties[0].1, WzProperty::String(s) if s == "Bd"));
        } else {
            panic!("Expected SubProperty");
        }
        assert!(matches!(&imported[1].1, WzProperty::Int(120)));
        assert!(matches!(
            &imported[2].1,
            WzProperty::Vector { x: -36, y: 100 }
        ));
        assert!(matches!(&imported[3].1, WzProperty::Short(1)));
    }

    #[test]
    fn test_roundtrip_client_mode_canvas() {
        use crate::image::encode as image_encode;

        // Build a valid 2×1 Bgra8888 canvas: 2 pixels × 4 bytes, zlib-compressed.
        let rgba = vec![255u8, 0, 0, 255, 0, 255, 0, 255]; // red, green
        let wz_pixels = image_encode::encode_pixels(&rgba, 2, 1, WzPngFormat::Bgra8888).unwrap();
        let png_data = image_encode::compress_png_data(&wz_pixels).unwrap();

        let props = vec![(
            "img".to_string(),
            WzProperty::Canvas {
                width: 2,
                height: 1,
                format: WzPngFormat::Bgra8888,
                scale: 0,
                properties: vec![("origin".to_string(), WzProperty::Vector { x: 5, y: 10 })],
                png_data,
            },
        )];
        let xml = export_wz_xml("test.img", &props, &XmlMode::WithBinaryData);
        // No format attribute — HaRepacker-compatible
        assert!(!xml.contains("format="));
        assert!(xml.contains("basedata="));

        // basedata is a standard PNG (magic bytes: 89 50 4E 47)
        let b64_start = xml.find("basedata=\"").unwrap() + 10;
        let b64_end = xml[b64_start..].find('"').unwrap() + b64_start;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&xml[b64_start..b64_end])
            .unwrap();
        assert_eq!(&decoded[..4], b"\x89PNG", "basedata must be a standard PNG");

        // Roundtrip: import → Canvas preserved
        let (_, imported) = import_wz_xml(&xml).unwrap();
        if let WzProperty::Canvas {
            width,
            height,
            format,
            properties,
            png_data: got_png,
            ..
        } = &imported[0].1
        {
            assert_eq!(*width, 2);
            assert_eq!(*height, 1);
            // After roundtrip, stored as Bgra8888 (canonical import format)
            assert_eq!(*format, WzPngFormat::Bgra8888);
            // png_data is non-empty and zlib-compressed
            assert!(!got_png.is_empty());
            assert_eq!(got_png[0], 0x78, "roundtrip png_data should be zlib");
            assert_eq!(properties.len(), 1);
        } else {
            panic!("Expected Canvas");
        }
    }
}

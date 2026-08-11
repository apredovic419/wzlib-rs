//! WZ image parsing — reads property trees from IMG data blocks.
//!
//! A WZ image is a block of data containing a tree of typed properties.
//! The first byte determines the image format:
//! - 0x73: "Property" image (standard)
//! - 0x1B: Offset-based new format
//! - 0x01: Lua property

use std::io::{Read, Seek};

use super::binary_reader::WzBinaryReader;
use super::error::{WzError, WzResult};
use super::keys::WzKey;
use super::properties::{CanvasData, WzProperty};
use super::types::WzPngFormat;
use crate::crypto::{WZ_BMSCLASSIC_IV, WZ_GMSIV, WZ_MSEAIV};

const KNOWN_IVS: [[u8; 4]; 3] = [WZ_BMSCLASSIC_IV, WZ_GMSIV, WZ_MSEAIV];

// Image may use a different encryption key than the directory —
// try all known IVs (common with JMS/KMS/CMS files).
fn try_iv_fallback<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    offset: u64,
    read_prop_string: impl Fn(&mut WzBinaryReader<R>) -> Result<String, WzError>,
) -> WzResult<Vec<(String, WzProperty)>> {
    for &iv in &KNOWN_IVS {
        reader.wz_key = WzKey::new(iv);
        if let Ok(s) = read_prop_string(reader) {
            if s == "Property" {
                return parse_property_list(reader, offset);
            }
        }
    }
    Err(WzError::InvalidImageHeader(0))
}

pub fn parse_image<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
) -> WzResult<Vec<(String, WzProperty)>> {
    let offset = reader.position()?; // used for string block resolution (C#'s WzImage.offset)
    let header_byte = reader.read_u8()?;

    match header_byte {
        0x73 => {
            let pos_after_header = reader.position()?;
            let prop_str = reader.read_wz_string()?;
            let val = reader.read_u16()?;
            if prop_str == "Property" && val == 0 {
                return parse_property_list(reader, offset);
            }

            try_iv_fallback(reader, offset, |r| {
                r.seek(pos_after_header)?;
                let s = r.read_wz_string()?;
                let v = r.read_u16()?;
                if v == 0 {
                    Ok(s)
                } else {
                    Err(WzError::InvalidImageHeader(0x73))
                }
            })
        }
        0x1B => {
            let str_offset = reader.read_i32()?;
            let string_pos = offset.wrapping_add(str_offset as i64 as u64);
            let prop_str = reader.read_string_at_offset(string_pos)?;
            let val = reader.read_u16()?;
            if prop_str == "Property" && val == 0 {
                return parse_property_list(reader, offset);
            }

            if val != 0 {
                return Err(WzError::InvalidImageHeader(header_byte));
            }
            try_iv_fallback(reader, offset, |r| r.read_string_at_offset(string_pos))
        }
        0x01 => {
            let data = read_lua_data(reader)?;
            Ok(vec![("Script".to_string(), WzProperty::Lua(data))])
        }
        other => Err(WzError::InvalidImageHeader(other)),
    }
}

/// Like [`parse_image`], but Canvas `png_data` is stored as a zero-copy
/// [`CanvasData::Ref`] into `src` rather than copied. `src` must be the exact
/// byte buffer `reader` reads from (index 0 = reader position 0) and must outlive
/// the returned property tree (the `Arc` co-owns it, so this is automatic).
///
/// Use this when parsing large IMGs where most Canvas frames are never decoded:
/// it avoids duplicating every frame's compressed pixels into the tree.
pub fn parse_image_lazy<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    src: std::sync::Arc<[u8]>,
) -> WzResult<Vec<(String, WzProperty)>> {
    reader.lazy_canvas_src = Some(src);
    let result = parse_image(reader);
    reader.lazy_canvas_src = None;
    result
}

/// Parse only the single property at `path` within an IMG, **skipping sibling
/// subtrees** instead of materializing the whole tree.
///
/// Each non-matching extended (`0x09`) sibling is skipped by reading its
/// block-size prefix and seeking past it, so reaching a deep node in a large
/// IMG parses only the nodes along `path` (plus a few seeks), not the entire
/// tree. Canvas `png_data` on the matched node is stored lazily as
/// [`CanvasData::Ref`] into `src`, same contract as [`parse_image_lazy`].
///
/// Returns `Ok(None)` when `path` is absent (a sibling name never matched, or a
/// non-final component was a scalar/non-`Property` and so can't be descended).
/// `path` must be non-empty. Useful for extracting one property from a large
/// IMG that contains many entries (e.g. one item's `info` out of a packed
/// bucket file); for reading a whole tree use [`parse_image`] /
/// [`parse_image_lazy`].
pub fn parse_image_path_lazy<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    src: std::sync::Arc<[u8]>,
    path: &[&str],
) -> WzResult<Option<WzProperty>> {
    reader.lazy_canvas_src = Some(src);
    let result = parse_image_path(reader, path);
    reader.lazy_canvas_src = None;
    result
}

fn parse_image_path<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    path: &[&str],
) -> WzResult<Option<WzProperty>> {
    if path.is_empty() {
        return Err(WzError::Custom("parse_image_path: empty path".into()));
    }
    let offset = reader.position()?;
    let header_byte = reader.read_u8()?;
    match header_byte {
        0x73 => {
            let pos_after_header = reader.position()?;
            let prop_str = reader.read_wz_string()?;
            let val = reader.read_u16()?;
            if prop_str == "Property" && val == 0 {
                return parse_property_list_path(reader, offset, path);
            }
            // Image encrypted with a different IV than the directory — retry
            // the known IVs (mirrors `parse_image`'s `try_iv_fallback`), then
            // traverse the path with the matching key.
            for &iv in &KNOWN_IVS {
                reader.wz_key = WzKey::new(iv);
                reader.seek(pos_after_header)?;
                if let Ok(s) = reader.read_wz_string() {
                    if s == "Property" && reader.read_u16()? == 0 {
                        return parse_property_list_path(reader, offset, path);
                    }
                }
            }
            Err(WzError::InvalidImageHeader(0x73))
        }
        0x1B => {
            let str_offset = reader.read_i32()?;
            let string_pos = offset.wrapping_add(str_offset as i64 as u64);
            let prop_str = reader.read_string_at_offset(string_pos)?;
            let val = reader.read_u16()?;
            if prop_str == "Property" && val == 0 {
                return parse_property_list_path(reader, offset, path);
            }
            Err(WzError::InvalidImageHeader(0x1B))
        }
        // 0x01 (Lua) has no addressable property path.
        other => Err(WzError::InvalidImageHeader(other)),
    }
}

/// Walk one property list looking for `path[0]`. On a match: if it is the
/// final component, parse and return its value; otherwise descend into it
/// (must be a `Property` container) and recurse on `path[1..]`. Non-matching
/// children are skipped by [`skip_property_value`]. Stops as soon as the
/// matched child is handled — later siblings are never read.
fn parse_property_list_path<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    offset: u64,
    path: &[&str],
) -> WzResult<Option<WzProperty>> {
    let count = reader.read_compressed_int()?;
    if !(0..=super::MAX_PROPERTY_COUNT).contains(&count) {
        return Err(WzError::Custom(format!(
            "Invalid property count: {}",
            count
        )));
    }
    let want = path[0];
    for _ in 0..count {
        let name = reader.read_string_block(offset)?;
        if name == want {
            if path.len() == 1 {
                return parse_property_value(reader, offset);
            }
            return descend_into_property(reader, offset, &path[1..]);
        }
        skip_property_value(reader, offset)?;
    }
    Ok(None)
}

/// Descend into the just-matched child (whose value bytes start at the reader)
/// when more path remains. The child must be a `0x09` extended `Property`
/// container; anything else means the path can't continue → `Ok(None)`.
fn descend_into_property<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    offset: u64,
    rest: &[&str],
) -> WzResult<Option<WzProperty>> {
    if reader.read_u8()? != 0x09 {
        return Ok(None); // a scalar/string — cannot descend by name
    }
    let _block_size = reader.read_u32()?; // we read within the block, not past it
    let type_byte = reader.read_u8()?;
    let type_str = match type_byte {
        0x01 | 0x1B => {
            let str_offset = reader.read_i32()?;
            reader.read_string_at_offset(offset.wrapping_add(str_offset as i64 as u64))?
        }
        0x00 | 0x73 => reader.read_wz_string()?,
        _ => return Ok(None),
    };
    if type_str.as_str() != super::WZ_TYPE_PROPERTY {
        return Ok(None); // e.g. Canvas/Convex — no named children to descend
    }
    let _padding = reader.read_u16()?;
    parse_property_list_path(reader, offset, rest)
}

/// Advance the reader past one property value without building it. Mirrors the
/// cursor movement of [`parse_property_value`]; the win is the `0x09` arm,
/// which seeks past the whole extended block (a sibling subtree — another
/// item, an animation frame) using its block-size prefix instead of recursing.
fn skip_property_value<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    offset: u64,
) -> WzResult<()> {
    let prop_type = reader.read_u8()?;
    match prop_type {
        0x00 => {}
        0x02 | 0x0B => {
            reader.read_i16()?;
        }
        0x03 | 0x13 => {
            reader.read_compressed_int()?;
        }
        0x14 => {
            reader.read_compressed_long()?;
        }
        0x04 => {
            if reader.read_u8()? == 0x80 {
                reader.read_f32()?;
            }
        }
        0x05 => {
            reader.read_f64()?;
        }
        0x08 => {
            reader.read_string_block(offset)?;
        }
        0x09 => {
            let block_size = reader.read_u32()?;
            let end_of_block = reader.position()? + block_size as u64;
            reader.seek(end_of_block)?;
        }
        other => return Err(WzError::UnknownPropertyType(format!("0x{:02X}", other))),
    }
    Ok(())
}

pub fn parse_property_list<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    offset: u64,
) -> WzResult<Vec<(String, WzProperty)>> {
    let count = reader.read_compressed_int()?;
    if !(0..=super::MAX_PROPERTY_COUNT).contains(&count) {
        return Err(WzError::Custom(format!(
            "Invalid property count: {}",
            count
        )));
    }
    let mut properties = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let name = reader.read_string_block(offset)?;
        if let Some(prop) = parse_property_value(reader, offset)? {
            properties.push((name, prop));
        }
        // C# silently drops the property for unknown float indicator bytes,
        // so we skip it here when parse_property_value returns None.
    }

    Ok(properties)
}

// Returns `None` for properties C# silently drops (e.g. unknown float indicators).
fn parse_property_value<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    offset: u64,
) -> WzResult<Option<WzProperty>> {
    let prop_type = reader.read_u8()?;

    match prop_type {
        0x00 => Ok(Some(WzProperty::Null)),

        0x02 | 0x0B => {
            let val = reader.read_i16()?;
            Ok(Some(WzProperty::Short(val)))
        }

        0x03 | 0x13 => {
            let val = reader.read_compressed_int()?;
            Ok(Some(WzProperty::Int(val)))
        }

        0x14 => {
            let val = reader.read_compressed_long()?;
            Ok(Some(WzProperty::Long(val)))
        }

        0x04 => {
            let indicator = reader.read_u8()?;
            match indicator {
                0x80 => Ok(Some(WzProperty::Float(reader.read_f32()?))),
                0x00 => Ok(Some(WzProperty::Float(0.0))),
                // C# silently drops the property for unknown indicator bytes
                // (the `break` exits the case without calling properties.Add).
                _ => Ok(None),
            }
        }

        0x05 => {
            let val = reader.read_f64()?;
            Ok(Some(WzProperty::Double(val)))
        }

        0x08 => {
            let val = reader.read_string_block(offset)?;
            Ok(Some(WzProperty::String(val)))
        }

        0x09 => {
            let block_size = reader.read_u32()?;
            let end_of_block = reader.position()? + block_size as u64;
            let result = parse_extended_property(reader, offset)?;
            if reader.position()? != end_of_block {
                reader.seek(end_of_block)?;
            }
            Ok(Some(result))
        }

        other => Err(WzError::UnknownPropertyType(format!("0x{:02X}", other))),
    }
}

fn parse_extended_property<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    offset: u64,
) -> WzResult<WzProperty> {
    let type_byte = reader.read_u8()?;
    let type_str = match type_byte {
        0x01 | 0x1B => {
            let str_offset = reader.read_i32()?;
            reader.read_string_at_offset(offset.wrapping_add(str_offset as i64 as u64))?
        }
        0x00 | 0x73 => reader.read_wz_string()?,
        _ => {
            return Err(WzError::Custom(format!(
                "Invalid extended prop type byte: 0x{:02X}",
                type_byte
            )));
        }
    };

    use super::{
        WZ_TYPE_CANVAS, WZ_TYPE_CONVEX, WZ_TYPE_PROPERTY, WZ_TYPE_RAW_DATA, WZ_TYPE_SOUND,
        WZ_TYPE_UOL, WZ_TYPE_VECTOR, WZ_TYPE_VIDEO,
    };
    match type_str.as_str() {
        WZ_TYPE_PROPERTY => {
            let _padding = reader.read_u16()?;
            let properties = parse_property_list(reader, offset)?;
            Ok(WzProperty::SubProperty { properties })
        }

        WZ_TYPE_CANVAS => parse_canvas_property(reader, offset),

        WZ_TYPE_VECTOR => {
            let x = reader.read_compressed_int()?;
            let y = reader.read_compressed_int()?;
            Ok(WzProperty::Vector { x, y })
        }

        WZ_TYPE_CONVEX => {
            let count = reader.read_compressed_int()?;
            if !(0..=super::MAX_CONVEX_POINTS).contains(&count) {
                return Err(WzError::Custom(format!(
                    "Invalid convex point count: {}",
                    count
                )));
            }
            let mut points = Vec::with_capacity(count as usize);
            for i in 0..count {
                points.push((i.to_string(), parse_extended_property(reader, offset)?));
            }
            Ok(WzProperty::Convex { points })
        }

        WZ_TYPE_SOUND => parse_sound_property(reader),

        WZ_TYPE_UOL => {
            let _skip = reader.read_u8()?;
            let uol_type = reader.read_u8()?;
            let path = match uol_type {
                0x00 => reader.read_wz_string()?,
                0x01 => {
                    let str_offset = reader.read_i32()?;
                    reader.read_string_at_offset(offset.wrapping_add(str_offset as i64 as u64))?
                }
                other => {
                    return Err(WzError::Custom(format!(
                        "Unsupported UOL type: 0x{:02X}",
                        other
                    )));
                }
            };
            Ok(WzProperty::Uol(path))
        }

        WZ_TYPE_RAW_DATA => {
            let raw_type = reader.read_u8()?;
            let properties = if raw_type == 1 {
                let has_props = reader.read_u8()?;
                if has_props == 1 {
                    let _padding = reader.read_u16()?;
                    parse_property_list(reader, offset)?
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            let len = reader.read_compressed_int()? as usize;
            let data = reader.read_bytes(len)?;
            Ok(WzProperty::RawData {
                raw_type,
                properties,
                data,
            })
        }

        WZ_TYPE_VIDEO => {
            let _skip = reader.read_u8()?;
            let has_props = reader.read_u8()?;
            let properties = read_optional_properties(reader, offset, has_props)?;
            let video_type = reader.read_u8()?;
            let data_len = reader.read_compressed_int()?;
            let data_offset = reader.position()?;
            let video_data = reader.read_bytes(data_len as usize)?;

            let mcv_header = if video_data.len() >= 36 {
                super::mcv::parse_mcv_header(&video_data[..36]).ok()
            } else {
                None
            };

            Ok(WzProperty::Video {
                video_type,
                properties,
                data_offset,
                data_length: data_len as u32,
                mcv_header,
                video_data: Some(video_data),
            })
        }

        other => Ok(WzProperty::String(other.to_string())),
    }
}

fn read_optional_properties<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    offset: u64,
    flag: u8,
) -> WzResult<Vec<(String, WzProperty)>> {
    if flag == 1 {
        let _padding = reader.read_u16()?;
        parse_property_list(reader, offset)
    } else {
        Ok(Vec::new())
    }
}

fn parse_canvas_property<R: Read + Seek>(
    reader: &mut WzBinaryReader<R>,
    offset: u64,
) -> WzResult<WzProperty> {
    let _skip = reader.read_u8()?;
    let has_children = reader.read_u8()?;
    let properties = read_optional_properties(reader, offset, has_children)?;

    let width = reader.read_compressed_int()?;
    let height = reader.read_compressed_int()?;
    let format_low = reader.read_compressed_int()?;
    let format_high = reader.read_compressed_int()?;
    let _zero = reader.read_i32()?; // Always 0

    let raw_data_len = reader.read_i32()?;
    let _header_byte = reader.read_u8()?; // 0x00

    if raw_data_len <= 1 {
        return Err(WzError::Custom(format!(
            "Invalid PNG data length: {}",
            raw_data_len
        )));
    }
    let data_len = (raw_data_len - 1) as usize;
    // Lazy mode: record an offset/len into the shared source buffer and skip the
    // bytes, instead of copying them into the property tree.
    let png_data = match reader.lazy_canvas_src.clone() {
        Some(src) => {
            let offset = reader.position()? as usize;
            if offset + data_len > src.len() {
                return Err(WzError::Custom(format!(
                    "Canvas data range {}..{} exceeds source buffer ({} bytes)",
                    offset,
                    offset + data_len,
                    src.len()
                )));
            }
            reader.seek((offset + data_len) as u64)?;
            CanvasData::Ref {
                src,
                offset,
                len: data_len,
            }
        }
        None => CanvasData::Loaded(reader.read_bytes(data_len)?),
    };

    // The first compressed int (`format_low`) is the full pixel-codec id; the
    // second (`format_high`) is a scale exponent (`format2`), not part of the
    // codec. Image data is stored at `(w >> scale) × (h >> scale)` and is
    // upscaled by `decode_canvas_pixels`. Folding the two together (the old
    // `low + (high << 8)`) silently corrupted scaled canvases.
    let format = WzPngFormat::from_combined(format_low.max(0) as u32);
    let scale = format_high.clamp(0, u8::MAX as i32) as u8;

    Ok(WzProperty::Canvas {
        width,
        height,
        format,
        scale,
        properties,
        png_data,
    })
}

const SOUND_HEADER_LEN: usize = 51; // C#'s `soundHeader` GUIDs
const WAVE_FORMAT_SIZE: usize = 18; // WAVEFORMATEX base (no extra data)

// Validates WAVEFORMATEX size; if invalid, tries XOR decryption with WzKey.
fn try_decrypt_wave_format(wav_header: &mut [u8], wz_key: &[u8]) -> bool {
    if wav_header.len() < WAVE_FORMAT_SIZE {
        return false;
    }

    let extra_size = u16::from_le_bytes([wav_header[16], wav_header[17]]) as usize;
    if WAVE_FORMAT_SIZE + extra_size == wav_header.len() {
        return false;
    }

    for i in 0..wav_header.len() {
        if i < wz_key.len() {
            wav_header[i] ^= wz_key[i];
        }
    }

    let extra_size = u16::from_le_bytes([wav_header[16], wav_header[17]]) as usize;
    WAVE_FORMAT_SIZE + extra_size == wav_header.len()
}

fn parse_sound_property<R: Read + Seek>(reader: &mut WzBinaryReader<R>) -> WzResult<WzProperty> {
    let _padding = reader.read_u8()?;
    let sound_data_len = reader.read_compressed_int()?;
    let duration = reader.read_compressed_int()?;

    let header_off = reader.position()?;
    reader.seek(header_off + SOUND_HEADER_LEN as u64)?;
    let wav_format_len = reader.read_u8()? as usize;

    reader.seek(header_off)?;
    let sound_header_bytes = reader.read_bytes(SOUND_HEADER_LEN)?;
    let unk1 = reader.read_bytes(1)?;
    let mut wav_format_bytes = reader.read_bytes(wav_format_len)?;

    let key_slice = reader.wz_key.get_slice(0, wav_format_len.max(1));
    try_decrypt_wave_format(&mut wav_format_bytes, key_slice);

    let mut header = Vec::with_capacity(SOUND_HEADER_LEN + 1 + wav_format_len);
    header.extend_from_slice(&sound_header_bytes);
    header.extend_from_slice(&unk1);
    header.extend_from_slice(&wav_format_bytes);

    let audio_data = reader.read_bytes(sound_data_len as usize)?;

    Ok(WzProperty::Sound {
        duration_ms: duration,
        data: audio_data,
        header,
    })
}

fn read_lua_data<R: Read + Seek>(reader: &mut WzBinaryReader<R>) -> WzResult<Vec<u8>> {
    let len = reader.read_compressed_int()? as usize;
    reader.read_bytes(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wz::test_utils::*;

    /// Encode an ASCII string with a specific IV's key (for testing IV fallback).
    fn encode_ascii_with_iv(s: &str, iv: [u8; 4]) -> Vec<u8> {
        let len = s.len();
        assert!(len > 0 && len < 128);
        let mut key = WzKey::new(iv);
        key.ensure_size(len);
        let indicator = -(len as i8);
        let mut out = vec![indicator as u8];
        let mut mask: u8 = 0xAA;
        for (i, b) in s.bytes().enumerate() {
            out.push(b ^ mask ^ key[i]);
            mask = mask.wrapping_add(1);
        }
        out
    }

    /// Build a 0x73 Property header encrypted with the given IV.
    fn property_image_header_with_iv(iv: [u8; 4]) -> Vec<u8> {
        let mut out = vec![0x73u8];
        out.extend_from_slice(&encode_ascii_with_iv("Property", iv));
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    // ── Header dispatch ────────────────────────────────────────────

    #[test]
    fn test_parse_image_0x73_iv_fallback() {
        // Image encrypted with GMS key, but reader starts with BMS (zero) key
        let mut data = property_image_header_with_iv(WZ_GMSIV);
        data.push(0); // count = 0

        let mut reader = make_reader(data);
        // reader was constructed with [0;4] (BMS), but image uses GMS key
        let props = parse_image(&mut reader).unwrap();
        assert!(props.is_empty());
    }

    #[test]
    fn test_parse_image_0x73_iv_fallback_ems() {
        // Image encrypted with EMS key, reader starts with BMS key
        let mut data = property_image_header_with_iv(WZ_MSEAIV);
        data.push(0);

        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert!(props.is_empty());
    }

    #[test]
    fn test_parse_image_0x73_empty_property_list() {
        let mut data = property_image_header();
        data.push(0); // count = 0
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert!(props.is_empty());
    }

    #[test]
    fn test_parse_image_invalid_header() {
        let data = vec![0xFF];
        let mut reader = make_reader(data);
        let err = parse_image(&mut reader).unwrap_err();
        matches!(err, WzError::InvalidImageHeader(0xFF));
    }

    #[test]
    fn test_parse_image_lua() {
        // Header 0x01 → Lua: compressed_int(len) + bytes
        let lua_bytes = b"print('hello')";
        let mut data = vec![0x01u8];
        data.push(lua_bytes.len() as u8); // compressed int = len
        data.extend_from_slice(lua_bytes);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].0, "Script");
        if let WzProperty::Lua(ref d) = props[0].1 {
            assert_eq!(d, lua_bytes);
        } else {
            panic!("Expected Lua property");
        }
    }

    // ── Null property (marker 0x00) ────────────────────────────────

    #[test]
    fn test_parse_null_property() {
        let data = build_image_with_property("n", &[0x00]);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].0, "n");
        assert!(matches!(props[0].1, WzProperty::Null));
    }

    // ── Short property (marker 0x02) ───────────────────────────────

    #[test]
    fn test_parse_short_property() {
        let mut value = vec![0x02u8];
        value.extend_from_slice(&42i16.to_le_bytes());
        let data = build_image_with_property("s", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert_eq!(props[0].1.as_int(), Some(42));
    }

    // ── Int property (marker 0x03) ─────────────────────────────────

    #[test]
    fn test_parse_int_property_small() {
        // Compressed int: indicator=99 → value=99
        let value = vec![0x03u8, 99];
        let data = build_image_with_property("i", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert_eq!(props[0].1.as_int(), Some(99));
    }

    #[test]
    fn test_parse_int_property_large() {
        // Compressed int: indicator=0x80 + i32
        let mut value = vec![0x03u8, 0x80];
        value.extend_from_slice(&100_000i32.to_le_bytes());
        let data = build_image_with_property("i", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert_eq!(props[0].1.as_int(), Some(100_000));
    }

    // ── Long property (marker 0x14) ────────────────────────────────

    #[test]
    fn test_parse_long_property() {
        let mut value = vec![0x14u8, 0x80]; // indicator -128 → read i64
        value.extend_from_slice(&9_999_999i64.to_le_bytes());
        let data = build_image_with_property("l", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert_eq!(props[0].1.as_int(), Some(9_999_999));
    }

    // ── Float property (marker 0x04) ───────────────────────────────

    #[test]
    fn test_parse_float_property_value() {
        let mut value = vec![0x04u8, 0x80]; // indicator 0x80 → read f32
        value.extend_from_slice(&1.5f32.to_le_bytes());
        let data = build_image_with_property("f", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        let v = props[0].1.as_float().unwrap();
        assert!((v - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_float_property_zero() {
        let value = vec![0x04u8, 0x00]; // indicator 0x00 → Float(0.0)
        let data = build_image_with_property("f", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert_eq!(props[0].1.as_float(), Some(0.0));
    }

    #[test]
    fn test_parse_float_property_unknown_indicator_skipped() {
        // indicator 0x42 → property silently dropped (returns None)
        let value = vec![0x04u8, 0x42];
        let data = build_image_with_property("f", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        // Property was skipped, so it should not appear
        assert!(props.is_empty());
    }

    // ── Double property (marker 0x05) ──────────────────────────────

    #[test]
    fn test_parse_double_property() {
        let mut value = vec![0x05u8];
        value.extend_from_slice(&3.14f64.to_le_bytes());
        let data = build_image_with_property("d", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        let v = props[0].1.as_float().unwrap();
        assert!((v - 3.14).abs() < f64::EPSILON);
    }

    // ── String property (marker 0x08) ──────────────────────────────

    #[test]
    fn test_parse_string_property() {
        let mut value = vec![0x08u8];
        value.extend_from_slice(&string_block("hello"));
        let data = build_image_with_property("str", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert_eq!(props[0].1.as_str(), Some("hello"));
    }

    // ── Vector extended property (marker 0x09) ─────────────────────

    #[test]
    fn test_parse_vector_property() {
        // Extended: block_size(u32) + type_byte(0x73) + "Shape2D#Vector2D" string + x + y
        let mut inner = vec![0x73u8]; // inline type name
        inner.extend_from_slice(&encode_wz_ascii("Shape2D#Vector2D"));
        inner.push(10); // x = 10 (compressed int)
        inner.push(20); // y = 20 (compressed int)

        let mut value = vec![0x09u8];
        value.extend_from_slice(&(inner.len() as u32).to_le_bytes()); // block_size
        value.extend_from_slice(&inner);

        let data = build_image_with_property("v", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        if let WzProperty::Vector { x, y } = &props[0].1 {
            assert_eq!(*x, 10);
            assert_eq!(*y, 20);
        } else {
            panic!("Expected Vector, got {:?}", props[0].1);
        }
    }

    // ── Multiple properties ────────────────────────────────────────

    #[test]
    fn test_parse_multiple_properties() {
        let mut data = property_image_header();
        data.push(3); // count = 3

        // Property 1: "a" = Null
        data.extend_from_slice(&string_block("a"));
        data.push(0x00);

        // Property 2: "b" = Short(7)
        data.extend_from_slice(&string_block("b"));
        data.push(0x02);
        data.extend_from_slice(&7i16.to_le_bytes());

        // Property 3: "c" = Int(42)
        data.extend_from_slice(&string_block("c"));
        data.push(0x03);
        data.push(42); // compressed int = 42

        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert_eq!(props.len(), 3);
        assert_eq!(props[0].0, "a");
        assert!(matches!(props[0].1, WzProperty::Null));
        assert_eq!(props[1].0, "b");
        assert_eq!(props[1].1.as_int(), Some(7));
        assert_eq!(props[2].0, "c");
        assert_eq!(props[2].1.as_int(), Some(42));
    }

    // ── Path (partial) parse — parse_image_path_lazy ───────────────

    /// A small compressed-int property value (`|v| < 128`).
    fn int_value(v: i32) -> Vec<u8> {
        vec![0x03u8, v as i8 as u8]
    }

    /// A `Property` SubProperty value from a child name→value list.
    fn subproperty(children: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut content = vec![0u8, 0u8]; // u16 padding read by WZ_TYPE_PROPERTY
        content.push(children.len() as u8); // count (compressed int, small)
        for (name, value) in children {
            content.extend_from_slice(&string_block(name));
            content.extend_from_slice(value);
        }
        build_extended_property("Property", &content)
    }

    /// Root tree:  first=Int(1), target={ junk={x:100}, info={icon:7, price:99} }, last=Int(2).
    /// `target` is reached by skipping the scalar `first`; `info` is reached by
    /// skipping the 0x09 SubProperty `junk` via its block-size — exercising the
    /// sibling-skip seek path.
    fn nested_path_image() -> Vec<u8> {
        let mut data = property_image_header();
        data.push(3); // count
        data.extend_from_slice(&string_block("first"));
        data.extend_from_slice(&int_value(1));
        data.extend_from_slice(&string_block("target"));
        data.extend_from_slice(&subproperty(&[
            ("junk", subproperty(&[("x", int_value(100))])),
            (
                "info",
                subproperty(&[("icon", int_value(7)), ("price", int_value(99))]),
            ),
        ]));
        data.extend_from_slice(&string_block("last"));
        data.extend_from_slice(&int_value(2));
        data
    }

    #[test]
    fn parse_image_path_reaches_nested_leaf_skipping_siblings() {
        let data = nested_path_image();
        let src: std::sync::Arc<[u8]> = std::sync::Arc::from(data.clone().into_boxed_slice());

        let mut reader = make_reader(data.clone());
        let node =
            parse_image_path_lazy(&mut reader, src.clone(), &["target", "info", "icon"]).unwrap();
        assert!(
            matches!(node, Some(WzProperty::Int(7))),
            "must reach the leaf past the scalar `first` + the 0x09 `junk` sibling, got {node:?}"
        );

        // A non-final match returns the whole matched SubProperty.
        let mut reader = make_reader(data.clone());
        let info = parse_image_path_lazy(&mut reader, src.clone(), &["target", "info"])
            .unwrap()
            .expect("info present");
        match info {
            WzProperty::SubProperty { properties } => {
                assert_eq!(properties.len(), 2, "info has icon + price");
                assert_eq!(properties[0].0, "icon");
            }
            other => panic!("expected SubProperty, got {other:?}"),
        }
    }

    #[test]
    fn parse_image_path_returns_none_for_absent_paths() {
        let data = nested_path_image();
        let src: std::sync::Arc<[u8]> = std::sync::Arc::from(data.clone().into_boxed_slice());
        for path in [
            &["target", "info", "missing"][..],
            &["target", "nope"][..],
            &["nope"][..],
            &["first", "x"][..], // can't descend into a scalar
        ] {
            let mut reader = make_reader(data.clone());
            assert!(
                parse_image_path_lazy(&mut reader, src.clone(), path)
                    .unwrap()
                    .is_none(),
                "absent path {path:?} must be None"
            );
        }
    }

    #[test]
    fn parse_image_path_matches_full_parse_at_that_path() {
        // The partial result must equal what a full parse holds at the path.
        let data = nested_path_image();
        let src: std::sync::Arc<[u8]> = std::sync::Arc::from(data.clone().into_boxed_slice());

        let mut reader = make_reader(data.clone());
        let full = parse_image(&mut reader).unwrap();
        let target = &full.iter().find(|(n, _)| n == "target").unwrap().1;
        let info = target
            .children()
            .unwrap()
            .iter()
            .find(|(n, _)| n == "info")
            .unwrap();
        let icon_full = info
            .1
            .children()
            .unwrap()
            .iter()
            .find(|(n, _)| n == "icon")
            .unwrap();
        assert_eq!(icon_full.1.as_int(), Some(7));

        let mut reader = make_reader(data.clone());
        let icon_partial =
            parse_image_path_lazy(&mut reader, src, &["target", "info", "icon"]).unwrap();
        assert_eq!(icon_partial.and_then(|p| p.as_int()), icon_full.1.as_int());
    }

    // ── Canvas property ───────────────────────────────────────────

    #[test]
    fn test_parse_canvas_property_no_children() {
        let png_payload = vec![0xAA, 0xBB, 0xCC]; // 3 bytes of fake PNG data
        let raw_data_len: i32 = png_payload.len() as i32 + 1; // +1 for header byte

        let mut inner = Vec::new();
        inner.push(0x00); // _skip byte
        inner.push(0x00); // has_children = 0 (no sub-properties)
        inner.push(4); // width = 4 (compressed int)
        inner.push(8); // height = 8 (compressed int)
        inner.push(2); // format_low = 2 → Bgra8888 (compressed int)
        inner.push(0); // format_high = 0 (compressed int)
        inner.extend_from_slice(&0i32.to_le_bytes()); // _zero
        inner.extend_from_slice(&raw_data_len.to_le_bytes()); // raw_data_len
        inner.push(0x00); // header byte
        inner.extend_from_slice(&png_payload);

        let value = build_extended_property("Canvas", &inner);
        let data = build_image_with_property("img", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();

        assert_eq!(props.len(), 1);
        assert_eq!(props[0].0, "img");
        if let WzProperty::Canvas {
            width,
            height,
            format,
            properties,
            png_data,
            ..
        } = &props[0].1
        {
            assert_eq!(*width, 4);
            assert_eq!(*height, 8);
            assert_eq!(*format, WzPngFormat::Bgra8888);
            assert!(properties.is_empty());
            assert_eq!(png_data.as_bytes(), &png_payload[..]);
        } else {
            panic!("Expected Canvas, got {:?}", props[0].1);
        }
    }

    #[test]
    fn test_parse_canvas_property_with_children() {
        let png_payload = vec![0xDD, 0xEE];
        let raw_data_len: i32 = png_payload.len() as i32 + 1;

        let mut inner = Vec::new();
        inner.push(0x00); // _skip byte
        inner.push(0x01); // has_children = 1
        inner.extend_from_slice(&0u16.to_le_bytes()); // _padding
                                                      // Child property list: count=1, name="delay", type=0x03(Int), value=100
        inner.push(1); // count
        inner.extend_from_slice(&string_block("delay"));
        inner.push(0x03); // Int marker
        inner.push(100); // compressed int = 100
                         // PNG fields
        inner.push(16); // width = 16
        inner.push(16); // height = 16
        inner.push(1); // format_low = 1 → Bgra4444
        inner.push(0); // format_high = 0
        inner.extend_from_slice(&0i32.to_le_bytes());
        inner.extend_from_slice(&raw_data_len.to_le_bytes());
        inner.push(0x00);
        inner.extend_from_slice(&png_payload);

        let value = build_extended_property("Canvas", &inner);
        let data = build_image_with_property("icon", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();

        if let WzProperty::Canvas {
            width,
            height,
            format,
            properties,
            ..
        } = &props[0].1
        {
            assert_eq!(*width, 16);
            assert_eq!(*height, 16);
            assert_eq!(*format, WzPngFormat::Bgra4444);
            assert_eq!(properties.len(), 1);
            assert_eq!(properties[0].0, "delay");
            assert_eq!(properties[0].1.as_int(), Some(100));
        } else {
            panic!("Expected Canvas");
        }
    }

    #[test]
    fn test_parse_canvas_lazy_zero_copy() {
        use std::sync::Arc;

        let png_payload = vec![0xAA, 0xBB, 0xCC, 0xDD];
        let raw_data_len: i32 = png_payload.len() as i32 + 1;

        let mut inner = Vec::new();
        inner.push(0x00); // _skip
        inner.push(0x00); // has_children = 0
        inner.push(4); // width
        inner.push(8); // height
        inner.push(2); // format_low → Bgra8888
        inner.push(0); // format_high
        inner.extend_from_slice(&0i32.to_le_bytes()); // _zero
        inner.extend_from_slice(&raw_data_len.to_le_bytes());
        inner.push(0x00); // header byte
        inner.extend_from_slice(&png_payload);

        let value = build_extended_property("Canvas", &inner);
        let data = build_image_with_property("img", &value);

        // Eager parse (baseline) and lazy parse must agree on the decoded bytes.
        let eager = parse_image(&mut make_reader(data.clone())).unwrap();
        let src: Arc<[u8]> = Arc::from(data.clone());
        let mut reader = make_reader(data);
        let lazy = parse_image_lazy(&mut reader, src.clone()).unwrap();

        let eager_png = match &eager[0].1 {
            WzProperty::Canvas { png_data, .. } => png_data,
            _ => panic!("Expected Canvas"),
        };
        match &lazy[0].1 {
            WzProperty::Canvas {
                png_data,
                width,
                height,
                ..
            } => {
                assert_eq!(*width, 4);
                assert_eq!(*height, 8);
                // Lazy must be a zero-copy Ref, not an owned copy.
                assert!(
                    matches!(png_data, CanvasData::Ref { .. }),
                    "lazy parse should produce CanvasData::Ref"
                );
                // The Ref must point at the same bytes within the shared buffer.
                assert_eq!(png_data.as_bytes(), &png_payload[..]);
                assert_eq!(png_data.as_bytes(), eager_png.as_bytes());
                if let CanvasData::Ref {
                    src: ref_src,
                    offset,
                    len,
                } = png_data
                {
                    assert!(
                        Arc::ptr_eq(ref_src, &src),
                        "Ref should share the source Arc"
                    );
                    assert_eq!(&src[*offset..*offset + *len], &png_payload[..]);
                }
            }
            _ => panic!("Expected Canvas"),
        }
    }

    #[test]
    fn test_parse_canvas_invalid_data_len() {
        let mut inner = Vec::new();
        inner.push(0x00); // _skip
        inner.push(0x00); // has_children = 0
        inner.push(1); // width
        inner.push(1); // height
        inner.push(2); // format_low
        inner.push(0); // format_high
        inner.extend_from_slice(&0i32.to_le_bytes());
        inner.extend_from_slice(&0i32.to_le_bytes()); // raw_data_len = 0 (invalid, must be > 1)
        inner.push(0x00);

        let value = build_extended_property("Canvas", &inner);
        let data = build_image_with_property("bad", &value);
        let mut reader = make_reader(data);
        let err = parse_image(&mut reader).unwrap_err();
        assert!(matches!(err, WzError::Custom(_)));
    }

    // ── Sound property ────────────────────────────────────────────

    #[test]
    fn test_parse_sound_property() {
        let audio_data = vec![0x01, 0x02, 0x03, 0x04]; // 4 bytes fake audio
        let sound_header = vec![0xAA; SOUND_HEADER_LEN]; // 51 bytes
        let wav_format_len: u8 = 4;
        let wav_format = vec![0xBB; wav_format_len as usize];

        let mut inner = Vec::new();
        inner.push(0x00); // _padding
        inner.push(audio_data.len() as u8); // sound_data_len (compressed int)
        inner.push(100); // duration = 100ms (compressed int)
                         // Data from header_off onward:
        inner.extend_from_slice(&sound_header); // 51 bytes
        inner.push(wav_format_len); // wav_format_len byte (also read as unk1)
        inner.extend_from_slice(&wav_format); // wav_format_len bytes
        inner.extend_from_slice(&audio_data); // sound_data_len bytes

        let value = build_extended_property("Sound_DX8", &inner);
        let data = build_image_with_property("snd", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();

        assert_eq!(props.len(), 1);
        assert_eq!(props[0].0, "snd");
        if let WzProperty::Sound {
            duration_ms,
            data,
            header,
            ..
        } = &props[0].1
        {
            assert_eq!(*duration_ms, 100);
            assert_eq!(data, &audio_data);
            // header = sound_header(51) + unk1(1) + wav_format(wav_format_len)
            assert_eq!(header.len(), SOUND_HEADER_LEN + 1 + wav_format_len as usize);
        } else {
            panic!("Expected Sound, got {:?}", props[0].1);
        }
    }

    #[test]
    fn test_parse_sound_property_zero_wav_format() {
        let audio_data = vec![0xFF; 2];
        let sound_header = vec![0x00; SOUND_HEADER_LEN];
        let wav_format_len: u8 = 0;

        let mut inner = Vec::new();
        inner.push(0x00); // _padding
        inner.push(audio_data.len() as u8);
        inner.push(50); // duration = 50ms
        inner.extend_from_slice(&sound_header);
        inner.push(wav_format_len); // unk1 / wav_format_len = 0
                                    // no wav_format bytes
        inner.extend_from_slice(&audio_data);

        let value = build_extended_property("Sound_DX8", &inner);
        let data = build_image_with_property("s2", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();

        if let WzProperty::Sound {
            duration_ms,
            data,
            header,
            ..
        } = &props[0].1
        {
            assert_eq!(*duration_ms, 50);
            assert_eq!(data, &audio_data);
            // header = 51 bytes + 1 byte unk1 + 0 wav_format bytes
            assert_eq!(header.len(), SOUND_HEADER_LEN + 1);
        } else {
            panic!("Expected Sound");
        }
    }

    // ── UOL property ──────────────────────────────────────────────

    #[test]
    fn test_parse_uol_property_inline() {
        let mut inner = Vec::new();
        inner.push(0x00); // _skip byte
        inner.push(0x00); // uol_type = 0x00 (inline WZ string)
        inner.extend_from_slice(&encode_wz_ascii("../stand/0"));

        let value = build_extended_property("UOL", &inner);
        let data = build_image_with_property("link", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();

        assert_eq!(props.len(), 1);
        assert_eq!(props[0].0, "link");
        if let WzProperty::Uol(path) = &props[0].1 {
            assert_eq!(path, "../stand/0");
        } else {
            panic!("Expected Uol, got {:?}", props[0].1);
        }
    }

    #[test]
    fn test_parse_uol_unsupported_type() {
        let mut inner = Vec::new();
        inner.push(0x00); // _skip
        inner.push(0x99); // unsupported uol_type

        let value = build_extended_property("UOL", &inner);
        let data = build_image_with_property("bad", &value);
        let mut reader = make_reader(data);
        let err = parse_image(&mut reader).unwrap_err();
        assert!(matches!(err, WzError::Custom(_)));
    }

    // ── Convex property ───────────────────────────────────────────

    #[test]
    fn test_parse_convex_property() {
        let mut inner = Vec::new();
        inner.push(2); // count = 2 points
                       // Point 1: extended Vector
        inner.push(0x73);
        inner.extend_from_slice(&encode_wz_ascii("Shape2D#Vector2D"));
        inner.push(1); // x = 1
        inner.push(2); // y = 2
                       // Point 2: extended Vector
        inner.push(0x73);
        inner.extend_from_slice(&encode_wz_ascii("Shape2D#Vector2D"));
        inner.push(3); // x = 3
        inner.push(4); // y = 4

        let value = build_extended_property("Shape2D#Convex2D", &inner);
        let data = build_image_with_property("cv", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();

        if let WzProperty::Convex { points } = &props[0].1 {
            assert_eq!(points.len(), 2);
            assert!(matches!(points[0].1, WzProperty::Vector { x: 1, y: 2 }));
            assert!(matches!(points[1].1, WzProperty::Vector { x: 3, y: 4 }));
        } else {
            panic!("Expected Convex");
        }
    }

    // ── SubProperty extended ──────────────────────────────────────

    #[test]
    fn test_parse_sub_property_extended() {
        let mut inner = Vec::new();
        inner.extend_from_slice(&0u16.to_le_bytes()); // _padding
        inner.push(1); // count = 1
        inner.extend_from_slice(&string_block("val"));
        inner.push(0x00); // Null property

        let value = build_extended_property("Property", &inner);
        let data = build_image_with_property("sub", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();

        if let WzProperty::SubProperty { properties, .. } = &props[0].1 {
            assert_eq!(properties.len(), 1);
            assert_eq!(properties[0].0, "val");
            assert!(matches!(properties[0].1, WzProperty::Null));
        } else {
            panic!("Expected SubProperty");
        }
    }

    // ── try_decrypt_wave_format ───────────────────────────────────

    #[test]
    fn test_try_decrypt_wave_format_already_valid() {
        // Build a valid WAVEFORMATEX: extra_size = 0, total = 18 bytes
        let mut wav = vec![0u8; WAVE_FORMAT_SIZE];
        wav[16] = 0;
        wav[17] = 0; // extra_size = 0
        let key = vec![0xFF; 18];
        let original = wav.clone();
        let result = try_decrypt_wave_format(&mut wav, &key);
        assert!(!result); // No decryption needed
        assert_eq!(wav, original); // Data unchanged
    }

    #[test]
    fn test_try_decrypt_wave_format_too_short() {
        let mut wav = vec![0u8; 10]; // < WAVE_FORMAT_SIZE
        let result = try_decrypt_wave_format(&mut wav, &[]);
        assert!(!result);
    }

    #[test]
    fn test_try_decrypt_wave_format_decrypts() {
        // Build a WAVEFORMATEX with extra_size=2, total=20 bytes
        let mut plain = vec![0u8; 20];
        plain[16] = 2;
        plain[17] = 0; // extra_size = 2 → 18 + 2 = 20 ✓

        // Encrypt with a key
        let key = vec![0x55u8; 20];
        let mut encrypted: Vec<u8> = plain.iter().zip(key.iter()).map(|(a, b)| a ^ b).collect();

        // Verify encrypted version is NOT valid before decryption
        let extra_before = u16::from_le_bytes([encrypted[16], encrypted[17]]) as usize;
        assert_ne!(WAVE_FORMAT_SIZE + extra_before, encrypted.len());

        let result = try_decrypt_wave_format(&mut encrypted, &key);
        assert!(result);
        assert_eq!(encrypted, plain); // Decrypted back to original
    }

    // ── Error cases ────────────────────────────────────────────────

    #[test]
    fn test_parse_unknown_property_type_error() {
        let value = vec![0xFEu8]; // unknown marker
        let data = build_image_with_property("x", &value);
        let mut reader = make_reader(data);
        let err = parse_image(&mut reader).unwrap_err();
        matches!(err, WzError::UnknownPropertyType(_));
    }

    #[test]
    fn test_parse_invalid_property_count() {
        let mut data = property_image_header();
        // Compressed int for 600,000: indicator=0x80 + i32
        data.push(0x80);
        data.extend_from_slice(&600_000i32.to_le_bytes());
        let mut reader = make_reader(data);
        let err = parse_image(&mut reader).unwrap_err();
        matches!(err, WzError::Custom(_));
    }

    #[test]
    fn test_parse_extended_invalid_type_byte() {
        // Extended property with invalid type byte (not 0x00, 0x01, 0x1B, 0x73)
        let inner = vec![0xFFu8]; // invalid type byte

        let mut value = vec![0x09u8];
        value.extend_from_slice(&(inner.len() as u32).to_le_bytes());
        value.extend_from_slice(&inner);

        let data = build_image_with_property("bad", &value);
        let mut reader = make_reader(data);
        let err = parse_image(&mut reader).unwrap_err();
        assert!(matches!(err, WzError::Custom(_)));
    }

    #[test]
    fn test_parse_unknown_extended_type_returns_string() {
        // An unknown type name falls through to the catch-all → WzProperty::String
        let inner: Vec<u8> = Vec::new();
        let value = build_extended_property("SomeUnknownType", &inner);
        let data = build_image_with_property("unk", &value);
        let mut reader = make_reader(data);
        let props = parse_image(&mut reader).unwrap();
        assert_eq!(props[0].1.as_str(), Some("SomeUnknownType"));
    }
}

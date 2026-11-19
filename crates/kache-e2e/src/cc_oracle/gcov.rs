//! GCC coverage stamps.
//!
//! GCC writes the compile time as a four-byte stamp into a `.gcno` notes file
//! (bytes 8..12) and into the `stamp` field of the `gcov_info` record in its
//! object, so two raw compiles of one unit differ there and nowhere else. The
//! comparator zeroes exactly that field, and only after finding the record by
//! its structure: version word, stamp, a relocated `.gcda` filename pointer and
//! a relocated `__gcov_merge_*` pointer, in the 64-bit little-endian ELF
//! layout. Anything else stays unresolved, and its bytes are compared as they
//! are.

use std::path::Path;

/// How a file compares besides its raw digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stamp {
    /// Not part of a coverage pair: compare the raw bytes.
    None,
    /// A coverage object or notes file whose pair agrees on one stamp: the
    /// digest of the bytes with that stamp zeroed.
    Zeroed(String),
    /// Part of a coverage pair whose stamp could not be located safely.
    Unresolved(String),
}

/// `gcov_info` in GCC's 64-bit layout: version, `next`, stamp, checksum,
/// filename, then one merge function pointer per counter kind.
const STAMP_AT: usize = 16;
const FILENAME_AT: usize = 24;
const MERGE_AT: usize = 32;
/// More than GCC's counter kinds, so every merge slot is searched.
const MERGE_SLOTS: usize = 16;

pub fn stamp(path: &Path, bytes: &[u8]) -> std::io::Result<Stamp> {
    let read_pair = |pair: &Path| match std::fs::read(pair) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    };
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("o") => {
            let Some(notes) = read_pair(&path.with_extension("gcno"))? else {
                return Ok(Stamp::None);
            };
            Ok(match notes_header(&notes).and_then(|(version, stamp)| stamp_offset(bytes, version, stamp)) {
                Ok(at) => Stamp::Zeroed(zeroed_digest(bytes, at)),
                Err(reason) => Stamp::Unresolved(reason),
            })
        }
        Some("gcno") => {
            let Some(object) = read_pair(&path.with_extension("o"))? else {
                return Ok(Stamp::None);
            };
            Ok(match notes_header(bytes).and_then(|(version, stamp)| stamp_offset(&object, version, stamp)) {
                Ok(_) => Stamp::Zeroed(zeroed_digest(bytes, 8)),
                Err(reason) => Stamp::Unresolved(reason),
            })
        }
        _ => Ok(Stamp::None),
    }
}

fn zeroed_digest(bytes: &[u8], at: usize) -> String {
    let mut bytes = bytes.to_vec();
    bytes[at..at + 4].fill(0);
    blake3::hash(&bytes).to_hex().to_string()
}

fn notes_header(notes: &[u8]) -> Result<([u8; 4], [u8; 4]), String> {
    if notes.len() < 12 || &notes[..4] != b"oncg" {
        return Err("the notes file has no little-endian .gcno header".into());
    }
    Ok((notes[4..8].try_into().unwrap(), notes[8..12].try_into().unwrap()))
}

fn field<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..at.checked_add(N)?)?.try_into().ok()
}
fn u16_at(bytes: &[u8], at: usize) -> Option<usize> {
    field::<2>(bytes, at).map(|b| u16::from_le_bytes(b) as usize)
}
fn u32_at(bytes: &[u8], at: usize) -> Option<usize> {
    field::<4>(bytes, at).map(|b| u32::from_le_bytes(b) as usize)
}
fn u64_at(bytes: &[u8], at: usize) -> Option<usize> {
    field::<8>(bytes, at).and_then(|b| usize::try_from(u64::from_le_bytes(b)).ok())
}

struct Section {
    kind: usize,
    offset: usize,
    size: usize,
    link: usize,
    info: usize,
}

struct Reloc {
    offset: usize,
    symbol: usize,
    addend: i64,
}

struct Symbol {
    name: usize,
    section: usize,
    value: usize,
}

const SHT_SYMTAB: usize = 2;
const SHT_RELA: usize = 4;
const SHT_NOBITS: usize = 8;

/// The file offset of the one `gcov_info` stamp field carrying `stamp`.
fn stamp_offset(object: &[u8], version: [u8; 4], stamp: [u8; 4]) -> Result<usize, String> {
    if object.get(..6) != Some(b"\x7fELF\x02\x01".as_slice()) {
        return Err("the object is not a 64-bit little-endian ELF file".into());
    }
    let malformed = || "the object's ELF headers are malformed".to_string();
    let shoff = u64_at(object, 0x28).ok_or_else(malformed)?;
    let shentsize = u16_at(object, 0x3a).ok_or_else(malformed)?;
    let shnum = u16_at(object, 0x3c).ok_or_else(malformed)?;
    if shentsize < 0x40 {
        return Err(malformed());
    }
    let sections = (0..shnum)
        .map(|index| {
            let at = shoff.checked_add(index.checked_mul(shentsize)?)?;
            let section = Section {
                kind: u32_at(object, at + 4)?,
                offset: u64_at(object, at + 0x18)?,
                size: u64_at(object, at + 0x20)?,
                link: u32_at(object, at + 0x28)?,
                info: u32_at(object, at + 0x2c)?,
            };
            let in_file = section.kind == SHT_NOBITS || object.get(section.offset..section.offset.checked_add(section.size)?).is_some();
            in_file.then_some(section)
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(malformed)?;
    let data = |section: &Section| object.get(section.offset..section.offset + section.size).unwrap_or_default();
    let mut found = Vec::new();
    for rela in sections.iter().filter(|s| s.kind == SHT_RELA) {
        let (Some(target), Some(symtab)) = (sections.get(rela.info), sections.get(rela.link)) else {
            return Err(malformed());
        };
        if target.kind == SHT_NOBITS || symtab.kind != SHT_SYMTAB {
            continue;
        }
        let strtab = sections.get(symtab.link).ok_or_else(malformed)?;
        let relocs = data(rela)
            .as_chunks::<24>()
            .0
            .iter()
            .map(|r| Reloc {
                offset: u64_at(r, 0).unwrap(),
                symbol: (u64::from_le_bytes(r[8..16].try_into().unwrap()) >> 32) as usize,
                addend: i64::from_le_bytes(r[16..24].try_into().unwrap()),
            })
            .collect::<Vec<_>>();
        let symbol = |index: usize| -> Option<Symbol> {
            let at = index.checked_mul(24)?;
            let entry = data(symtab).get(at..at + 24)?;
            Some(Symbol { name: u32_at(entry, 0)?, section: u16_at(entry, 6)?, value: u64_at(entry, 8)? })
        };
        let name = |symbol: &Symbol| -> Option<&[u8]> {
            let bytes = data(strtab).get(symbol.name..)?;
            Some(&bytes[..bytes.iter().position(|&b| b == 0)?])
        };
        let points_at_gcda = |reloc: &Reloc| -> bool {
            let Some(symbol) = symbol(reloc.symbol) else { return false };
            let Some(section) = sections.get(symbol.section).filter(|s| s.kind != SHT_NOBITS) else {
                return false;
            };
            let Some(start) = (symbol.value as i64).checked_add(reloc.addend).and_then(|at| usize::try_from(at).ok()) else {
                return false;
            };
            let Some(text) = data(section).get(start..) else { return false };
            text.iter().position(|&b| b == 0).is_some_and(|end| text[..end].ends_with(b".gcda"))
        };
        let merges = |base: usize| {
            relocs.iter().any(|reloc| {
                reloc.offset >= base + MERGE_AT
                    && reloc.offset < base + MERGE_AT + 8 * MERGE_SLOTS
                    && (reloc.offset - base - MERGE_AT) % 8 == 0
                    && symbol(reloc.symbol).and_then(|s| name(&s).map(|n| n.starts_with(b"__gcov_merge_"))) == Some(true)
            })
        };
        let bytes = data(target);
        for reloc in relocs.iter().filter(|r| points_at_gcda(r)) {
            let Some(base) = reloc.offset.checked_sub(FILENAME_AT) else { continue };
            if field::<4>(bytes, base) == Some(version) && field::<4>(bytes, base + STAMP_AT) == Some(stamp) && merges(base) {
                found.push(target.offset + base + STAMP_AT);
            }
        }
    }
    match found.as_slice() {
        [at] => Ok(*at),
        [] => Err("no gcov_info record carries the notes file's version and stamp".into()),
        _ => Err(format!("{} gcov_info records carry the notes file's stamp", found.len())),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A relocatable ELF64 object: `.text` holding `text`, `.rodata` holding
    /// the `.gcda` path, `.data.rel` holding one `gcov_info` record with
    /// `version` and `stamp`, and the relocations GCC emits for it.
    pub fn object(text: &[u8], version: [u8; 4], stamp: [u8; 4]) -> Vec<u8> {
        let rodata = b"/w/u.gcda\0".to_vec();
        let mut info = vec![0u8; 40];
        info[..4].copy_from_slice(&version);
        info[STAMP_AT..STAMP_AT + 4].copy_from_slice(&stamp);
        let strtab = b"\0__gcov_merge_add\0".to_vec();
        let mut symtab = vec![0u8; 24];
        let mut rodata_symbol = vec![0u8; 24];
        rodata_symbol[6..8].copy_from_slice(&2u16.to_le_bytes());
        symtab.extend(rodata_symbol);
        let mut merge_symbol = vec![0u8; 24];
        merge_symbol[..4].copy_from_slice(&1u32.to_le_bytes());
        symtab.extend(merge_symbol);
        let reloc = |offset: u64, symbol: u64| {
            [offset.to_le_bytes(), ((symbol << 32) | 1).to_le_bytes(), 0i64.to_le_bytes()].concat()
        };
        let rela = [reloc(FILENAME_AT as u64, 1), reloc(MERGE_AT as u64, 2)].concat();
        // null, .text, .rodata, .data.rel, .symtab, .strtab, .rela.data.rel
        let bodies: [(u32, &[u8], u32, u32); 6] = [
            (1, text, 0, 0),
            (1, &rodata, 0, 0),
            (1, &info, 0, 0),
            (SHT_SYMTAB as u32, &symtab, 5, 0),
            (3, &strtab, 0, 0),
            (SHT_RELA as u32, &rela, 4, 3),
        ];
        let mut out = vec![0u8; 0x40];
        out[..6].copy_from_slice(b"\x7fELF\x02\x01");
        let mut headers = vec![0u8; 0x40];
        for (kind, body, link, info) in bodies {
            let offset = out.len() as u64;
            out.extend_from_slice(body);
            let mut header = vec![0u8; 0x40];
            header[4..8].copy_from_slice(&kind.to_le_bytes());
            header[0x18..0x20].copy_from_slice(&offset.to_le_bytes());
            header[0x20..0x28].copy_from_slice(&(body.len() as u64).to_le_bytes());
            header[0x28..0x2c].copy_from_slice(&link.to_le_bytes());
            header[0x2c..0x30].copy_from_slice(&info.to_le_bytes());
            headers.extend(header);
        }
        let shoff = out.len() as u64;
        out.extend(headers);
        out[0x28..0x30].copy_from_slice(&shoff.to_le_bytes());
        out[0x3a..0x3c].copy_from_slice(&0x40u16.to_le_bytes());
        out[0x3c..0x3e].copy_from_slice(&7u16.to_le_bytes());
        out
    }

    pub fn notes(version: [u8; 4], stamp: [u8; 4]) -> Vec<u8> {
        [&b"oncg"[..], &version, &stamp, b"notes body"].concat()
    }

    pub const VERSION: [u8; 4] = *b"*51B";

    #[test]
    fn finds_only_the_gcov_info_stamp_field() {
        let stamp = *b"\xa6\x55\xbf\xf4";
        let text = [&b"code"[..], &stamp, b"more"].concat();
        let object = object(&text, VERSION, stamp);
        let at = stamp_offset(&object, VERSION, stamp).unwrap();
        assert_eq!(&object[at..at + 4], &stamp);
        assert_ne!(object.windows(4).position(|w| w == stamp), Some(at), "the .text copy comes first");
    }

    #[test]
    fn a_zero_stamp_is_located_like_any_other() {
        let object = object(b"\0\0\0\0\0\0\0\0", VERSION, [0; 4]);
        let at = stamp_offset(&object, VERSION, [0; 4]).unwrap();
        assert_eq!(zeroed_digest(&object, at), blake3::hash(&object).to_hex().to_string());
    }

    #[test]
    fn records_that_disagree_with_the_notes_stay_unresolved() {
        let object = object(b"code", VERSION, *b"\x01\x02\x03\x04");
        assert!(stamp_offset(&object, VERSION, *b"\x05\x06\x07\x08").is_err());
        assert!(stamp_offset(&object, *b"*41B", *b"\x01\x02\x03\x04").is_err());
        assert!(stamp_offset(b"\xcf\xfa\xed\xfe macho", VERSION, [0; 4]).is_err());
        let mut truncated = object.clone();
        truncated.truncate(object.len() - 2 * 0x40);
        assert!(stamp_offset(&truncated, VERSION, *b"\x01\x02\x03\x04").is_err());
        assert!(notes_header(b"oncg*51B\x01\x02").is_err());
        assert!(notes_header(&notes(VERSION, [0; 4])).is_ok());
    }
}

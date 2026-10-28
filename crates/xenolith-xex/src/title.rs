//! The name a title gives itself.
//!
//! A title carries the name the dashboard shows in a resource embedded in its
//! image, named after its title id. The resource is a small database of tables,
//! with one table of strings for each language, and the name is one string in
//! the table for the language the title calls its own.

use crate::container::{Container, OptionalHeaderKey, OptionalHeaderValue};
use crate::headers::keys;
use crate::image::Image;
use crate::reader::Reader;

/// What a database of tables starts with.
const DATABASE: [u8; 4] = *b"XDBF";

/// The tables describing the title.
const DESCRIBING: u16 = 1;

/// The tables of strings, one for each language and numbered by it.
const STRINGS: u16 = 3;

/// The table saying which language the title calls its own.
const OWN_LANGUAGE: u64 = 0x5853_5443;

/// The language read where a title names none of its own, English.
const ENGLISH: u64 = 1;

/// The string a title's name is kept under.
const NAME: u16 = 0x8000;

/// Bytes of the database's header, and of each entry and each free span after
/// it.
const HEADER: u64 = 24;
const ENTRY: u64 = 18;
const FREE: u64 = 8;

/// Returns the name a title gives itself, or nothing where its container
/// carries no such resource or the resource does not read as one.
#[must_use]
pub fn title_name(container: &Container<'_>, image: &Image) -> Option<String> {
    let wanted = format!("{:08X}", container.execution_info()?.title_id);
    let directory = container
        .optional_header(OptionalHeaderKey(keys::RESOURCE_INFO))
        .and_then(OptionalHeaderValue::data)?;

    let mut reader = Reader::new(directory);
    let size = reader.u32("resource_info.size").ok()?;
    let count = size.saturating_sub(4) / 16;
    for _ in 0..count {
        let name = reader.take_array::<8>("resource_info.name").ok()?;
        let address = reader.u32("resource_info.address").ok()?;
        let length = reader.u32("resource_info.size").ok()?;
        let name = name.split(|byte| *byte == 0).next().unwrap_or_default();
        if name == wanted.as_bytes() {
            let bytes = image.read(address, usize::try_from(length).ok()?).ok()?;
            return name_in_database(bytes);
        }
    }
    None
}

/// Reads a title's name out of the database it embeds.
fn name_in_database(bytes: &[u8]) -> Option<String> {
    let mut reader = Reader::new(bytes);
    if reader.take_array::<4>("database.magic").ok()? != DATABASE {
        return None;
    }
    let _version = reader.u32("database.version").ok()?;
    let entry_room = reader.u32("database.entry_room").ok()?;
    let entry_count = reader.u32("database.entry_count").ok()?;
    let free_room = reader.u32("database.free_room").ok()?;
    let _free_count = reader.u32("database.free_count").ok()?;
    let data = HEADER + u64::from(entry_room) * ENTRY + u64::from(free_room) * FREE;

    let mut entries = Vec::new();
    for _ in 0..entry_count {
        let namespace = reader.u16("database.namespace").ok()?;
        let high = reader.u32("database.id").ok()?;
        let low = reader.u32("database.id").ok()?;
        let offset = reader.u32("database.offset").ok()?;
        let _length = reader.u32("database.length").ok()?;
        let id = (u64::from(high) << 32) | u64::from(low);
        entries.push((namespace, id, data + u64::from(offset)));
    }
    let find = |namespace: u16, id: u64| {
        entries
            .iter()
            .find(|entry| entry.0 == namespace && entry.1 == id)
            .map(|entry| entry.2)
    };

    /* The table naming the language is a magic, a version and a size, and then
     * the language. */
    let language = find(DESCRIBING, OWN_LANGUAGE)
        .and_then(|at| reader.at(at + 12, "database.language").ok())
        .and_then(|mut at| at.u32("database.language").ok())
        .map_or(ENGLISH, u64::from);
    let table = find(STRINGS, language).or_else(|| find(STRINGS, ENGLISH))?;

    let mut strings = reader.at(table + 12, "strings.count").ok()?;
    let count = strings.u16("strings.count").ok()?;
    for _ in 0..count {
        let id = strings.u16("strings.id").ok()?;
        let length = strings.u16("strings.length").ok()?;
        let text = strings.take(usize::from(length), "strings.text").ok()?;
        if id == NAME {
            let name = core::str::from_utf8(text).ok()?.trim();
            return (!name.is_empty()).then(|| name.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::name_in_database;

    /// A database holding a table naming `language` as the title's own, and a
    /// table of strings for each of `tables`.
    fn database(language: u32, tables: &[(u32, &str)]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut entries = Vec::new();

        entries.push((1u16, 0x5853_5443u64, data.len()));
        data.extend_from_slice(b"XSTC");
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(&4u32.to_be_bytes());
        data.extend_from_slice(&language.to_be_bytes());

        for (table, name) in tables {
            entries.push((3, u64::from(*table), data.len()));
            data.extend_from_slice(b"XSTR");
            data.extend_from_slice(&1u32.to_be_bytes());
            data.extend_from_slice(&0u32.to_be_bytes());
            data.extend_from_slice(&2u16.to_be_bytes());
            for (id, text) in [(0x8001u16, "not the name"), (0x8000, *name)] {
                data.extend_from_slice(&id.to_be_bytes());
                data.extend_from_slice(&u16::try_from(text.len()).unwrap_or(0).to_be_bytes());
                data.extend_from_slice(text.as_bytes());
            }
        }

        let mut bytes = b"XDBF".to_vec();
        let count = u32::try_from(entries.len()).unwrap_or(0);
        for word in [0x10000u32, count + 1, count, 1, 0] {
            bytes.extend_from_slice(&word.to_be_bytes());
        }
        for (namespace, id, offset) in entries {
            bytes.extend_from_slice(&namespace.to_be_bytes());
            bytes.extend_from_slice(&id.to_be_bytes());
            bytes.extend_from_slice(&u32::try_from(offset).unwrap_or(0).to_be_bytes());
            bytes.extend_from_slice(&0u32.to_be_bytes());
        }
        /* The room left for one more entry, and the one free span. */
        bytes.extend_from_slice(&[0; 18]);
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&data);
        bytes
    }

    #[test]
    fn the_name_is_read_in_the_title_s_own_language() {
        let bytes = database(2, &[(1, "In English"), (2, "In its own")]);
        assert_eq!(name_in_database(&bytes).as_deref(), Some("In its own"));
    }

    #[test]
    fn a_language_with_no_table_falls_back_to_english() {
        let bytes = database(5, &[(1, "In English")]);
        assert_eq!(name_in_database(&bytes).as_deref(), Some("In English"));
    }

    #[test]
    fn something_that_is_not_a_database_has_no_name() {
        let mut bytes = database(1, &[(1, "In English")]);
        bytes[0] = b'Y';
        assert_eq!(name_in_database(&bytes), None);
        assert_eq!(name_in_database(&[]), None);
    }
}

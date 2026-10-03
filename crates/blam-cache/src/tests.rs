use super::*;
use std::io::Cursor;

fn put(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn magic(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

/// Build a tiny but structurally valid Vista map with two tags.
fn synthetic_map() -> Vec<u8> {
    let mut b = vec![0u8; 0x2000];
    put(&mut b, 0, magic(b"head"));
    put(&mut b, 4, 8);
    put(&mut b, 0x7FC, magic(b"foot"));
    b[0x12C..0x12C + 4].copy_from_slice(b"test");
    put(&mut b, 0x14C, 1);
    b[0x1A4..0x1A4 + 7].copy_from_slice(b"lockout");

    // string ids at 0x800 (index) / 0x810 (data)
    let sdata = b"\0default\0red\0";
    put(&mut b, 0x170, 3);
    put(&mut b, 0x174, sdata.len() as u32);
    put(&mut b, 0x178, 0x800);
    put(&mut b, 0x17C, 0x810);
    for (i, o) in [0u32, 1, 9].iter().enumerate() {
        put(&mut b, 0x800 + i * 4, *o);
    }
    b[0x810..0x810 + sdata.len()].copy_from_slice(sdata);

    // tag names at 0x900 (index) / 0x910 (data)
    let ndata = b"scenarios\\multi\\lockout\0globals\\globals\0";
    put(&mut b, 0x2CC, 2);
    put(&mut b, 0x2D0, 0x910);
    put(&mut b, 0x2D4, ndata.len() as u32);
    put(&mut b, 0x2D8, 0x900);
    put(&mut b, 0x900, 0);
    put(&mut b, 0x904, 24);
    b[0x910..0x910 + ndata.len()].copy_from_slice(ndata);

    // meta area at 0x1000, mask 0x8000_0000
    let meta = 0x1000;
    let mask = 0x8000_0000u32;
    put(&mut b, 0x10, meta as u32);
    put(&mut b, 0x14, 0x100);
    put(&mut b, 0x1C, 0x1000);
    put(&mut b, 0x20, mask);
    // meta header
    put(&mut b, meta, 0x20); // group table
    put(&mut b, meta + 4, 2);
    put(&mut b, meta + 8, 0x38); // tag table
    put(&mut b, meta + 0xC, 0xE174_0000);
    put(&mut b, meta + 0x10, 0xE175_0001);
    put(&mut b, meta + 0x18, 2);
    put(&mut b, meta + 0x1C, magic(b"tags"));
    // groups
    put(&mut b, meta + 0x20, magic(b"scnr"));
    put(&mut b, meta + 0x24, u32::MAX);
    put(&mut b, meta + 0x28, u32::MAX);
    put(&mut b, meta + 0x2C, magic(b"matg"));
    put(&mut b, meta + 0x30, u32::MAX);
    put(&mut b, meta + 0x34, u32::MAX);
    // tags
    let t = meta + 0x38;
    put(&mut b, t, magic(b"scnr"));
    put(&mut b, t + 4, 0xE174_0000);
    put(&mut b, t + 8, mask + 0x200);
    put(&mut b, t + 0xC, 4);
    put(&mut b, t + 0x10, magic(b"matg"));
    put(&mut b, t + 0x14, 0xE175_0001);
    put(&mut b, t + 0x18, 0);
    put(&mut b, t + 0x1C, 0);
    b[meta + 0x200..meta + 0x204].copy_from_slice(&[1, 2, 3, 4]);
    b
}

#[test]
fn parses_synthetic_map() {
    let mut map = CacheFile::from_reader(Cursor::new(synthetic_map())).unwrap();
    assert_eq!(map.header.internal_name, "lockout");
    assert_eq!(map.header.map_type, MapType::Multiplayer);
    assert_eq!(map.groups.len(), 2);
    assert_eq!(map.groups[0].tag.to_string(), "scnr");
    assert_eq!(map.tags.len(), 2);
    assert_eq!(map.tags[0].name, "scenarios\\multi\\lockout");
    assert_eq!(map.tags[1].name, "globals\\globals");
    assert_eq!(
        map.tag(map.scenario).unwrap().group,
        GroupTag::parse("scnr").unwrap()
    );
    assert_eq!(map.string_id(0x0700_0001), Some("default"));
    assert_eq!(map.string_id(2), Some("red"));

    let scnr = map.tags[0].clone();
    assert_eq!(map.read_tag_data(&scnr).unwrap(), vec![1, 2, 3, 4]);
    let matg = map.tags[1].clone();
    assert!(!matg.has_data());
    assert!(map.read_tag_data(&matg).unwrap().is_empty());
}

#[test]
fn rejects_non_map() {
    let err = CacheFile::from_reader(Cursor::new(vec![0u8; 0x1000]))
        .err()
        .unwrap();
    assert!(matches!(err, Error::BadMagic(_)));
}

#[test]
fn rejects_wrong_version() {
    let mut b = synthetic_map();
    put(&mut b, 4, 9);
    assert!(matches!(
        CacheFile::from_reader(Cursor::new(b)),
        Err(Error::Version(9))
    ));
}

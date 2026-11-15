use super::*;

#[test]
fn anvil_mca_and_chunk_mutation_roundtrip() {
    let dir = std::env::temp_dir().join("rad_anvil_test");
    let _ = std::fs::remove_dir_all(&dir);

    let mut chunk = ChunkColumn::flat();
    assert_eq!(chunk.get_block(3, 64, 4), 0);
    assert!(chunk.set_block(3, 64, 4, 1));
    assert_eq!(chunk.get_block(3, 64, 4), 1);
    assert!(chunk.dirty);

    let pkt = chunk.to_packet(0, 0);
    assert!(!pkt.data.is_empty());

    let encoded = chunk.encode_storage_bytes().unwrap();
    AnvilRegion::write_chunk_payload(&dir, 0, 0, &encoded).unwrap();
    let read_back = AnvilRegion::read_chunk_payload(&dir, 0, 0).unwrap();
    let decoded = ChunkColumn::decode_storage_bytes(&read_back).unwrap();
    assert_eq!(decoded.get_block(3, 64, 4), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn direct_palette_encoding_handles_over_256_distinct_states() {
    let mut chunk = ChunkColumn::flat();
    // Set 300 distinct block states inside section y=64..79
    for i in 0..300u16 {
        let x = (i % 16) as usize;
        let z = ((i / 16) % 16) as usize;
        let y = 64 + (i / 256) as i32;
        assert!(chunk.set_block(x, y, z, i + 1));
    }
    // Verify storage roundtrip preserves all 300 states
    let encoded = chunk.encode_storage_bytes().unwrap();
    let decoded = ChunkColumn::decode_storage_bytes(&encoded).unwrap();
    for i in 0..300u16 {
        let x = (i % 16) as usize;
        let z = ((i / 16) % 16) as usize;
        let y = 64 + (i / 256) as i32;
        assert_eq!(decoded.get_block(x, y, z), i + 1);
    }
    // Packet generation must succeed cleanly without truncation/panic
    let pkt = chunk.to_packet(0, 0);
    assert!(!pkt.data.is_empty());
}

#[test]
fn vanilla_mca_nbt_chunk_imports_sections_and_palette() {
    use rad_core::nbt::{NbtTag, encode_named_nbt};
    use std::collections::BTreeMap;

    let mut air = BTreeMap::new();
    air.insert("Name".into(), NbtTag::String("minecraft:air".into()));
    let mut stone = BTreeMap::new();
    stone.insert("Name".into(), NbtTag::String("minecraft:stone".into()));
    let mut grass = BTreeMap::new();
    grass.insert(
        "Name".into(),
        NbtTag::String("minecraft:grass_block".into()),
    );

    // Pack section Y = 4 (y = 64..79): index 0 = grass (pal 2), index 1 = stone (pal 1), rest air (pal 0)
    // 4 bits per entry -> 16 entries per i64 -> 256 i64s
    let mut longs = vec![0i64; 256];
    longs[0] = (2i64) | (1i64 << 4);

    let mut bs = BTreeMap::new();
    bs.insert(
        "palette".into(),
        NbtTag::List(vec![
            NbtTag::Compound(air),
            NbtTag::Compound(stone),
            NbtTag::Compound(grass),
        ]),
    );
    bs.insert("data".into(), NbtTag::LongArray(longs));

    let mut sec4 = BTreeMap::new();
    sec4.insert("Y".into(), NbtTag::Byte(4));
    sec4.insert("block_states".into(), NbtTag::Compound(bs));

    let mut root = BTreeMap::new();
    root.insert(
        "sections".into(),
        NbtTag::List(vec![NbtTag::Compound(sec4)]),
    );

    let mut nbt_buf = BytesMut::new();
    encode_named_nbt("", &NbtTag::Compound(root), &mut nbt_buf).unwrap();

    let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(&nbt_buf).unwrap();
    let compressed = enc.finish().unwrap();

    let dir = std::env::temp_dir().join(format!(
        "radbyte_vanilla_mca_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    AnvilRegion::write_chunk_payload(&dir, 0, 0, &compressed).unwrap();

    let payload = AnvilRegion::read_chunk_payload(&dir, 0, 0).unwrap();
    let decoded = ChunkColumn::decode_storage_bytes(&payload).unwrap();
    assert_eq!(decoded.get_block(0, 64, 0), 9); // grass_block
    assert_eq!(decoded.get_block(1, 64, 0), 1); // stone
    assert_eq!(decoded.get_block(2, 64, 0), 0); // air

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vanilla_nbt_chunk_rejects_unknown_block_name_instead_of_stone() {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use rad_core::nbt::{NbtTag, encode_named_nbt};
    use std::collections::BTreeMap;
    use std::io::Write;

    let mut p0 = BTreeMap::new();
    p0.insert(
        "Name".into(),
        NbtTag::String("minecraft:nonexistent_modded_block_xyz".into()),
    );
    let mut bs = BTreeMap::new();
    bs.insert("palette".into(), NbtTag::List(vec![NbtTag::Compound(p0)]));

    let mut sec4 = BTreeMap::new();
    sec4.insert("Y".into(), NbtTag::Byte(4));
    sec4.insert("block_states".into(), NbtTag::Compound(bs));

    let mut root = BTreeMap::new();
    root.insert(
        "sections".into(),
        NbtTag::List(vec![NbtTag::Compound(sec4)]),
    );

    let mut nbt_buf = BytesMut::new();
    encode_named_nbt("", &NbtTag::Compound(root), &mut nbt_buf).unwrap();

    let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(&nbt_buf).unwrap();
    let compressed = enc.finish().unwrap();

    assert!(
        ChunkColumn::decode_storage_bytes(&compressed).is_none(),
        "unknown vanilla block name must be rejected rather than silently mapped to stone"
    );
}

#[test]
fn vanilla_import_preserves_properties_metadata_biomes_and_separates_rad1() {
    use rad_core::nbt::{NbtTag, encode_named_nbt};
    use std::collections::BTreeMap;

    let mut props = BTreeMap::new();
    props.insert("age".into(), NbtTag::String("7".into()));
    let mut p_wheat = BTreeMap::new();
    p_wheat.insert("Name".into(), NbtTag::String("minecraft:wheat".into()));
    p_wheat.insert("Properties".into(), NbtTag::Compound(props));

    let mut bs = BTreeMap::new();
    bs.insert(
        "palette".into(),
        NbtTag::List(vec![NbtTag::Compound(p_wheat)]),
    );

    let mut biomes = BTreeMap::new();
    biomes.insert(
        "palette".into(),
        NbtTag::List(vec![NbtTag::String("minecraft:plains".into())]),
    );

    let mut sec4 = BTreeMap::new();
    sec4.insert("Y".into(), NbtTag::Byte(4));
    sec4.insert("block_states".into(), NbtTag::Compound(bs));
    sec4.insert("biomes".into(), NbtTag::Compound(biomes));

    let mut be = BTreeMap::new();
    be.insert("id".into(), NbtTag::String("minecraft:chest".into()));
    be.insert("x".into(), NbtTag::Int(1));
    be.insert("y".into(), NbtTag::Int(64));
    be.insert("z".into(), NbtTag::Int(2));

    let mut root = BTreeMap::new();
    root.insert("DataVersion".into(), NbtTag::Int(4790));
    root.insert(
        "sections".into(),
        NbtTag::List(vec![NbtTag::Compound(sec4)]),
    );
    root.insert(
        "block_entities".into(),
        NbtTag::List(vec![NbtTag::Compound(be)]),
    );

    let mut nbt_buf = BytesMut::new();
    encode_named_nbt("", &NbtTag::Compound(root), &mut nbt_buf).unwrap();

    assert_eq!(detect_storage_format(&nbt_buf), StorageFormat::VanillaAnvil);
    assert_eq!(detect_storage_format(b"RAD1\x00"), StorageFormat::Rad1);

    let imported = ChunkColumn::decode_vanilla_nbt_chunk_with_metadata(&nbt_buf).unwrap();
    assert_eq!(imported.data_version, Some(4790));
    assert_eq!(imported.block_entities.len(), 1);
    assert_eq!(
        imported.section_biomes,
        vec![(4, vec!["minecraft:plains".to_owned()])]
    );
    assert_eq!(
        imported.column.get_block(0, 64, 0),
        rad_core::mc777_data::blocks::WHEAT_RANGE.1
    );
}

#[test]
fn dynamic_block_and_sky_lighting_propagate_from_torch() {
    use rad_core::mc777_data::blocks;
    let mut col = ChunkColumn::flat();
    // Place a torch in air at (4, 64, 4) -> section 8 (y=64..79), light section 9.
    assert!(col.set_block(4, 64, 4, blocks::TORCH));
    let pkt = col.to_packet(0, 0);

    let block_mask = u32::from_le_bytes(pkt.block_light_mask.clone().try_into().unwrap());
    assert_ne!(
        block_mask & (1u32 << 9),
        0,
        "section 9 should have block light"
    );
    let block_arr_idx = (block_mask & ((1u32 << 9) - 1)).count_ones() as usize;
    let sec9_block = &pkt.block_lights[block_arr_idx];
    assert_eq!(get_nibble(sec9_block, 4, 0, 4), 14);
    assert_eq!(get_nibble(sec9_block, 5, 0, 4), 13);
    assert_eq!(get_nibble(sec9_block, 6, 0, 4), 12);

    // Skylight above grass (y=64) should be 15, while below stone (y=60) should be 0.
    let sky_mask = u32::from_le_bytes(pkt.sky_light_mask.clone().try_into().unwrap());
    assert_ne!(sky_mask & (1u32 << 9), 0);
    assert_eq!(
        sky_mask & (1u32 << 7),
        0,
        "underground section 7 has no skylight"
    );
}

#[test]
fn vanilla_export_preserves_unknown_fields_and_roundtrips_mca() {
    use rad_core::mc777_data::blocks as mc_blocks;
    use rad_core::nbt::{NbtTag, decode_named_nbt, encode_named_nbt};
    use std::collections::BTreeMap;

    let uniq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tmp_root = std::env::temp_dir().join(format!("rad_vanilla_export_{uniq}"));
    let src_world = tmp_root.join("src_vanilla");
    let rad_world = tmp_root.join("rad_sidecar");
    let dst_world = tmp_root.join("dst_vanilla");
    std::fs::create_dir_all(&src_world).unwrap();
    std::fs::create_dir_all(&rad_world).unwrap();
    std::fs::create_dir_all(&dst_world).unwrap();

    // Build a realistic vanilla Anvil NBT chunk with unknown root and section fields.
    let mut p_stone = BTreeMap::new();
    p_stone.insert("Name".into(), NbtTag::String("minecraft:stone".into()));
    let mut bs4 = BTreeMap::new();
    bs4.insert(
        "palette".into(),
        NbtTag::List(vec![NbtTag::Compound(p_stone)]),
    );

    let mut biomes4 = BTreeMap::new();
    biomes4.insert(
        "palette".into(),
        NbtTag::List(vec![NbtTag::String("minecraft:plains".into())]),
    );

    let mut sec4 = BTreeMap::new();
    sec4.insert("Y".into(), NbtTag::Byte(4));
    sec4.insert("block_states".into(), NbtTag::Compound(bs4));
    sec4.insert("biomes".into(), NbtTag::Compound(biomes4.clone()));
    sec4.insert("BlockLight".into(), NbtTag::ByteArray(vec![0x12; 2048]));
    sec4.insert("SkyLight".into(), NbtTag::ByteArray(vec![-1i8; 2048]));
    sec4.insert(
        "CustomSecTag".into(),
        NbtTag::String("keep_sec_meta".into()),
    );

    let mut heightmaps = BTreeMap::new();
    heightmaps.insert(
        "MOTION_BLOCKING".into(),
        NbtTag::LongArray(vec![11, 22, 33]),
    );

    let mut structures = BTreeMap::new();
    structures.insert("starts".into(), NbtTag::Compound(BTreeMap::new()));

    let mut be = BTreeMap::new();
    be.insert("id".into(), NbtTag::String("minecraft:chest".into()));
    be.insert("x".into(), NbtTag::Int(5));
    be.insert("y".into(), NbtTag::Int(65));
    be.insert("z".into(), NbtTag::Int(6));

    let mut custom_meta = BTreeMap::new();
    custom_meta.insert("seed_hint".into(), NbtTag::Long(987_654_321));

    let mut root = BTreeMap::new();
    root.insert("DataVersion".into(), NbtTag::Int(4790));
    root.insert("xPos".into(), NbtTag::Int(1));
    root.insert("yPos".into(), NbtTag::Int(-4));
    root.insert("zPos".into(), NbtTag::Int(-1));
    root.insert("Status".into(), NbtTag::String("minecraft:full".into()));
    root.insert("InhabitedTime".into(), NbtTag::Long(42_000));
    root.insert("LastUpdate".into(), NbtTag::Long(99_999));
    root.insert("Heightmaps".into(), NbtTag::Compound(heightmaps.clone()));
    root.insert("structures".into(), NbtTag::Compound(structures.clone()));
    root.insert("entities".into(), NbtTag::List(vec![]));
    root.insert(
        "block_entities".into(),
        NbtTag::List(vec![NbtTag::Compound(be.clone())]),
    );
    root.insert("CustomMeta".into(), NbtTag::Compound(custom_meta.clone()));
    root.insert(
        "sections".into(),
        NbtTag::List(vec![NbtTag::Compound(sec4)]),
    );

    let mut orig_nbt = BytesMut::new();
    encode_named_nbt("", &NbtTag::Compound(root), &mut orig_nbt).unwrap();
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&orig_nbt).unwrap();
    let orig_zlib = enc.finish().unwrap();
    AnvilRegion::write_chunk_payload(&src_world, 1, -1, &orig_zlib).unwrap();

    // Import from source vanilla .mca, mutate blocks, save RAD1 sidecar, and reload RAD1.
    let src_payload = AnvilRegion::read_chunk_payload_checked(&src_world, 1, -1)
        .unwrap()
        .expect("source vanilla chunk present");
    let mut dec = ZlibDecoder::new(&src_payload[..]);
    let mut src_raw_nbt = Vec::new();
    dec.read_to_end(&mut src_raw_nbt).unwrap();
    let imported = ChunkColumn::decode_vanilla_nbt_chunk_with_metadata(&src_raw_nbt).unwrap();
    let mut col = imported.column;
    assert_eq!(col.get_block(0, 64, 0), mc_blocks::STONE);

    let wheat_def = mc_blocks::by_name("wheat").unwrap();
    let wheat_age5 = wheat_def.min_state + 5;
    let oak_log_def = mc_blocks::by_name("oak_log").unwrap();
    let oak_log_z = oak_log_def.max_state;
    assert!(col.set_block(1, 65, 1, wheat_age5));
    assert!(col.set_block(2, 66, 2, mc_blocks::CHEST));
    assert!(col.set_block(3, 64, 3, oak_log_z));
    assert!(col.set_block(4, 64, 4, mc_blocks::LEVER + 1));

    // Roundtrip through RAD1 sidecar storage.
    let rad1_zlib = col.encode_storage_bytes().unwrap();
    AnvilRegion::write_chunk_payload(&rad_world, 1, -1, &rad1_zlib).unwrap();
    let rad1_loaded_payload = AnvilRegion::read_chunk_payload_checked(&rad_world, 1, -1)
        .unwrap()
        .unwrap();
    let mut rad1_raw = Vec::new();
    ZlibDecoder::new(&rad1_loaded_payload[..])
        .read_to_end(&mut rad1_raw)
        .unwrap();
    assert_eq!(detect_storage_format(&rad1_raw), StorageFormat::Rad1);
    let rad1_col = ChunkColumn::decode_storage_bytes_checked(&rad1_loaded_payload).unwrap();

    // Export modified RAD1 chunk back into standard vanilla Anvil .mca using original NBT template.
    let exported_zlib = rad1_col
        .export_to_vanilla_storage_bytes(Some(&src_raw_nbt), 1, -1)
        .unwrap();
    AnvilRegion::write_chunk_payload(&dst_world, 1, -1, &exported_zlib).unwrap();

    // Independent verification of the exported .mca raw NBT AST & re-import.
    let dst_payload = AnvilRegion::read_chunk_payload_checked(&dst_world, 1, -1)
        .unwrap()
        .expect("exported vanilla chunk present");
    let mut dst_raw_nbt = Vec::new();
    ZlibDecoder::new(&dst_payload[..])
        .read_to_end(&mut dst_raw_nbt)
        .unwrap();
    assert_eq!(
        detect_storage_format(&dst_raw_nbt),
        StorageFormat::VanillaAnvil
    );

    let mut cursor = &dst_raw_nbt[..];
    let (root_name, NbtTag::Compound(dst_root)) = decode_named_nbt(&mut cursor).unwrap() else {
        panic!("exported payload must be a named NBT compound");
    };
    assert_eq!(root_name, "");
    assert_eq!(dst_root.get("DataVersion"), Some(&NbtTag::Int(4790)));
    assert_eq!(dst_root.get("InhabitedTime"), Some(&NbtTag::Long(42_000)));
    assert_eq!(dst_root.get("LastUpdate"), Some(&NbtTag::Long(99_999)));
    assert_eq!(
        dst_root.get("Heightmaps"),
        Some(&NbtTag::Compound(heightmaps))
    );
    assert_eq!(
        dst_root.get("structures"),
        Some(&NbtTag::Compound(structures))
    );
    assert_eq!(
        dst_root.get("CustomMeta"),
        Some(&NbtTag::Compound(custom_meta))
    );
    assert_eq!(
        dst_root.get("block_entities"),
        Some(&NbtTag::List(vec![NbtTag::Compound(be)]))
    );

    let NbtTag::List(dst_sections) = dst_root.get("sections").unwrap() else {
        panic!("sections list missing");
    };
    let sec4_out = dst_sections
        .iter()
        .find_map(|s| match s {
            NbtTag::Compound(m) if m.get("Y") == Some(&NbtTag::Byte(4)) => Some(m),
            _ => None,
        })
        .expect("section Y=4 must be present");
    assert_eq!(sec4_out.get("biomes"), Some(&NbtTag::Compound(biomes4)));
    assert_eq!(
        sec4_out.get("BlockLight"),
        Some(&NbtTag::ByteArray(vec![0x12; 2048]))
    );
    assert_eq!(
        sec4_out.get("SkyLight"),
        Some(&NbtTag::ByteArray(vec![-1i8; 2048]))
    );
    assert_eq!(
        sec4_out.get("CustomSecTag"),
        Some(&NbtTag::String("keep_sec_meta".into()))
    );

    // Re-import exported vanilla NBT and verify all mutated & untouched blocks.
    let reimported = ChunkColumn::decode_vanilla_nbt_chunk_with_metadata(&dst_raw_nbt).unwrap();
    assert_eq!(reimported.data_version, Some(4790));
    assert_eq!(reimported.column.get_block(0, 64, 0), mc_blocks::STONE);
    assert_eq!(reimported.column.get_block(1, 65, 1), wheat_age5);
    assert_eq!(reimported.column.get_block(2, 66, 2), mc_blocks::CHEST);
    assert_eq!(reimported.column.get_block(3, 64, 3), oak_log_z);
    assert_eq!(reimported.column.get_block(4, 64, 4), mc_blocks::LEVER + 1);

    let _ = std::fs::remove_dir_all(&tmp_root);
}

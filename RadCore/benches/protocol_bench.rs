use std::hint::black_box;

use bytes::BytesMut;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use rad_core::crypto::MinecraftCipher;
use rad_core::forwarding::{
    ForwardingData, decode_velocity_forwarding, encode_velocity_forwarding,
};
use rad_core::packet::handshake::Handshake;
use rad_core::prelude::*;
use uuid::Uuid;

fn bench_varint(c: &mut Criterion) {
    let mut group = c.benchmark_group("varint");

    group.bench_function("encode_25565", |b| {
        let mut buf = BytesMut::with_capacity(8);
        b.iter(|| {
            buf.clear();
            VarInt(black_box(25565)).encode(&mut buf).unwrap();
        });
    });

    let mut encoded = BytesMut::new();
    VarInt(25565).encode(&mut encoded).unwrap();
    let encoded_bytes = encoded.freeze();

    group.bench_function("decode_25565", |b| {
        b.iter(|| {
            let mut slice = black_box(&encoded_bytes[..]);
            black_box(VarInt::decode(&mut slice).unwrap())
        });
    });

    group.finish();
}

fn bench_frame_codec(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec");

    let hs = Handshake {
        protocol_version: VarInt(PROTOCOL_VERSION),
        server_address: "play.radbyte.xyz".into(),
        server_port: 25565,
        next_state: ConnectionState::Status,
    };

    group.bench_function("encode_handshake", |b| {
        let mut buf = BytesMut::with_capacity(64);
        b.iter(|| {
            buf.clear();
            encode_packet(black_box(&hs), &mut buf).unwrap();
        });
    });

    let mut wire = BytesMut::new();
    encode_packet(&hs, &mut wire).unwrap();
    let wire_template = wire.freeze();

    group.bench_function("decode_handshake_zero_copy", |b| {
        let mut scratch = BytesMut::with_capacity(wire_template.len());
        b.iter(|| {
            scratch.clear();
            scratch.extend_from_slice(&wire_template);
            let frame = decode_frame(&mut scratch, DEFAULT_MAX_PACKET_SIZE)
                .unwrap()
                .unwrap();
            black_box(frame.decode_as::<Handshake>().unwrap())
        });
    });

    group.bench_function("peek_handshake_next_state_zero_alloc", |b| {
        let mut scratch = BytesMut::with_capacity(wire_template.len());
        b.iter(|| {
            scratch.clear();
            scratch.extend_from_slice(&wire_template);
            let frame = decode_frame(&mut scratch, DEFAULT_MAX_PACKET_SIZE)
                .unwrap()
                .unwrap();
            black_box(Handshake::peek_next_state(&frame.payload).unwrap())
        });
    });

    let comp_cfg = CompressionConfig::new(256);
    let payload_1kb: Vec<u8> = (0..1024).map(|i| (i % 64) as u8).collect();
    let mut comp_wire = BytesMut::new();
    encode_frame_with_compression(0x27, &payload_1kb, &mut comp_wire, Some(comp_cfg.threshold))
        .unwrap();
    let comp_template = comp_wire.freeze();

    group.throughput(Throughput::Bytes(1024));
    group.bench_function("encode_compressed_1kb", |b| {
        let mut out = BytesMut::with_capacity(1200);
        b.iter(|| {
            out.clear();
            encode_frame_with_compression(
                0x27,
                black_box(&payload_1kb),
                &mut out,
                Some(comp_cfg.threshold),
            )
            .unwrap();
        });
    });

    group.bench_function("decode_compressed_1kb", |b| {
        let mut scratch = BytesMut::with_capacity(comp_template.len());
        b.iter(|| {
            scratch.clear();
            scratch.extend_from_slice(&comp_template);
            let frame = decode_frame_with_compression(
                &mut scratch,
                DEFAULT_MAX_PACKET_SIZE,
                Some(comp_cfg),
            )
            .unwrap()
            .unwrap();
            black_box(frame)
        });
    });

    group.finish();
}

fn bench_crypto(c: &mut Criterion) {
    let mut group = c.benchmark_group("crypto");
    group.throughput(Throughput::Bytes(1024));

    let secret = [0x42_u8; 16];
    let mut enc = MinecraftCipher::new(&secret);
    let mut dec = MinecraftCipher::new(&secret);
    let mut buf = vec![0xAB_u8; 1024];

    group.bench_function("aes_cfb8_encrypt_1kb", |b| {
        b.iter(|| {
            enc.encrypt(black_box(&mut buf));
        });
    });

    group.bench_function("aes_cfb8_decrypt_1kb", |b| {
        b.iter(|| {
            dec.decrypt(black_box(&mut buf));
        });
    });

    group.finish();
}

fn bench_forwarding(c: &mut Criterion) {
    let mut group = c.benchmark_group("forwarding");
    let secret = b"radbyte-benchmark-secret-key";
    let data = ForwardingData {
        address: "192.0.2.42".into(),
        uuid: Uuid::new_v3(&Uuid::NAMESPACE_DNS, b"Notch"),
        username: "Notch".into(),
        properties: Vec::new(),
    };

    group.bench_function("velocity_hmac_sign_and_verify", |b| {
        b.iter(|| {
            let signed = encode_velocity_forwarding(black_box(&data), secret).unwrap();
            black_box(decode_velocity_forwarding(&signed, secret).unwrap())
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_varint,
    bench_frame_codec,
    bench_crypto,
    bench_forwarding
);
criterion_main!(benches);

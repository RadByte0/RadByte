//! HAProxy `PROXY` protocol (v1 text & v2 binary) header parser.
//!
//! Allows `RadProxy` to recover the real client IP/port when placed behind
//! TCPShield, Cloudflare Spectrum, HAProxy, or nginx stream proxies.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use tokio::io::{AsyncRead, AsyncReadExt};

const V2_SIG: [u8; 12] = [
    0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
];
const V1_PREFIX: &[u8] = b"PROXY ";

/// Parses a HAProxy v1 or v2 header at the start of `buf`.
///
/// - `Ok(None)` if `buf` does not yet contain the complete header.
/// - `Ok(Some((real_addr, consumed)))` on success (`real_addr` is `None` for `UNKNOWN`/`LOCAL`).
/// - `Err(reason)` if the header is malformed.
pub fn parse_haproxy_header(
    buf: &[u8],
) -> Result<Option<(Option<SocketAddr>, usize)>, &'static str> {
    if buf.is_empty() {
        return Ok(None);
    }

    // Check v2 binary signature (16-byte fixed header + variable address payload).
    if buf[0] == V2_SIG[0] {
        let sig_check_len = buf.len().min(V2_SIG.len());
        if buf[..sig_check_len] != V2_SIG[..sig_check_len] {
            return Err("invalid HAProxy v2 signature");
        }
        if buf.len() < 16 {
            return Ok(None);
        }

        let ver_cmd = buf[12];
        if (ver_cmd & 0xF0) != 0x20 {
            return Err("unsupported HAProxy v2 version");
        }
        let cmd = ver_cmd & 0x0F;
        if cmd > 1 {
            return Err("invalid HAProxy v2 command");
        }

        let fam = buf[13];
        let payload_len = u16::from_be_bytes([buf[14], buf[15]]) as usize;
        let total_len = 16 + payload_len;
        if total_len > 512 {
            return Err("HAProxy v2 header too large");
        }
        if buf.len() < total_len {
            return Ok(None);
        }

        if cmd == 0 {
            // LOCAL healthcheck
            return Ok(Some((None, total_len)));
        }

        let addr_data = &buf[16..total_len];
        let parsed = match fam & 0xF0 {
            0x10 => {
                // AF_INET (IPv4): src_addr (4) + dst_addr (4) + src_port (2) + dst_port (2) = 12
                if addr_data.len() < 12 {
                    return Err("truncated HAProxy v2 IPv4 address");
                }
                let ip = Ipv4Addr::new(addr_data[0], addr_data[1], addr_data[2], addr_data[3]);
                let port = u16::from_be_bytes([addr_data[8], addr_data[9]]);
                Some(SocketAddr::new(IpAddr::V4(ip), port))
            }
            0x20 => {
                // AF_INET6 (IPv6): src_addr (16) + dst_addr (16) + src_port (2) + dst_port (2) = 36
                if addr_data.len() < 36 {
                    return Err("truncated HAProxy v2 IPv6 address");
                }
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&addr_data[..16]);
                let ip = Ipv6Addr::from(octets);
                let port = u16::from_be_bytes([addr_data[32], addr_data[33]]);
                Some(SocketAddr::new(IpAddr::V6(ip), port))
            }
            _ => None, // AF_UNSPEC
        };

        return Ok(Some((parsed, total_len)));
    }

    // Check v1 ASCII line ("PROXY TCP4 1.2.3.4 5.6.7.8 12345 25565\r\n").
    let prefix_check_len = buf.len().min(V1_PREFIX.len());
    if buf[..prefix_check_len] != V1_PREFIX[..prefix_check_len] {
        return Err("missing HAProxy PROXY header");
    }

    let max_v1 = buf.len().min(108);
    let Some(cr_pos) = buf[..max_v1].windows(2).position(|w| w == b"\r\n") else {
        if buf.len() >= 108 {
            return Err("HAProxy v1 header exceeds 108 bytes");
        }
        return Ok(None);
    };

    let line = core::str::from_utf8(&buf[..cr_pos]).map_err(|_| "invalid UTF-8 in HAProxy v1")?;
    let mut parts = line.split(' ');
    let _proxy = parts.next();
    let proto = parts.next().ok_or("missing HAProxy v1 protocol")?;
    let consumed = cr_pos + 2;

    if proto == "UNKNOWN" {
        return Ok(Some((None, consumed)));
    }
    if proto != "TCP4" && proto != "TCP6" {
        return Err("unsupported HAProxy v1 protocol");
    }

    let src_ip: IpAddr = parts
        .next()
        .ok_or("missing HAProxy v1 src ip")?
        .parse()
        .map_err(|_| "invalid HAProxy v1 src ip")?;
    let dst_ip: IpAddr = parts
        .next()
        .ok_or("missing HAProxy v1 dst ip")?
        .parse()
        .map_err(|_| "invalid HAProxy v1 dst ip")?;
    if (proto == "TCP4" && (!src_ip.is_ipv4() || !dst_ip.is_ipv4()))
        || (proto == "TCP6" && (!src_ip.is_ipv6() || !dst_ip.is_ipv6()))
    {
        return Err("HAProxy v1 address family mismatch");
    }
    let src_port = parse_v1_port(parts.next().ok_or("missing HAProxy v1 src port")?)?;
    let _dst_port = parse_v1_port(parts.next().ok_or("missing HAProxy v1 dst port")?)?;
    if parts.next().is_some() {
        return Err("unexpected HAProxy v1 fields");
    }

    Ok(Some((Some(SocketAddr::new(src_ip, src_port)), consumed)))
}

fn parse_v1_port(value: &str) -> Result<u16, &'static str> {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err("invalid HAProxy v1 port");
    }
    value.parse().map_err(|_| "invalid HAProxy v1 port")
}

/// Reads a HAProxy v1/v2 header from `stream` into `buf`, returning the real client [`SocketAddr`]
/// and leaving any subsequent protocol bytes shifted to the front of `buf`.
pub async fn read_haproxy_peer<S: AsyncRead + Unpin>(
    stream: &mut S,
    buf: &mut [u8],
    filled: &mut usize,
    fallback: SocketAddr,
) -> io::Result<SocketAddr> {
    if *filled > buf.len() || buf.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid HAProxy receive buffer",
        ));
    }
    loop {
        if *filled > 0 {
            match parse_haproxy_header(&buf[..*filled]) {
                Ok(Some((addr_opt, consumed))) => {
                    let rem = *filled - consumed;
                    if rem > 0 {
                        buf.copy_within(consumed..*filled, 0);
                    }
                    *filled = rem;
                    return Ok(addr_opt.unwrap_or(fallback));
                }
                Ok(None) => {
                    if *filled == buf.len() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "HAProxy header exceeds buffer",
                        ));
                    }
                }
                Err(msg) => {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, msg));
                }
            }
        }

        let n = stream.read(&mut buf[*filled..]).await?;
        if n == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        *filled += n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_v1_endpoints_and_extra_fields() {
        for raw in [
            "PROXY TCP4 203.0.113.9 invalid 123 25565\r\n",
            "PROXY TCP4 ::1 127.0.0.1 123 25565\r\n",
            "PROXY TCP6 ::1 127.0.0.1 123 25565\r\n",
            "PROXY TCP4 203.0.113.9 127.0.0.1 123\r\n",
            "PROXY TCP4 203.0.113.9 127.0.0.1 123 65536\r\n",
            "PROXY TCP4 203.0.113.9 127.0.0.1 +123 25565\r\n",
            "PROXY TCP4 203.0.113.9 127.0.0.1 0123 25565\r\n",
            "PROXY TCP4 203.0.113.9 127.0.0.1 123 25565 extra\r\n",
        ] {
            assert!(parse_haproxy_header(raw.as_bytes()).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn accepts_v1_ipv6_unknown_and_partial_headers() {
        assert_eq!(
            parse_haproxy_header(b"PROXY TCP6 ::1 ::1 0 65535\r\n")
                .unwrap()
                .unwrap()
                .0,
            Some("[::1]:0".parse().unwrap())
        );
        assert_eq!(
            parse_haproxy_header(b"PROXY UNKNOWN ignored\r\n")
                .unwrap()
                .unwrap()
                .0,
            None
        );
        assert!(
            parse_haproxy_header(b"PROXY TCP4 127.0.0.1")
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn preserves_following_protocol_bytes_and_rejects_invalid_buffer_state() {
        let fallback = "127.0.0.1:25565".parse().unwrap();
        let mut input = b"PROXY TCP4 203.0.113.9 127.0.0.1 123 25565\r\nMC".as_slice();
        let mut buffer = [0; 128];
        let mut filled = 0;
        assert_eq!(
            read_haproxy_peer(&mut input, &mut buffer, &mut filled, fallback)
                .await
                .unwrap(),
            "203.0.113.9:123".parse().unwrap()
        );
        assert_eq!(&buffer[..filled], b"MC");
        filled = buffer.len() + 1;
        assert!(
            read_haproxy_peer(&mut input, &mut buffer, &mut filled, fallback)
                .await
                .is_err()
        );
    }

    #[test]
    fn parses_haproxy_v1_tcp4_with_trailing_bytes() {
        let raw = b"PROXY TCP4 203.0.113.9 10.0.0.1 54321 25565\r\n\x10\x00";
        let (addr, consumed) = parse_haproxy_header(raw).unwrap().unwrap();
        assert_eq!(addr, Some("203.0.113.9:54321".parse().unwrap()));
        assert_eq!(&raw[consumed..], b"\x10\x00");
    }

    #[test]
    fn parses_haproxy_v2_ipv4_with_trailing_bytes() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&V2_SIG);
        raw.push(0x21); // v2 + PROXY
        raw.push(0x11); // AF_INET + STREAM
        raw.extend_from_slice(&12_u16.to_be_bytes());
        raw.extend_from_slice(&[198, 51, 100, 7]); // src ip
        raw.extend_from_slice(&[127, 0, 0, 1]); // dst ip
        raw.extend_from_slice(&44444_u16.to_be_bytes()); // src port
        raw.extend_from_slice(&25565_u16.to_be_bytes()); // dst port
        raw.extend_from_slice(b"MC");

        let (addr, consumed) = parse_haproxy_header(&raw).unwrap().unwrap();
        assert_eq!(addr, Some("198.51.100.7:44444".parse().unwrap()));
        assert_eq!(&raw[consumed..], b"MC");
    }
}

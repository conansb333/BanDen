//! Lightweight DPI: DNS message parsing and TLS SNI extraction.
//!
//! Used by the forwarder to classify the target's flows per application.
//! Only the minimum is parsed: DNS question/answer names and A-record
//! addresses, and TLS ClientHello SNI. Nothing is decrypted - DNS names
//! and TLS SNI are plaintext by protocol design.
//!
//! All functions are pure and unit-tested; the forwarder only calls them.

use std::net::Ipv4Addr;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct DnsInfo {
    /// Names asked by the client (queries).
    pub queries: Vec<String>,
    /// Names that appear in the answer section (owners + CNAME chains).
    pub answer_names: Vec<String>,
    /// IPv4 addresses found in A records.
    pub answer_ips: Vec<Ipv4Addr>,
    /// True when this is a response (QR bit set).
    pub is_response: bool,
}

/// Parse a DNS message from a UDP payload.
pub fn parse_dns(payload: &[u8]) -> Option<DnsInfo> {
    if payload.len() < 12 {
        return None;
    }
    let flags = u16::from_be_bytes([payload[2], payload[3]]);
    let is_response = flags & 0x8000 != 0;
    let qd = u16::from_be_bytes([payload[4], payload[5]]) as usize;
    let an = u16::from_be_bytes([payload[6], payload[7]]) as usize;

    let mut info = DnsInfo {
        is_response,
        ..Default::default()
    };

    let mut off = 12usize;
    for _ in 0..qd {
        let (name, noff) = read_name(payload, off, 0)?;
        if !name.is_empty() {
            info.queries.push(name);
        }
        off = noff + 4; // qtype + qclass
    }
    for _ in 0..an {
        let (name, noff) = read_name(payload, off, 0)?;
        off = noff;
        if off + 10 > payload.len() {
            break;
        }
        let rtype = u16::from_be_bytes([payload[off], payload[off + 1]]);
        let rdlen = u16::from_be_bytes([payload[off + 8], payload[off + 9]]) as usize;
        let rd = off + 10;
        if rd + rdlen > payload.len() {
            break;
        }
        if !name.is_empty() {
            info.answer_names.push(name);
        }
        if rtype == 1 && rdlen == 4 {
            info.answer_ips.push(Ipv4Addr::new(
                payload[rd],
                payload[rd + 1],
                payload[rd + 2],
                payload[rd + 3],
            ));
        }
        off = rd + rdlen;
    }
    Some(info)
}

/// Read a (possibly compressed) domain name. Returns (name, offset after).
fn read_name(payload: &[u8], mut off: usize, depth: u8) -> Option<(String, usize)> {
    if depth > 8 {
        return None;
    }
    let mut labels: Vec<String> = Vec::new();
    let mut jumped = false;
    let mut next_after = off; // offset following the name at the original site
    let mut guard = 0usize;
    loop {
        guard += 1;
        if guard > 128 || off >= payload.len() {
            return None;
        }
        let len = payload[off] as usize;
        if len & 0xC0 == 0xC0 {
            if off + 1 >= payload.len() {
                return None;
            }
            let ptr = ((len & 0x3F) << 8) | payload[off + 1] as usize;
            if !jumped {
                next_after = off + 2;
                jumped = true;
            }
            off = ptr;
            continue;
        }
        if len == 0 {
            if !jumped {
                next_after = off + 1;
            }
            break;
        }
        if off + 1 + len > payload.len() {
            return None;
        }
        labels.push(String::from_utf8_lossy(&payload[off + 1..off + 1 + len]).into_owned());
        off += 1 + len;
    }
    Some((labels.join("."), next_after))
}

/// Extract the SNI server name from a TCP payload carrying a TLS
/// ClientHello. Returns None when the payload is not a recognizable
/// ClientHello with an SNI extension.
pub fn parse_sni(payload: &[u8]) -> Option<String> {
    // TLS record: type(1)=0x16 handshake, version(2), length(2)
    if payload.len() < 6 || payload[0] != 0x16 || payload[1] != 0x03 {
        return None;
    }
    let rec_len = u16::from_be_bytes([payload[3], payload[4]]) as usize;
    let end = (5 + rec_len).min(payload.len());

    // Handshake: type(1)=0x01 ClientHello, length(3)
    if payload[5] != 0x01 || 5 + 4 > end {
        return None;
    }
    let hs_len = ((payload[6] as usize) << 16) | ((payload[7] as usize) << 8) | payload[8] as usize;
    let hs_end = (5 + 4 + hs_len).min(end);
    let mut off = 9; // after record header + handshake header

    off += 2; // client version
    off += 32; // random
    if off >= hs_end {
        return None;
    }
    let sid = payload[off] as usize; // session id
    off += 1 + sid;
    if off + 2 > hs_end {
        return None;
    }
    let cs = u16::from_be_bytes([payload[off], payload[off + 1]]) as usize;
    off += 2 + cs;
    if off >= hs_end {
        return None;
    }
    let comp = payload[off] as usize;
    off += 1 + comp;
    if off + 2 > hs_end {
        return None;
    }
    let ext_total = u16::from_be_bytes([payload[off], payload[off + 1]]) as usize;
    off += 2;
    let ext_end = (off + ext_total).min(hs_end);

    while off + 4 <= ext_end {
        let etype = u16::from_be_bytes([payload[off], payload[off + 1]]);
        let elen = u16::from_be_bytes([payload[off + 2], payload[off + 3]]) as usize;
        let edata = off + 4;
        if edata + elen > ext_end {
            break;
        }
        if etype == 0x0000 {
            // ServerNameList: list_len(2), then ServerName entries:
            // name_type(1)=0 host_name, name_len(2), name.
            let edata_end = edata + elen;
            let entry = edata + 2; // skip server_name_list_length
            if entry + 3 > edata_end {
                return None;
            }
            if payload[entry] != 0 {
                return None; // only host_name type exists
            }
            let nlen = u16::from_be_bytes([payload[entry + 1], payload[entry + 2]]) as usize;
            if entry + 3 + nlen > edata_end {
                return None;
            }
            return Some(
                String::from_utf8_lossy(&payload[entry + 3..entry + 3 + nlen])
                    .trim_end_matches('.')
                    .to_owned(),
            );
        }
        off = edata + elen;
    }
    None
}

/// TCP/UDP transport facts extracted from an ethernet+IPv4 frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transport {
    pub ip_src: [u8; 4],
    pub ip_dst: [u8; 4],
    pub proto: u8,
    pub src_port: u16,
    pub dst_port: u16,
    /// Byte offset of the L4 payload within the frame (None for non-TCP/UDP).
    pub payload_off: Option<usize>,
}

/// Extract transport facts from an ethernet+IPv4 frame.
pub fn transport_of(frame: &[u8]) -> Option<Transport> {
    if frame.len() < 34 || frame[12] != 0x08 || frame[13] != 0x00 {
        return None;
    }
    let ip_h = ((frame[14] & 0x0F) as usize) * 4;
    if ip_h < 20 || 14 + ip_h > frame.len() {
        return None;
    }
    let proto = frame[14 + 9];
    let ip_src = [frame[26], frame[27], frame[28], frame[29]];
    let ip_dst = [frame[30], frame[31], frame[32], frame[33]];
    let l4 = 14 + ip_h;
    if proto != 17 && proto != 6 {
        return Some(Transport {
            ip_src,
            ip_dst,
            proto,
            src_port: 0,
            dst_port: 0,
            payload_off: None,
        });
    }
    if l4 + 20 > frame.len() {
        return None;
    }
    let src_port = u16::from_be_bytes([frame[l4], frame[l4 + 1]]);
    let dst_port = u16::from_be_bytes([frame[l4 + 2], frame[l4 + 3]]);
    let payload_off = if proto == 17 {
        Some(l4 + 8)
    } else {
        let doff = ((frame[l4 + 12] >> 4) as usize) * 4;
        Some(l4 + doff.max(20))
    };
    Some(Transport {
        ip_src,
        ip_dst,
        proto,
        src_port,
        dst_port,
        payload_off,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a DNS response: query example.com A, answer example.com A 93.184.216.34
    fn build_dns_response() -> Vec<u8> {
        let mut p = vec![0x12, 0x34]; // id
        p.extend([0x81, 0x80]); // response, recursion
        p.extend([0x00, 0x01]); // qd=1
        p.extend([0x00, 0x01]); // an=1
        p.extend([0x00, 0x00, 0x00, 0x00]); // ns=ar=0
                                            // question: example.com A IN
        p.push(7);
        p.extend(b"example");
        p.push(3);
        p.extend(b"com");
        p.push(0);
        p.extend([0x00, 0x01, 0x00, 0x01]);
        // answer: ptr to question name(0xC00C), A IN ttl=300 rdlen=4
        p.extend([0xC0, 0x0C]);
        p.extend([0x00, 0x01, 0x00, 0x01]);
        p.extend([0x00, 0x00, 0x01, 0x2C]);
        p.extend([0x00, 0x04]);
        p.extend([93, 184, 216, 34]);
        p
    }

    #[test]
    fn parses_dns_response_with_compression() {
        let info = parse_dns(&build_dns_response()).unwrap();
        assert!(info.is_response);
        assert_eq!(info.queries, vec!["example.com"]);
        assert!(info.answer_names.iter().any(|n| n == "example.com"));
        assert_eq!(info.answer_ips, vec![Ipv4Addr::new(93, 184, 216, 34)]);
    }

    #[test]
    fn parses_dns_query() {
        // same question section, but flags = standard query (QR=0), an=0
        let mut p = vec![
            0xAB, 0xCD, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        p.push(4);
        p.extend(b"wa");
        p.push(0);
        // actually type the real query name: wa.me
        p.clear();
        p.extend([
            0xAB, 0xCD, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        p.push(2);
        p.extend(b"wa");
        p.push(2);
        p.extend(b"me");
        p.push(0);
        p.extend([0x00, 0x01, 0x00, 0x01]);
        let info = parse_dns(&p).unwrap();
        assert!(!info.is_response);
        assert_eq!(info.queries, vec!["wa.me"]);
    }

    #[test]
    fn garbage_returns_none() {
        assert!(parse_dns(&[0u8; 4]).is_none());
        assert!(parse_dns(&[]).is_none());
    }

    /// Build a TLS ClientHello with SNI "rr3---sn-1.googlevideo.com".
    fn build_client_hello(sni: &str) -> Vec<u8> {
        let name = sni.as_bytes();
        // SNI extension
        let mut sni_ext = Vec::new();
        sni_ext.extend(((name.len() + 3) as u16).to_be_bytes()); // list len
        sni_ext.push(0); // host_name
        sni_ext.extend((name.len() as u16).to_be_bytes());
        sni_ext.extend(name);

        let mut hs = Vec::new();
        hs.extend([0x03, 0x03]); // client version TLS1.2
        hs.extend([0xAA; 32]); // random
        hs.push(0); // session id len
        hs.extend([0x00, 0x02]); // cipher suites len
        hs.extend([0x13, 0x01]); // one cipher
        hs.push(1);
        hs.push(0); // compression methods
                    // extensions block: SNI extension = type(0x0000) + len + sni_ext
        hs.extend((sni_ext.len() as u16 + 4).to_be_bytes());
        hs.extend([0x00, 0x00]);
        hs.extend((sni_ext.len() as u16).to_be_bytes());
        hs.extend(sni_ext);

        let mut rec = Vec::new();
        rec.push(0x16); // handshake
        rec.extend([0x03, 0x01]); // record version TLS1.0
        rec.extend(((hs.len() + 4) as u16).to_be_bytes());
        rec.push(0x01); // ClientHello
        rec.extend((hs.len() as u32).to_be_bytes()[1..4].iter().copied());
        rec.extend(hs);
        rec
    }

    #[test]
    fn parses_sni_from_client_hello() {
        let payload = build_client_hello("rr3---sn-1.googlevideo.com");
        assert_eq!(
            parse_sni(&payload).as_deref(),
            Some("rr3---sn-1.googlevideo.com")
        );
        let payload = build_client_hello("media-fb.om.whatsapp.net");
        assert_eq!(
            parse_sni(&payload).as_deref(),
            Some("media-fb.om.whatsapp.net")
        );
    }

    #[test]
    fn non_client_hello_returns_none() {
        // ServerHello (0x02) has no SNI.
        let mut payload = build_client_hello("example.com");
        payload[5] = 0x02;
        assert_eq!(parse_sni(&payload), None);
        assert_eq!(parse_sni(&[0u8; 10]), None);
    }

    #[test]
    fn transport_extraction() {
        let _ = build_dns_response(); // exercise the builder; frame is hand-built below
        let mut frame = vec![0xff; 12]; // eth dst+src
        frame.extend([0x08, 0x00]); // ethertype IPv4
        frame.extend([0x45, 0x00, 0x00, 0x00]); // v4 ihl5, len placeholder
        frame.extend([0x00, 0x00, 0x00, 0x00, 0x40, 0x11, 0x00, 0x00]); // ttl, proto 17, csum
        frame.extend([192, 168, 8, 1]); // src
        frame.extend([192, 168, 8, 99]); // dst
        frame.extend([0x00, 0x35, 0xEA, 0xDB, 0x00, 0x00, 0x00, 0x00]); // udp 53->EA DB
        frame.extend(build_dns_response());
        let t = transport_of(&frame).unwrap();
        assert_eq!(t.proto, 17);
        assert_eq!(t.src_port, 53);
        assert_eq!(t.ip_src, [192, 168, 8, 1]);
        assert_eq!(t.ip_dst, [192, 168, 8, 99]);
        let dns = parse_dns(&frame[t.payload_off.unwrap()..]).unwrap();
        assert!(dns.is_response);
        assert_eq!(dns.answer_ips, vec![Ipv4Addr::new(93, 184, 216, 34)]);
    }
}

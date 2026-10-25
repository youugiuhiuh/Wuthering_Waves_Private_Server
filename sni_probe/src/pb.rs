//! `DomainList` protobuf（`proto/sni.proto`）手工编解码。
//!
//! 只有两个 `repeated string` 字段，长度前缀语义固定，因此不需要 codegen；
//! 手写实现还能免掉 protoc/prost-build 的构建期依赖。
//!
//! ```proto
//! message DomainList {
//!     repeated string pq_domains = 1;
//!     repeated string classic_domains = 2;
//! }
//! ```

const TAG_PQ: u8 = 0x0A; // field 1, wire type 2 (length-delimited)
const TAG_CLASSIC: u8 = 0x12; // field 2, wire type 2

/// 写入 varint。
fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_string_field(out: &mut Vec<u8>, tag: u8, s: &str) {
    if s.is_empty() {
        return;
    }
    out.push(tag);
    put_varint(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

/// 编码为 `DomainList`。
pub fn encode(pq_domains: &[String], classic_domains: &[String]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + pq_domains.len() * 24 + classic_domains.len() * 24);
    for d in pq_domains {
        put_string_field(&mut out, TAG_PQ, d);
    }
    for d in classic_domains {
        put_string_field(&mut out, TAG_CLASSIC, d);
    }
    out
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        let b = *buf.get(*pos).ok_or("truncated varint")?;
        *pos += 1;
        result |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err("varint too long".into());
        }
    }
}

/// 解码 `DomainList`，返回 `(pq_domains, classic_domains)`。
pub fn decode(buf: &[u8]) -> Result<(Vec<String>, Vec<String>), String> {
    let mut pq = Vec::new();
    let mut classic = Vec::new();
    let mut pos = 0usize;

    while pos < buf.len() {
        let tag = buf[pos];
        pos += 1;
        match tag {
            TAG_PQ | TAG_CLASSIC => {
                let len = read_varint(buf, &mut pos)? as usize;
                let end = pos.checked_add(len).ok_or("length overflow")?;
                let slice = buf.get(pos..end).ok_or("truncated string")?;
                let s = String::from_utf8(slice.to_vec()).map_err(|e| e.to_string())?;
                if tag == TAG_PQ {
                    pq.push(s);
                } else {
                    classic.push(s);
                }
                pos = end;
            }
            // 未知字段：跳过（前向兼容）
            other => {
                let wire_type = other & 0x07;
                match wire_type {
                    0 => {
                        read_varint(buf, &mut pos)?;
                    }
                    2 => {
                        let len = read_varint(buf, &mut pos)? as usize;
                        pos = pos.checked_add(len).ok_or("length overflow")?;
                    }
                    5 => pos += 4,
                    1 => pos += 8,
                    _ => return Err(format!("unsupported wire type {wire_type}")),
                }
                if pos > buf.len() {
                    return Err("truncated unknown field".into());
                }
            }
        }
    }

    Ok((pq, classic))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_both_pools() {
        let pq = vec!["a.com".to_string(), "b.com".to_string()];
        let classic = vec!["c.com".to_string()];
        let encoded = encode(&pq, &classic);
        let (got_pq, got_classic) = decode(&encoded).unwrap();
        assert_eq!(got_pq, pq);
        assert_eq!(got_classic, classic);
    }

    #[test]
    fn encodes_field_tags_per_proto_schema() {
        let encoded = encode(&["x".to_string()], &["y".to_string()]);
        // field1(wire2)=0x0A len=1 'x'  field2(wire2)=0x12 len=1 'y'
        assert_eq!(encoded, vec![0x0A, 0x01, b'x', 0x12, 0x01, b'y']);
    }

    #[test]
    fn handles_empty_and_skips_empty_strings() {
        assert!(encode(&[], &[]).is_empty());
        assert!(encode(&[String::new()], &[]).is_empty());
        assert_eq!(decode(&[]).unwrap(), (vec![], vec![]));
    }

    #[test]
    fn parses_long_string_length_prefix() {
        // 200 字节域名触发两字节 varint 长度
        let long = "a".repeat(200);
        let encoded = encode(std::slice::from_ref(&long), &[]);
        let (pq, _) = decode(&encoded).unwrap();
        assert_eq!(pq, vec![long]);
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(decode(&[0x0A, 0x05, b'a']).is_err());
        assert!(decode(&[0x0A, 0xFF]).is_err());
    }

    #[test]
    fn skips_unknown_fields_for_forward_compat() {
        // field3 varint = 0x18 0x2A，应被跳过
        let mut buf = encode(&["a.com".to_string()], &[]);
        buf.extend_from_slice(&[0x18, 0x2A]);
        let (pq, classic) = decode(&buf).unwrap();
        assert_eq!(pq, vec!["a.com".to_string()]);
        assert!(classic.is_empty());
    }
}

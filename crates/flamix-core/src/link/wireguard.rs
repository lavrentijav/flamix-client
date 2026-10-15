//! `wireguard://` / `wg://` links (sing-box / wireguard-go style).

use super::{encode_userinfo, finish_link, make_server, parse_parts, parse_userinfo, Query};
use crate::error::Error;
use crate::model::{Credentials, Security, Server, Transport};
use crate::Result;

pub(super) fn parse(rest: &str, raw: &str, scope: &str) -> Result<Server> {
    let parts = parse_parts(rest)?;
    let private_key = parts
        .userinfo
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Parse("wireguard: missing private key".into()))?;
    let peer_public_key = parts
        .get("publickey")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Parse("wireguard: missing publickey".into()))?
        .to_string();
    let port = parts.port()?;

    let local_address = parts
        .get("address")
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let reserved = parts.get("reserved").and_then(parse_reserved);
    let mtu = parts.get("mtu").and_then(|s| s.parse().ok());
    let traffic = parts.get("userinfo").and_then(parse_userinfo);

    let name = parts.remark(port);
    let pre_shared_key = parts
        .get("presharedkey")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(make_server(
        scope,
        raw,
        parts.host,
        port,
        Credentials::Wireguard {
            private_key,
            peer_public_key,
            pre_shared_key,
            local_address,
            reserved,
            mtu,
        },
        Transport::default(),
        Security::None,
        name,
        traffic,
    ))
}

fn parse_reserved(s: &str) -> Option<[u8; 3]> {
    let mut out = [0u8; 3];
    let mut n = 0;
    for part in s.split(',') {
        if n >= 3 {
            break;
        }
        out[n] = part.trim().parse().ok()?;
        n += 1;
    }
    (n == 3).then_some(out)
}

pub(super) fn to_link(server: &Server) -> String {
    let (private_key, public_key, pre_shared_key, address, reserved, mtu) =
        match &server.credentials {
            Credentials::Wireguard {
                private_key,
                peer_public_key,
                pre_shared_key,
                local_address,
                reserved,
                mtu,
            } => (
                private_key.clone(),
                peer_public_key.clone(),
                pre_shared_key.clone(),
                local_address.join(","),
                *reserved,
                *mtu,
            ),
            _ => (
                String::new(),
                String::new(),
                None,
                String::new(),
                None,
                None,
            ),
        };

    let mut query = Query::new();
    query.add("publickey", &public_key);
    query.add("presharedkey", pre_shared_key.as_deref().unwrap_or(""));
    query.add("address", &address);
    if let Some(reserved) = reserved {
        query.add(
            "reserved",
            &format!("{},{},{}", reserved[0], reserved[1], reserved[2]),
        );
    }
    if let Some(mtu) = mtu {
        query.add("mtu", &mtu.to_string());
    }
    if let Some(traffic) = &server.traffic {
        query.add("userinfo", &super::vless::format_userinfo(traffic));
    }

    let base = format!(
        "wireguard://{}@{}",
        encode_userinfo(&private_key),
        super::format_host_port(&server.address, server.port)
    );
    finish_link(base, &query, &server.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::parse_link;

    #[test]
    fn wg_full() {
        let link = "wireguard://eDg4%2Bc3d4aGVscGVy@wg.example.com:51820?publickey=Zm9vYmFy&presharedkey=c2hhcmVk&address=10.0.0.2%2F32,fd00::2%2F128&reserved=1,2,3&mtu=1280#WG";
        let s = parse_link(link, "local").unwrap();
        assert_eq!(s.address, "wg.example.com");
        assert_eq!(s.port, 51820);
        match &s.credentials {
            Credentials::Wireguard {
                private_key,
                peer_public_key,
                pre_shared_key,
                local_address,
                reserved,
                mtu,
            } => {
                assert_eq!(private_key, "eDg4+c3d4aGVscGVy");
                assert_eq!(peer_public_key, "Zm9vYmFy");
                assert_eq!(pre_shared_key.as_deref(), Some("c2hhcmVk"));
                assert_eq!(local_address, &["10.0.0.2/32", "fd00::2/128"]);
                assert_eq!(reserved.as_ref(), Some(&[1u8, 2, 3]));
                assert_eq!(mtu.as_ref(), Some(&1280u32));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn wg_simple_scheme() {
        let s = parse_link("wg://key@1.2.3.4:51820?publickey=pub#w", "local").unwrap();
        assert_eq!(s.port, 51820);
        assert_eq!(s.name, "w");
    }

    #[test]
    fn wg_roundtrip() {
        let link = "wireguard://a2V5@wg.example.com:51820?publickey=cHVi&presharedkey=cHNr&address=10.0.0.2%2F32&reserved=4,5,6&mtu=1420#RT";
        let a = parse_link(link, "local").unwrap();
        let b = parse_link(&to_link(&a), "local").unwrap();
        assert_eq!(a.credentials, b.credentials);
        assert_eq!(a.address, b.address);
        assert_eq!(a.name, b.name);
    }
}

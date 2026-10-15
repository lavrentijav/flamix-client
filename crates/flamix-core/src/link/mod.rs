//! Share links <-> [`Server`]. OWNER: task T01.
//!
//! Supported schemes: `vless://`, `vmess://` (v2rayN base64 JSON and the URI form),
//! `trojan://`, `ss://` (SIP002 + legacy base64), `hysteria2://` / `hy2://`, `tuic://`,
//! `wireguard://` / `wg://`, `socks://` / `socks5://`, `http://` / `https://` (proxy links).

use crate::model::Server;
use crate::Result;

/// Parse one share link. `scope` is passed to [`Server::compute_id`]
/// (subscription id or "local"). Leading/trailing whitespace is ignored.
pub fn parse_link(link: &str, scope: &str) -> Result<Server> {
    let _ = (link, scope);
    todo!("T01")
}

/// Parse many links separated by newlines (also `\r\n`, spaces are not separators).
/// Lines that fail to parse are skipped and reported in the second element as (line, error).
pub fn parse_links(text: &str, scope: &str) -> (Vec<Server>, Vec<(String, String)>) {
    let _ = (text, scope);
    todo!("T01")
}

/// Export a server back to a share link (round-trips with [`parse_link`]).
pub fn to_link(server: &Server) -> String {
    let _ = server;
    todo!("T01")
}

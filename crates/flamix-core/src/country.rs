//! Flag emoji <-> ISO 3166-1 alpha-2. OWNER: task T01.

/// Find the first regional-indicator flag emoji in `name` and return its code ("DE").
/// Falls back to country names / codes in the text (Russian and English: "Германия",
/// "Germany", "DE-1", "[NL]"), `None` if nothing found.
pub fn detect_country(name: &str) -> Option<String> {
    let _ = name;
    todo!("T01")
}

/// "DE" -> "🇩🇪".
pub fn flag_emoji(code: &str) -> String {
    let _ = code;
    todo!("T01")
}

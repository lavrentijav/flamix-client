//! Flag emoji <-> ISO 3166-1 alpha-2. OWNER: task T01.

/// Find the first regional-indicator flag emoji in `name` and return its code ("DE").
/// Falls back to country names / codes in the text (Russian and English: "Германия",
/// "Germany", "DE-1", "[NL]"), `None` if nothing found.
pub fn detect_country(name: &str) -> Option<String> {
    if let Some(code) = detect_flag(name) {
        return Some(code);
    }
    let lower = name.to_lowercase();
    for (code, aliases) in COUNTRIES {
        for alias in *aliases {
            if alias.contains(' ') {
                if lower.contains(alias.to_lowercase().as_str()) {
                    return Some((*code).to_string());
                }
            } else if lower
                .split(|c: char| !c.is_alphanumeric())
                .any(|token| token == alias.to_lowercase().as_str())
            {
                return Some((*code).to_string());
            }
        }
    }
    for token in name.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.len() == 2 && token.chars().all(|c| c.is_ascii_uppercase()) && is_iso_code(token) {
            return Some(token.to_string());
        }
    }
    None
}

/// "DE" -> "🇩🇪".
pub fn flag_emoji(code: &str) -> String {
    let bytes = code.as_bytes();
    if bytes.len() != 2 || !bytes[0].is_ascii_alphabetic() || !bytes[1].is_ascii_alphabetic() {
        return String::new();
    }
    let hi = 0x1F1E6 + (bytes[0].to_ascii_uppercase() - b'A') as u32;
    let lo = 0x1F1E6 + (bytes[1].to_ascii_uppercase() - b'A') as u32;
    match (char::from_u32(hi), char::from_u32(lo)) {
        (Some(hi), Some(lo)) => {
            let mut s = String::new();
            s.push(hi);
            s.push(lo);
            s
        }
        _ => String::new(),
    }
}

fn detect_flag(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    for pair in chars.windows(2) {
        if let (Some(a), Some(b)) = (regional_to_letter(pair[0]), regional_to_letter(pair[1])) {
            return Some(format!("{a}{b}"));
        }
    }
    None
}

fn regional_to_letter(c: char) -> Option<char> {
    let n = c as u32;
    if (0x1F1E6..=0x1F1FF).contains(&n) {
        char::from_u32(b'A' as u32 + (n - 0x1F1E6))
    } else {
        None
    }
}

fn is_iso_code(token: &str) -> bool {
    ISO_CODES.split_ascii_whitespace().any(|c| c == token)
}

/// ~70 common countries: ISO code + English / Russian aliases.
const COUNTRIES: &[(&str, &[&str])] = &[
    ("DE", &["germany", "германия", "немецкий"]),
    (
        "US",
        &[
            "united states",
            "usa",
            "america",
            "сша",
            "америка",
            "американский",
        ],
    ),
    (
        "GB",
        &[
            "united kingdom",
            "britain",
            "england",
            "великобритания",
            "англия",
            "британия",
        ],
    ),
    ("NL", &["netherlands", "holland", "нидерланды", "голландия"]),
    ("FR", &["france", "франция"]),
    ("FI", &["finland", "финляндия"]),
    ("SE", &["sweden", "швеция"]),
    ("NO", &["norway", "норвегия"]),
    ("DK", &["denmark", "дания"]),
    ("PL", &["poland", "польша"]),
    ("CZ", &["czechia", "czech", "чехия"]),
    ("AT", &["austria", "австрия"]),
    ("CH", &["switzerland", "швейцария"]),
    ("IT", &["italy", "италия"]),
    ("ES", &["spain", "испания"]),
    ("PT", &["portugal", "португалия"]),
    ("IE", &["ireland", "ирландия"]),
    ("BE", &["belgium", "бельгия"]),
    ("LU", &["luxembourg", "люксембург"]),
    ("RO", &["romania", "румыния"]),
    ("BG", &["bulgaria", "болгария"]),
    ("GR", &["greece", "греция"]),
    ("HU", &["hungary", "венгрия"]),
    ("SK", &["slovakia", "словакия"]),
    ("SI", &["slovenia", "словения"]),
    ("HR", &["croatia", "хорватия"]),
    ("RS", &["serbia", "сербия"]),
    ("UA", &["ukraine", "украина"]),
    ("BY", &["belarus", "беларусь", "белоруссия"]),
    ("RU", &["russia", "россия"]),
    ("MD", &["moldova", "молдова", "молдавия"]),
    ("LT", &["lithuania", "литва"]),
    ("LV", &["latvia", "латвия"]),
    ("EE", &["estonia", "эстония"]),
    ("GE", &["georgia", "грузия"]),
    ("AM", &["armenia", "армения"]),
    ("AZ", &["azerbaijan", "азербайджан"]),
    ("KZ", &["kazakhstan", "казахстан"]),
    ("UZ", &["uzbekistan", "узбекистан"]),
    ("TR", &["turkey", "турция"]),
    ("IL", &["israel", "израиль"]),
    ("AE", &["emirates", "оаэ", "эмираты"]),
    ("SA", &["saudi", "саудовская"]),
    ("IN", &["india", "индия"]),
    ("CN", &["china", "китай"]),
    ("JP", &["japan", "япония"]),
    ("KR", &["korea", "корея"]),
    ("SG", &["singapore", "сингапур"]),
    ("HK", &["hong kong", "гонконг"]),
    ("TW", &["taiwan", "тайвань"]),
    ("TH", &["thailand", "таиланд"]),
    ("VN", &["vietnam", "вьетнам"]),
    ("MY", &["malaysia", "малайзия"]),
    ("ID", &["indonesia", "индонезия"]),
    ("PH", &["philippines", "филиппины"]),
    ("AU", &["australia", "австралия"]),
    ("NZ", &["new zealand", "новая зеландия"]),
    ("CA", &["canada", "канада"]),
    ("MX", &["mexico", "мексика"]),
    ("BR", &["brazil", "бразилия"]),
    ("AR", &["argentina", "аргентина"]),
    ("CL", &["chile", "чили"]),
    ("CO", &["colombia", "колумбия"]),
    ("ZA", &["south africa", "юар", "южная африка"]),
    ("EG", &["egypt", "египет"]),
    ("IR", &["iran", "иран"]),
    ("IQ", &["iraq", "ирак"]),
    ("PK", &["pakistan", "пакистан"]),
    ("BD", &["bangladesh", "бангладеш"]),
    ("NG", &["nigeria", "нигерия"]),
    ("KE", &["kenya", "кения"]),
    ("MA", &["morocco", "марокко"]),
    ("DZ", &["algeria", "алжир"]),
    ("TN", &["tunisia", "тунис"]),
];

/// Full ISO 3166-1 alpha-2 code list (used to validate standalone code tokens).
const ISO_CODES: &str = "AD AE AF AG AI AL AM AO AQ AR AS AT AU AW AX AZ BA BB BD BE BF BG BH BI BJ BL BM BN BO BQ BR BS BT BV BW BY BZ CA CC CD CF CG CH CI CK CL CM CN CO CR CU CV CW CX CY CZ DE DJ DK DM DO DZ EC EE EG EH ER ES ET FI FJ FK FM FO FR GA GB GD GE GF GG GH GI GL GM GN GP GQ GR GS GT GU GW GY HK HM HN HR HT HU ID IE IL IM IN IO IQ IR IS IT JE JM JO JP KE KG KH KI KM KN KP KR KW KY KZ LA LB LC LI LK LR LS LT LU LV LY MA MC MD ME MF MG MH MK ML MM MN MO MP MQ MR MS MT MU MV MW MX MY MZ NA NC NE NF NG NI NL NO NP NR NU NZ OM PA PE PF PG PH PK PL PM PN PR PS PT PW PY QA RE RO RS RU RW SA SB SC SD SE SG SH SI SJ SK SL SM SN SO SR SS ST SV SX SY SZ TC TD TF TG TH TJ TK TL TM TN TO TR TT TV TW TZ UA UG UM US UY UZ VA VC VE VG VI VN VU WF WS YE YT ZA ZM ZW";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_emoji_roundtrip() {
        assert_eq!(flag_emoji("de"), "🇩🇪");
        assert_eq!(flag_emoji("DE"), "🇩🇪");
        assert_eq!(flag_emoji("gb"), "🇬🇧");
        assert_eq!(flag_emoji(""), "");
        assert_eq!(flag_emoji("USA"), "");
        assert_eq!(flag_emoji("d1"), "");
    }

    #[test]
    fn regional_indicator_first() {
        assert_eq!(detect_country("🇩🇪 Germany 1"), Some("DE".into()));
        assert_eq!(detect_country("🇬🇧 London"), Some("GB".into()));
        assert_eq!(detect_country("🇳🇱 Amsterdam"), Some("NL".into()));
        assert_eq!(detect_country("🇯🇵 Tokyo"), Some("JP".into()));
    }

    #[test]
    fn russian_and_english_names() {
        assert_eq!(detect_country("Германия #1"), Some("DE".into()));
        assert_eq!(detect_country("Germany"), Some("DE".into()));
        assert_eq!(detect_country("Нидерланды 2"), Some("NL".into()));
        assert_eq!(detect_country("Netherlands"), Some("NL".into()));
        assert_eq!(detect_country("Россия Москва"), Some("RU".into()));
        assert_eq!(detect_country("United States West"), Some("US".into()));
        assert_eq!(detect_country("Финляндия"), Some("FI".into()));
    }

    #[test]
    fn iso_code_tokens() {
        assert_eq!(detect_country("DE-1"), Some("DE".into()));
        assert_eq!(detect_country("[NL]"), Some("NL".into()));
        assert_eq!(detect_country("NL 2"), Some("NL".into()));
        assert_eq!(detect_country("node FR"), Some("FR".into()));
    }

    #[test]
    fn ignores_non_codes_and_unknown() {
        assert_eq!(detect_country("VPN"), None);
        assert_eq!(detect_country("XX"), None);
        assert_eq!(detect_country("fast node"), None);
        assert_eq!(detect_country(""), None);
        assert_eq!(detect_country("Node 12"), None);
    }

    #[test]
    fn case_insensitive_names() {
        assert_eq!(detect_country("germany"), Some("DE".into()));
        assert_eq!(detect_country("ГЕРМАНИЯ"), Some("DE".into()));
    }
}

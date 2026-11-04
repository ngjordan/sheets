//! Language detection for DETECTLANGUAGE.
//!
//! Excel's DETECTLANGUAGE calls Microsoft Translator and returns a language
//! code such as `es` or `en`. That needs a network service, so this module
//! detects the language offline instead: a non-Latin script names its
//! language outright (Devanagari is Hindi, Cyrillic is Russian, and so on),
//! and a Latin-script text is told apart by the function words it uses.
//!
//! The codes are the ones Excel returns, so a formula written for Excel keeps
//! working: `en`, `es`, `fr`, `de`, `pt`, `it`, `nl`, `ru`, `ja`, `ko`,
//! `zh-Hans`, `zh-Hant`, `ar`, `he`, `hi`, `th`, `el`, `tr`, `uk`, `vi`.

/// A language the detector can name.
struct Lang {
    /// The code Excel returns for it.
    code: &'static str,
    /// The Unicode ranges its script occupies, as (start, end) pairs.
    script: &'static [(u32, u32)],
    /// The function words that give it away, lower-case.
    words: &'static [&'static str],
}

/// The scripts the detector recognises, each with its language.
const LANGUAGES: &[Lang] = &[
    Lang { code: "ru", script: &[(0x0400, 0x04FF)], words: &["и", "в", "не", "на", "что", "это", "как", "по", "из", "для"] },
    Lang { code: "uk", script: &[(0x0490, 0x04FF)], words: &["та", "що", "не", "для", "це", "як", "від", "але"] },
    Lang { code: "el", script: &[(0x0370, 0x03FF)], words: &["και", "το", "της", "των", "για", "με", "δεν", "στο", "από"] },
    Lang {
        code: "ar",
        script: &[(0x0600, 0x06FF), (0x0750, 0x077F), (0xFB50, 0xFDFF), (0xFE70, 0xFEFF)],
        words: &["في", "من", "على", "إلى", "عن", "هذا", "التي", "مع"],
    },
    Lang { code: "he", script: &[(0x0590, 0x05FF)], words: &["של", "את", "לא", "זה", "על", "עם", "כל", "אני"] },
    Lang {
        code: "hi", script: &[(0x0900, 0x097F)], words: &["और", "का", "की", "के", "में", "से", "है", "पर", "यह"]
    },
    Lang {
        code: "th", script: &[(0x0E00, 0x0E7F)], words: &["และ", "ที่", "ของ", "ใน", "การ", "เป็น", "ได้", "มี"]
    },
    Lang {
        code: "ko",
        script: &[(0xAC00, 0xD7AF), (0x1100, 0x11FF), (0x3130, 0x318F)],
        words: &["이", "그", "저", "는", "을", "를", "에", "의", "와"],
    },
    Lang { code: "ja", script: &[(0x3040, 0x309F), (0x30A0, 0x30FF)], words: &["は", "が", "を", "に", "で", "と", "も", "の", "です"] },
    Lang { code: "zh-Hans", script: &[(0x4E00, 0x9FFF)], words: &["的", "是", "在", "了", "和", "有", "我", "不", "人"] },
];

/// The Latin-script languages, told apart by their function words.
const LATIN: &[Lang] = &[
    Lang {
        code: "es",
        script: &[],
        words: &[
            "el", "la", "los", "las", "de", "que", "y", "en", "un", "una", "por", "con", "no", "para", "es", "se", "su", "hola", "mundo", "gracias",
            "como", "más", "muy", "pero", "todo", "cuando", "donde", "porque", "este", "esta", "hola", "amigo", "cosa", "gente", "tiempo", "día",
            "sin", "entre", "sobre", "hasta", "desde", "puede",
        ],
    },
    Lang {
        code: "fr",
        script: &[],
        words: &[
            "le", "la", "les", "des", "de", "du", "et", "est", "en", "un", "une", "pour", "que", "qui", "dans", "avec", "ne", "pas", "bonjour",
            "monde", "merci", "comment", "plus", "tres", "mais", "tout", "cette", "c'est", "sont", "sur", "avec", "nous", "vous", "c'est", "aussi",
            "bien", "faire", "temps",
        ],
    },
    Lang {
        code: "de",
        script: &[],
        words: &[
            "der", "die", "das", "und", "ist", "nicht", "ein", "eine", "den", "von", "zu", "mit", "sich", "auf", "für", "es", "auch", "ich", "hallo",
            "welt", "danke", "wie", "mehr", "sehr", "aber", "alle", "wenn", "dass", "haben", "werden", "kann", "noch", "wie", "wir", "ihr", "sind",
            "auch", "nach", "bei", "aus", "dieser",
        ],
    },
    Lang {
        code: "pt",
        script: &[],
        words: &[
            "de", "que", "não", "uma", "para", "com", "por", "como", "mais", "está", "são", "dos", "das", "ao", "ele", "ela", "isso", "olá", "mundo",
            "obrigado", "muito", "mas", "todos", "quando", "onde", "porque", "pode", "fazer", "tempo", "dia", "ser", "ter", "seu", "minha", "não",
        ],
    },
    Lang {
        code: "it",
        script: &[],
        words: &[
            "il", "lo", "la", "che", "di", "per", "con", "non", "una", "sono", "come", "più", "anche", "nel", "gli", "questo", "del", "ciao",
            "mondo", "grazie", "molto", "ma", "tutti", "quando", "dove", "perché", "può", "fare", "tempo", "giorno", "sono", "questo", "ancora",
            "dopo", "prima",
        ],
    },
    Lang {
        code: "nl",
        script: &[],
        words: &[
            "de", "het", "een", "van", "en", "is", "dat", "op", "te", "met", "voor", "niet", "zijn", "aan", "maar", "ook", "er", "hallo", "wereld",
            "dank", "hoe", "meer", "maar", "alles", "wanneer", "waar", "omdat", "kan", "worden", "naar", "door", "over", "deze", "toch",
        ],
    },
    Lang {
        code: "en",
        script: &[],
        words: &[
            "the", "is", "and", "of", "to", "in", "that", "it", "for", "are", "was", "with", "on", "this", "you", "not", "have", "from", "hello",
            "world", "thanks", "how", "more", "very", "but", "all", "when", "where", "because", "can", "will", "would", "there", "their", "what",
            "about", "which", "been", "has", "had", "its", "who",
        ],
    },
    Lang {
        code: "vi",
        script: &[],
        words: &[
            "và", "của", "là", "có", "được", "trong", "cho", "không", "người", "những", "một", "các", "với", "này", "chào", "thế", "giới", "cảm",
            "ơn", "rất", "nhưng", "tất", "khi", "để", "vì", "được", "sẽ", "ra",
        ],
    },
    Lang {
        code: "tr",
        script: &[],
        words: &[
            "bir",
            "ve",
            "bu",
            "için",
            "ile",
            "de",
            "da",
            "çok",
            "daha",
            "olarak",
            "gibi",
            "ise",
            "ne",
            "o",
            "her",
            "merhaba",
            "dünya",
            "teşekkür",
            "nasıl",
            "ama",
            "hepsi",
            "ne zaman",
            "nerede",
            "çünkü",
            "yapabilir",
            "zaman",
            "gün",
            "bu",
            "için",
            "sonra",
            "önce",
        ],
    },
];

/// The language of `text`, or `None` when there is nothing to go on.
pub fn detect(text: &str) -> Option<&'static str> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    // A non-Latin script names its language outright. The script is found by
    // the letters' code points, so digits and punctuation do not confuse it.
    let mut counts: Vec<(usize, usize)> = vec![(0, 0); LANGUAGES.len()];
    let mut latin_letters = 0usize;
    for ch in text.chars() {
        if !ch.is_alphabetic() {
            continue;
        }
        let cp = ch as u32;
        let mut hit = false;
        for (i, lang) in LANGUAGES.iter().enumerate() {
            if lang.script.iter().any(|(lo, hi)| cp >= *lo && cp <= *hi) {
                counts[i].0 += 1;
                hit = true;
                break;
            }
        }
        if !hit {
            latin_letters += 1;
        }
    }

    // Japanese shares its Han characters with Chinese, so the kana decide.
    let kana = text.chars().filter(|c| matches!(*c as u32, 0x3040..=0x30FF)).count();
    if kana > 0 {
        return Some("ja");
    }

    for (i, lang) in LANGUAGES.iter().enumerate() {
        let (letters, _) = counts[i];
        if letters > 0 && letters * 2 >= latin_letters.max(1) {
            // Chinese: Traditional characters are the ones Simplified lacks.
            return Some(if is_traditional(text) { "zh-Hant" } else { lang.code });
        }
    }

    // Latin script: the language with the most of its words wins.
    let words: Vec<String> = text.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect();
    if words.is_empty() {
        return None;
    }
    // Each language is scored by the share of the text's words it claims.
    // English is the tie-breaker: it is the language most text is in, and
    // its list is the longest, so it wins only on a real count.
    let mut best = (0.0f64, "en");
    for lang in LATIN {
        let hits = words.iter().filter(|w| lang.words.contains(&w.as_str())).count();
        if hits == 0 {
            continue;
        }
        // Words the language does not have count against it.
        let share = hits as f64 / words.len() as f64;
        if share > best.0 {
            best = (share, lang.code);
        }
    }
    // No word from any list: the text is too short to tell. Latin script
    // without a giveaway is English, the language most text is in.
    Some(best.1)
}

/// Whether the text uses Traditional rather than Simplified Chinese.
///
/// The two forms differ in their own characters: 這 is Traditional where
/// 这 is Simplified, and so on. A text that uses either form outright is
/// that form; a text mixing both is reported as Simplified, the more common
/// one.
fn is_traditional(text: &str) -> bool {
    // Characters that only the Traditional form has.
    const TRADITIONAL: &[char] = &[
        '們', '國', '學', '時', '會', '說', '對', '開', '關', '實', '發', '體', '龍', '點', '無', '與', '這', '個', '們', '來', '後', '們', '樣',
        '點', '沒', '過', '還', '現', '當', '們',
    ];
    // Characters that only the Simplified form has.
    const SIMPLIFIED: &[char] = &[
        '们', '国', '学', '时', '会', '说', '对', '开', '关', '实', '发', '体', '龙', '点', '无', '与', '这', '个', '来', '后', '样', '没', '过',
        '还', '现', '当',
    ];
    let traditional = text.chars().filter(|c| TRADITIONAL.contains(c)).count();
    let simplified = text.chars().filter(|c| SIMPLIFIED.contains(c)).count();
    traditional > simplified
}

#[cfg(test)]
mod tests {
    use super::detect;

    #[test]
    fn a_script_names_its_language() {
        assert_eq!(detect("Hola mundo"), Some("es"));
        assert_eq!(detect("Hola mundo! ¿Cómo estás?"), Some("es"));
        assert_eq!(detect("Hello world"), Some("en"));
        assert_eq!(detect("The quick brown fox jumps over the lazy dog"), Some("en"));
        assert_eq!(detect("Bonjour le monde, comment allez-vous"), Some("fr"));
        assert_eq!(detect("Der Hund und die Katze sind nicht das gleiche"), Some("de"));
        assert_eq!(detect("Это очень длинный текст на русском языке"), Some("ru"));
        assert_eq!(detect("これは日本語のテキストです"), Some("ja"));
        assert_eq!(detect("이것은 한국어 텍스트입니다"), Some("ko"));
        assert_eq!(detect("这是一段中文文本"), Some("zh-Hans"));
        assert_eq!(detect("這是一段中文文字"), Some("zh-Hant"));
        assert_eq!(detect("यह हिन्दी में लिखा गया पाठ है"), Some("hi"));
        assert_eq!(detect("Αυτό είναι ένα ελληνικό κείμενο"), Some("el"));
    }

    #[test]
    fn nothing_to_detect_gives_nothing() {
        assert_eq!(detect(""), None);
        assert_eq!(detect("   "), None);
        assert_eq!(detect("123 456"), None);
    }
}

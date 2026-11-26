use std::collections::{BTreeMap, HashMap};

const EN_JSON: &str = include_str!("../../../locales/en.json");
const TR_JSON: &str = include_str!("../../../locales/tr.json");

#[derive(Debug, Clone)]
pub struct LanguageInfo {
    pub code: String,
    pub display_name: String,
}

/// Lightweight JSON-backed localization engine with fallback to English and key name.
#[derive(Debug, Clone)]
pub struct I18n {
    active_lang: String,
    catalogs: BTreeMap<String, HashMap<String, String>>,
}

impl Default for I18n {
    fn default() -> Self {
        Self::new("en")
    }
}

impl I18n {
    pub fn new(initial_lang: &str) -> Self {
        let mut catalogs = BTreeMap::new();
        if let Ok(en_map) = serde_json::from_str::<HashMap<String, String>>(EN_JSON) {
            catalogs.insert("en".to_string(), en_map);
        }
        if let Ok(tr_map) = serde_json::from_str::<HashMap<String, String>>(TR_JSON) {
            catalogs.insert("tr".to_string(), tr_map);
        }

        let mut instance = Self {
            active_lang: "en".to_string(),
            catalogs,
        };
        instance.set_language(initial_lang);
        instance
    }

    pub fn active_language(&self) -> &str {
        &self.active_lang
    }

    pub fn set_language(&mut self, code: &str) {
        let normalized = code.trim().to_lowercase();
        if self.catalogs.contains_key(&normalized) {
            self.active_lang = normalized;
        } else {
            self.active_lang = "en".to_string();
        }
    }

    pub fn available_languages(&self) -> Vec<LanguageInfo> {
        self.catalogs
            .keys()
            .map(|code| LanguageInfo {
                code: code.clone(),
                display_name: match code.as_str() {
                    "en" => "English".to_string(),
                    "tr" => "Türkçe".to_string(),
                    other => other.to_uppercase(),
                },
            })
            .collect()
    }

    pub fn t(&self, key: &str) -> String {
        if let Some(active) = self.catalogs.get(&self.active_lang) {
            if let Some(val) = active.get(key) {
                return val.clone();
            }
        }
        if let Some(en) = self.catalogs.get("en") {
            if let Some(val) = en.get(key) {
                return val.clone();
            }
        }
        key.to_string()
    }

    pub fn t_fmt(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let mut out = self.t(key);
        for (name, value) in vars {
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn en_and_tr_locales_have_exact_key_parity_and_placeholders() {
        let en: HashMap<String, String> = serde_json::from_str(EN_JSON).expect("valid en.json");
        let tr: HashMap<String, String> = serde_json::from_str(TR_JSON).expect("valid tr.json");

        let en_keys: BTreeSet<&String> = en.keys().collect();
        let tr_keys: BTreeSet<&String> = tr.keys().collect();

        let missing_in_tr: Vec<_> = en_keys.difference(&tr_keys).collect();
        let extra_in_tr: Vec<_> = tr_keys.difference(&en_keys).collect();

        assert!(
            missing_in_tr.is_empty(),
            "Missing keys in tr.json: {missing_in_tr:?}"
        );
        assert!(
            extra_in_tr.is_empty(),
            "Extra keys in tr.json: {extra_in_tr:?}"
        );

        let extract_placeholders = |s: &str| -> BTreeSet<String> {
            let mut out = BTreeSet::new();
            let mut rest = s;
            while let Some(start) = rest.find('{') {
                let after = &rest[start + 1..];
                if let Some(end) = after.find('}') {
                    out.insert(after[..end].to_string());
                    rest = &after[end + 1..];
                } else {
                    break;
                }
            }
            out
        };

        for (key, en_val) in &en {
            let tr_val = &tr[key];
            assert_eq!(
                extract_placeholders(en_val),
                extract_placeholders(tr_val),
                "Placeholder mismatch for key `{key}`: en={en_val:?}, tr={tr_val:?}"
            );
        }
    }

    #[test]
    fn translation_and_fallback_work() {
        let mut i18n = I18n::new("tr");
        assert_eq!(i18n.t("app.title"), "Omvix Hub");
        assert_eq!(i18n.t("nav.battery"), "Batarya Bakımı");
        assert_eq!(
            i18n.t_fmt("fan.rpm", &[("rpm", "2400")]),
            "2400 RPM".to_string()
        );
        assert_eq!(i18n.t("non.existent.key"), "non.existent.key");

        i18n.set_language("fr");
        assert_eq!(i18n.active_language(), "en");
    }
}

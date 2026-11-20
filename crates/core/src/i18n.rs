use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

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

    /// Loads additional `<lang>.json` translation packs from an external directory.
    pub fn load_extra_locales(&mut self, dir: &Path) -> usize {
        let Ok(entries) = fs::read_dir(dir) else {
            return 0;
        };
        let mut loaded = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let code = stem.to_lowercase();
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&raw) {
                    self.catalogs.insert(code, map);
                    loaded += 1;
                }
            }
        }
        loaded
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
    fn en_and_tr_locales_have_exact_key_parity() {
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

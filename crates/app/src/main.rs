use omvix_core::{init_logging, AppConfig, I18n};

fn main() {
    let config_path = AppConfig::default_config_path();
    let config = AppConfig::load_from_path(&config_path).unwrap_or_default();
    let _log = init_logging(&config.logging, false);
    let i18n = I18n::new(&config.general.language);
    tracing::info!(lang = %i18n.active_language(), "Omvix Hub scaffold initialized");
    println!("{} - {}", i18n.t("app.title"), i18n.t("app.subtitle"));
}

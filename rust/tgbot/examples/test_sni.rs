use tgbot::logic::config::RealityProto;
use tgbot::logic::geoip::GeoIPService;
use tgbot::logic::sni_selector::SNISelector;

#[tokio::main]
async fn main() {
    // Initialize logger
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    log::info!("Starting SNI Selector Test");

    // 1. Test GeoIP
    log::info!("Testing GeoIP Service...");
    let geoip = GeoIPService::new();
    let country = geoip.get_country_code().await;
    log::info!("Detected Country Code: {}", country);

    // 2. Test SNI Selector
    log::info!("Testing SNI Selector for {}", country);
    let mut selector = SNISelector::get_for_country(&country, RealityProto::Vision);

    log::info!("Generating 5 SNIs for {}:", country);
    for i in 0..5 {
        let sni = selector.next();
        log::info!("  [{}] {}", i + 1, sni);
    }

    // 3. Test US (Manual file with ports and commas)
    log::info!("Testing US SNI (should be cleaned):");
    let mut us_selector = SNISelector::get_for_country("US", RealityProto::XHTTP);
    for i in 0..5 {
        let sni = us_selector.next();
        log::info!("  [US-{}] '{}'", i + 1, sni);
    }

    // 4. Test Fallback (Unknown country)
    log::info!("Testing SNI Selector for UNKNOWN country");
    let mut fallback_selector = SNISelector::get_for_country("UNKNOWN", RealityProto::Vision);
    let fallback_sni = fallback_selector.next();
    log::info!("  Fallback SNI: {}", fallback_sni);
}

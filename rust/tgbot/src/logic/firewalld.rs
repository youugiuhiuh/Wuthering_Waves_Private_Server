use anyhow::Result;
use std::collections::HashSet;
use zbus::proxy;

// Runtime - Main Interface
#[proxy(
    interface = "org.fedoraproject.FirewallD1",
    default_service = "org.fedoraproject.FirewallD1",
    default_path = "/org/fedoraproject/FirewallD1"
)]
trait FirewallD1 {
    #[zbus(name = "reload")]
    fn reload(&self) -> zbus::Result<()>;
    #[zbus(name = "getDefaultZone")]
    fn get_default_zone(&self) -> zbus::Result<String>;

    // config() method returns the object path to the config interface
    #[zbus(property)]
    fn config(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

// Runtime - Zone Interface
#[proxy(
    interface = "org.fedoraproject.FirewallD1.zone",
    default_service = "org.fedoraproject.FirewallD1",
    default_path = "/org/fedoraproject/FirewallD1"
)]
trait FirewallD1Zone {
    #[zbus(name = "addPort")]
    fn add_port(&self, zone: &str, port: &str, protocol: &str, timeout: i32) -> zbus::Result<()>;
    #[zbus(name = "removePort")]
    fn remove_port(&self, zone: &str, port: &str, protocol: &str) -> zbus::Result<()>;
    #[zbus(name = "queryPort")]
    fn query_port(&self, zone: &str, port: &str, protocol: &str) -> zbus::Result<bool>;
    #[zbus(name = "setTarget")]
    fn set_target(&self, zone: &str, target: &str) -> zbus::Result<()>;
}

// Config (Permanent) - Main Interface
// The object path is usually /org/fedoraproject/FirewallD1/config, but we get it dynamically
#[proxy(
    interface = "org.fedoraproject.FirewallD1.config",
    default_service = "org.fedoraproject.FirewallD1"
)]
trait FirewallD1Config {
    #[zbus(name = "getZoneByName")]
    fn get_zone_by_name(&self, name: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

// Config (Permanent) - Zone Interface
// The object path is returned by getZoneByName
#[proxy(
    interface = "org.fedoraproject.FirewallD1.config.zone",
    default_service = "org.fedoraproject.FirewallD1"
)]
trait FirewallD1ConfigZone {
    #[zbus(name = "addPort")]
    fn add_port(&self, port: &str, protocol: &str) -> zbus::Result<()>;
    #[zbus(name = "removePort")]
    fn remove_port(&self, port: &str, protocol: &str) -> zbus::Result<()>;
    #[zbus(name = "queryPort")]
    fn query_port(&self, port: &str, protocol: &str) -> zbus::Result<bool>;
    #[zbus(name = "setTarget")]
    fn set_target(&self, target: &str) -> zbus::Result<()>;
}

pub struct FirewalldClient;

impl FirewalldClient {
    pub async fn add_port(port: u16, protocol: &str) -> Result<()> {
        let connection = zbus::Connection::system().await?;

        // Proxies
        let proxy = FirewallD1Proxy::new(&connection).await?;
        let zone_proxy = FirewallD1ZoneProxy::new(&connection).await?;

        let zone = proxy.get_default_zone().await?;
        let port_str = port.to_string();

        // 1. Runtime: Add port immediately (Safe to fail)
        if !zone_proxy
            .query_port(&zone, &port_str, protocol)
            .await
            .unwrap_or(false)
        {
            let _ = zone_proxy.add_port(&zone, &port_str, protocol, 0).await;
        }

        // 2. Permanent: Add port to config
        // Get config object path (With fallback if Property 'Config' doesn't exist)
        let config_path = match proxy.config().await {
            Ok(path) => path,
            Err(_) => {
                zbus::zvariant::OwnedObjectPath::try_from("/org/fedoraproject/FirewallD1/config")
                    .unwrap()
            }
        };
        let config_proxy = FirewallD1ConfigProxy::builder(&connection)
            .path(config_path)?
            .build()
            .await?;

        // Get permanent zone object path
        if let Ok(zone_path) = config_proxy.get_zone_by_name(&zone).await {
            let config_zone_proxy = FirewallD1ConfigZoneProxy::builder(&connection)
                .path(zone_path)?
                .build()
                .await?;

            // Check if port exists in permanent config
            if !config_zone_proxy
                .query_port(&port_str, protocol)
                .await
                .unwrap_or(false)
            {
                config_zone_proxy.add_port(&port_str, protocol).await?;
            }
        }

        Ok(())
    }

    pub async fn harden_with_ports(ports: HashSet<u16>) -> Result<()> {
        let connection = zbus::Connection::system().await?;
        let proxy = FirewallD1Proxy::new(&connection).await?;

        let zone = proxy.get_default_zone().await?;

        // Get config object path (Permanent) (With fallback if Property 'Config' doesn't exist)
        let config_path = match proxy.config().await {
            Ok(path) => path,
            Err(_) => {
                zbus::zvariant::OwnedObjectPath::try_from("/org/fedoraproject/FirewallD1/config")
                    .unwrap()
            }
        };
        let config_proxy = FirewallD1ConfigProxy::builder(&connection)
            .path(config_path)?
            .build()
            .await?;

        // Get zone object path (Permanent)
        let zone_path = config_proxy.get_zone_by_name(&zone).await?;
        let config_zone_proxy = FirewallD1ConfigZoneProxy::builder(&connection)
            .path(zone_path)?
            .build()
            .await?;

        // 1. Set default zone target to DROP to harden (Permanent)
        config_zone_proxy.set_target("DROP").await?;

        // 2. Add all scanned ports (Permanent)
        for port in ports {
            let port_str = port.to_string();
            for proto in &["tcp", "udp"] {
                if !config_zone_proxy
                    .query_port(&port_str, proto)
                    .await
                    .unwrap_or(false)
                {
                    config_zone_proxy.add_port(&port_str, proto).await?;
                }
            }
        }

        // 3. Reload to apply permanent changes to runtime
        proxy.reload().await?;

        Ok(())
    }
}

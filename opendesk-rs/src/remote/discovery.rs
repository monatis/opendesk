//! LAN discovery via mDNS / Zeroconf / Bonjour.
//!
//! Service type: `_opendesk._tcp.local.`

use anyhow::{Result, anyhow};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::time::Duration;

use crate::protocol::storage::fingerprint;

pub const SERVICE_TYPE: &str = "_opendesk._tcp.local.";

#[derive(Clone, Debug)]
pub struct DiscoveredPeer {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub public_key: Vec<u8>,
    pub fingerprint: String,
    pub description: String,
}

impl DiscoveredPeer {
    pub fn url(&self) -> String {
        format!("ws://{}:{}", self.host, self.port)
    }
}

pub struct Advertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Advertisement {
    pub fn unregister(self) {
        let _ = self.daemon.unregister(&self.fullname);
    }
}

/// Register this opendesk service on the LAN via mDNS.
pub fn advertise(
    name: &str,
    port: u16,
    public_key: &[u8],
    description: &str,
) -> Result<Advertisement> {
    let daemon = ServiceDaemon::new().map_err(|e| anyhow!("Failed to start mDNS daemon: {e}"))?;

    let fp = fingerprint(public_key);
    let mut properties = HashMap::new();
    properties.insert("v".to_string(), "1".to_string());
    properties.insert("pk".to_string(), data_encoding::HEXLOWER.encode(public_key));
    properties.insert("fp".to_string(), fp);
    if !description.is_empty() {
        let desc = if description.len() > 120 {
            &description[..120]
        } else {
            description
        };
        properties.insert("desc".to_string(), desc.to_string());
    }

    let host_name = format!("{}.local.", name);
    let service_info = ServiceInfo::new(SERVICE_TYPE, name, &host_name, "", port, properties)
        .map_err(|e| anyhow!("Failed to create ServiceInfo: {e}"))?;

    let fullname = service_info.get_fullname().to_string();
    daemon
        .register(service_info)
        .map_err(|e| anyhow!("Failed to register mDNS service: {e}"))?;

    Ok(Advertisement { daemon, fullname })
}

/// Browse the LAN for opendesk peers and return what's found.
pub async fn discover(timeout: Duration) -> Result<Vec<DiscoveredPeer>> {
    let daemon = ServiceDaemon::new().map_err(|e| anyhow!("Failed to start mDNS daemon: {e}"))?;
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|e| anyhow!("Failed to browse mDNS: {e}"))?;

    let deadline = tokio::time::Instant::now() + timeout;
    let mut peers: HashMap<String, DiscoveredPeer> = HashMap::new();

    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline - now;

        match tokio::time::timeout(
            remaining,
            tokio::task::spawn_blocking({
                let receiver = receiver.clone();
                move || receiver.recv_timeout(Duration::from_millis(200))
            }),
        )
        .await
        {
            Ok(Ok(Ok(ServiceEvent::ServiceResolved(info)))) => {
                let name = info
                    .get_fullname()
                    .trim_end_matches(&format!(".{SERVICE_TYPE}"))
                    .to_string();

                let port = info.get_port();
                let addrs = info.get_addresses();
                let host = addrs
                    .iter()
                    .find(|a| a.is_ipv4())
                    .map(|a| a.to_string())
                    .or_else(|| addrs.iter().next().map(|a| a.to_string()))
                    .unwrap_or_else(|| "127.0.0.1".to_string());

                let props = info.get_properties();
                let pk_hex = props.get_property_val_str("pk").unwrap_or_default();
                let pk_bytes = data_encoding::HEXLOWER
                    .decode(pk_hex.as_bytes())
                    .unwrap_or_default();

                let fp = props
                    .get_property_val_str("fp")
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        if !pk_bytes.is_empty() {
                            fingerprint(&pk_bytes)
                        } else {
                            String::new()
                        }
                    });

                let desc = props
                    .get_property_val_str("desc")
                    .unwrap_or_default()
                    .to_string();

                if !pk_bytes.is_empty() {
                    let key = format!("{}:{}", host, port);
                    peers.insert(
                        key,
                        DiscoveredPeer {
                            name,
                            host,
                            port,
                            public_key: pk_bytes,
                            fingerprint: fp,
                            description: desc,
                        },
                    );
                }
            }
            _ => {
                // Timeout or receive error, continue until deadline
            }
        }
    }

    let _ = daemon.stop_browse(SERVICE_TYPE);
    let mut list: Vec<DiscoveredPeer> = peers.into_values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
}

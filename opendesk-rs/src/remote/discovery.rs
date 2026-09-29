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

impl Drop for Advertisement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
    }
}

impl Advertisement {
    pub fn unregister(self) {
        drop(self);
    }
}

fn score_ip_str(addr_str: &str) -> u32 {
    let Ok(addr) = addr_str.parse::<std::net::IpAddr>() else {
        return 0;
    };
    match addr {
        std::net::IpAddr::V4(v4) => {
            if v4.is_loopback() {
                1
            } else if v4.is_link_local() {
                2
            } else if v4.octets()[0] == 172 && (16..=31).contains(&v4.octets()[1]) {
                // Hyper-V / WSL / Docker virtual interface
                8
            } else if v4.is_private() {
                // Real LAN IPv4 (192.168.x.x, 10.x.x.x)
                10
            } else {
                // Public or other IPv4
                9
            }
        }
        std::net::IpAddr::V6(v6) => {
            if v6.is_loopback() {
                0
            } else if (v6.segments()[0] & 0xffc0) == 0xfe80 {
                // Link-local IPv6
                3
            } else {
                // Global IPv6
                4
            }
        }
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

    let clean_host: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_lowercase();
    let host_label = if clean_host.is_empty() {
        "opendesk"
    } else {
        &clean_host
    };
    let host_name = format!("{}.local.", host_label);

    let service_info = ServiceInfo::new(SERVICE_TYPE, name, &host_name, "", port, properties)
        .map_err(|e| anyhow!("Failed to create ServiceInfo: {e}"))?
        .enable_addr_auto();

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
            Ok(Ok(Ok(event))) => {
                tracing::debug!("mDNS event: {:?}", event);
                if let ServiceEvent::ServiceResolved(info) = event {
                    let fullname = info.get_fullname();
                    let name = fullname
                        .strip_suffix(&format!(".{SERVICE_TYPE}"))
                        .or_else(|| fullname.strip_suffix(SERVICE_TYPE))
                        .unwrap_or(fullname)
                        .to_string();

                    let port = info.get_port();
                    let addrs = info.get_addresses();
                    if addrs.is_empty() {
                        continue;
                    }

                    let best_ip = addrs.iter().max_by_key(|a| score_ip_str(&a.to_string()));

                    let host = best_ip
                        .map(|a| a.to_string())
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
                        let key = if !fp.is_empty() {
                            fp.clone()
                        } else if !pk_hex.is_empty() {
                            pk_hex.to_string()
                        } else {
                            name.clone()
                        };

                        if let Some(existing) = peers.get_mut(&key) {
                            let existing_score = score_ip_str(&existing.host);
                            let new_score = score_ip_str(&host);
                            if new_score > existing_score {
                                existing.host = host;
                                existing.port = port;
                            }
                        } else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mdns_advertise_and_discover() {
        let dummy_pk = vec![42u8; 32];
        let adv =
            advertise("test-peer", 8424, &dummy_pk, "test description").expect("advertise failed");
        let peers = discover(Duration::from_secs(3))
            .await
            .expect("discover failed");
        println!("Discovered peers: {:?}", peers);
        adv.unregister();
        assert_eq!(peers.len(), 1, "Expected exactly 1 peer discovered");
        assert_eq!(peers[0].name, "test-peer");
        assert_ne!(
            peers[0].host, "127.0.0.1",
            "Should prefer LAN IP over loopback"
        );
    }
}

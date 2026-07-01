// Optional mDNS / DNS-SD discovery, alongside the UDP-broadcast path. Best-effort: if mDNS is
// unavailable (no multicast, firewall, no daemon) it simply contributes nothing and the UDP
// path still works. The host advertises `_nutler._tcp.local.` with its name + user count in TXT
// records; clients browse for it. (TXT records are cached, so the mDNS user count is a snapshot,
// unlike the UDP responder which answers each probe with a live count.)

use crate::sockets::ServerInfo;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const SERVICE_TYPE: &str = "_nutler._tcp.local.";

/// Register this host as an mDNS service (best-effort). Keep the returned daemon alive while
/// hosting (its background thread advertises the service); call `.shutdown()` on teardown.
pub fn register(name: &str, tcp_port: u16, user_count: usize) -> Option<ServiceDaemon> {
    let mdns = ServiceDaemon::new().ok()?;
    let mut props = HashMap::new();
    props.insert("name".to_string(), name.to_string());
    props.insert("users".to_string(), user_count.to_string());
    let instance = if name.is_empty() { "nutler-host" } else { name };
    let info = ServiceInfo::new(SERVICE_TYPE, instance, "nutler.local.", "", tcp_port, props)
        .ok()?
        .enable_addr_auto();
    mdns.register(info).ok()?;
    tracing::info!("📡 mDNS service registered ({})", SERVICE_TYPE);
    Some(mdns)
}

/// Browse for Nutler hosts via mDNS for `window`. Returns [] if mDNS is unavailable.
pub async fn browse(window: Duration) -> Vec<ServerInfo> {
    tokio::task::spawn_blocking(move || {
        let mut found: Vec<ServerInfo> = Vec::new();
        let mdns = match ServiceDaemon::new() {
            Ok(m) => m,
            Err(_) => return found,
        };
        let recv = match mdns.browse(SERVICE_TYPE) {
            Ok(r) => r,
            Err(_) => return found,
        };
        let deadline = Instant::now() + window;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match recv.recv_timeout(remaining) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    if let Some(addr) = info.get_addresses().iter().next() {
                        found.push(ServerInfo {
                            address: addr.to_string(),
                            port: info.get_port(),
                            name: info
                                .get_property_val_str("name")
                                .unwrap_or("Nutler host")
                                .to_string(),
                            user_count: info
                                .get_property_val_str("users")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0),
                        });
                    }
                }
                Ok(_) => {}      // other lifecycle events — ignore
                Err(_) => break, // timeout → window closed
            }
        }
        let _ = mdns.shutdown();
        found
    })
    .await
    .unwrap_or_default()
}

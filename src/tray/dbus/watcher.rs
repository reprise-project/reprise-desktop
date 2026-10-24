use super::{Wake, lock};
use std::sync::{Arc, Mutex};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;

/// `service` as RegisterStatusNotifierItem passes it: an object path on the caller's bus name, a
/// bus name at the well-known path, or a bus name followed by the path.
pub(super) fn resolve_registration(
    service: &str,
    sender: Option<&str>,
) -> Option<(String, String)> {
    if service.starts_with('/') {
        Some((sender?.to_string(), service.to_string()))
    } else if service.is_empty() {
        None
    } else {
        Some(split_service(service))
    }
}

pub(super) fn split_service(service: &str) -> (String, String) {
    match service.find('/') {
        Some(i) => (service[..i].to_string(), service[i..].to_string()),
        None => (service.to_string(), "/StatusNotifierItem".to_string()),
    }
}

/// Each item costs property round-trips and a match rule per [`super::ITEM_SIGNALS`] entry, and
/// the bus daemon caps rules per connection.
pub(super) const MAX_REGISTRATIONS: usize = 64;
pub(super) const MAX_REGISTRATIONS_PER_SENDER: usize = 8;

pub(super) struct Published {
    pub(super) key: String,
    pub(super) sender: String,
}

#[derive(Debug, PartialEq)]
pub(super) enum Admission {
    Known,
    New,
    TotalFull,
    SenderFull,
}

pub(super) fn admit(published: &[Published], key: &str, sender: &str) -> Admission {
    if published.iter().any(|p| p.key == key) {
        return Admission::Known;
    }
    if published.len() >= MAX_REGISTRATIONS {
        return Admission::TotalFull;
    }
    if published.iter().filter(|p| p.sender == sender).count() >= MAX_REGISTRATIONS_PER_SENDER {
        return Admission::SenderFull;
    }
    Admission::New
}

pub(super) struct Watcher {
    pub(super) items: Arc<Mutex<Vec<Published>>>,
    pub(super) registrations: async_channel::Sender<Wake>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] hdr: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let Some(sender) = hdr.sender().map(|s| s.as_str()) else {
            return;
        };
        let Some((bus, path)) = resolve_registration(service, Some(sender)) else {
            log::warn!("tray: ignoring item registration {service:?}");
            return;
        };
        let key = format!("{bus}{path}");
        {
            let mut items = lock(&self.items);
            match admit(&items, &key, sender) {
                Admission::Known => {}
                Admission::New => items.push(Published {
                    key: key.clone(),
                    sender: sender.to_string(),
                }),
                Admission::TotalFull => {
                    log::warn!("tray: refusing item {key}: {MAX_REGISTRATIONS} already registered");
                    return;
                }
                Admission::SenderFull => {
                    log::warn!(
                        "tray: refusing item {key}: {sender} already registered {MAX_REGISTRATIONS_PER_SENDER}"
                    );
                    return;
                }
            }
        }
        let _ = self
            .registrations
            .send(Wake::Registered { bus, path })
            .await;
        let _ = Self::status_notifier_item_registered(&emitter, &key).await;
    }

    async fn register_status_notifier_host(
        &self,
        _service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let _ = Self::status_notifier_host_registered(&emitter).await;
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        let items = lock(&self.items);
        items.iter().map(|p| p.key.clone()).collect()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub(super) async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

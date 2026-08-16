//! Platform mDNS/DNS-SD adapters.
//!
//! macOS routes publication through the system Bonjour daemon. Raw multicast sockets can appear
//! healthy locally while their announcements remain local-only under macOS's network privacy
//! controls, so a Bonjour callback is the only state reported as announced. Browsing continues
//! through bounded DNS-SD multicast events so all resolved addresses can be ranked consistently.

#[cfg(target_os = "macos")]
mod implementation {
    use std::{collections::BTreeMap, sync::Mutex};

    use dns_sd_native::{ServiceRegistration, ServiceRegistrationBuilder};
    use mdns_sd::{
        DaemonEvent, Receiver, ResolvedService, ServiceDaemon, ServiceEvent, TryRecvError,
    };
    use tokio::runtime::{Builder as RuntimeBuilder, Runtime};

    use super::super::{
        announcement_properties, service_instance_name, LanDiscoveryError, LanResolvedRecord,
        LanSessionAdvertisement, LAN_DISCOVERY_SERVICE_TYPE,
    };

    pub(crate) struct Advertiser {
        registration: Mutex<Option<ServiceRegistration>>,
        runtime: Runtime,
    }

    impl Advertiser {
        pub(crate) fn start(
            advertisement: &LanSessionAdvertisement,
        ) -> Result<Self, LanDiscoveryError> {
            let runtime = RuntimeBuilder::new_current_thread()
                .build()
                .map_err(LanDiscoveryError::daemon)?;
            let mut registration = ServiceRegistrationBuilder::new(
                "_hexgame._udp",
                advertisement.connection_code.endpoint.port(),
            );
            registration.name(service_instance_name(advertisement));
            for (key, value) in announcement_properties(advertisement) {
                registration.add_txt_record_key_string(key, value);
            }
            let registered = runtime
                .block_on(registration.register())
                .map_err(LanDiscoveryError::daemon)?;
            Ok(Self {
                registration: Mutex::new(Some(registered)),
                runtime,
            })
        }

        pub(crate) fn poll_health(&mut self) -> Result<(), LanDiscoveryError> {
            Ok(())
        }

        pub(crate) fn is_announced(&self) -> bool {
            self.registration
                .lock()
                .is_ok_and(|registration| registration.is_some())
        }
    }

    impl Drop for Advertiser {
        fn drop(&mut self) {
            let registration = self.registration.get_mut().ok().and_then(Option::take);
            if let Some(registration) = registration {
                let _status = self.runtime.block_on(registration.unregister());
            }
        }
    }

    pub(crate) struct Browser {
        daemon: ServiceDaemon,
        events: Receiver<ServiceEvent>,
        monitor: Receiver<DaemonEvent>,
    }

    impl Browser {
        pub(crate) fn start() -> Result<Self, LanDiscoveryError> {
            let daemon = ServiceDaemon::new().map_err(LanDiscoveryError::daemon)?;
            let monitor = daemon.monitor().map_err(LanDiscoveryError::daemon)?;
            let events = daemon
                .browse(LAN_DISCOVERY_SERVICE_TYPE)
                .map_err(LanDiscoveryError::daemon)?;
            Ok(Self {
                daemon,
                events,
                monitor,
            })
        }

        pub(crate) fn poll_health(&mut self) -> Result<(), LanDiscoveryError> {
            loop {
                match self.monitor.try_recv() {
                    Ok(DaemonEvent::Error(error)) => {
                        return Err(LanDiscoveryError::daemon(error));
                    }
                    Ok(_) => {}
                    Err(TryRecvError::Empty) => return Ok(()),
                    Err(TryRecvError::Disconnected) => {
                        return Err(LanDiscoveryError::DaemonStopped);
                    }
                }
            }
        }

        pub(crate) fn try_recv(&mut self) -> Result<Option<BrowserEvent>, LanDiscoveryError> {
            loop {
                match self.events.try_recv() {
                    Ok(ServiceEvent::ServiceResolved(service)) => {
                        return Ok(Some(BrowserEvent::Resolved(resolved_record(&service))));
                    }
                    Ok(ServiceEvent::ServiceRemoved(_service_type, fullname)) => {
                        return Ok(Some(BrowserEvent::Removed(fullname)));
                    }
                    Ok(_) => {}
                    Err(TryRecvError::Empty) => return Ok(None),
                    Err(TryRecvError::Disconnected) => {
                        return Err(LanDiscoveryError::DaemonStopped);
                    }
                }
            }
        }
    }

    impl Drop for Browser {
        fn drop(&mut self) {
            let _status = self.daemon.stop_browse(LAN_DISCOVERY_SERVICE_TYPE);
            let _status = self.daemon.shutdown();
        }
    }

    pub(crate) enum BrowserEvent {
        Resolved(LanResolvedRecord),
        Removed(String),
    }

    fn resolved_record(service: &ResolvedService) -> LanResolvedRecord {
        let properties = service
            .get_properties()
            .iter()
            .filter_map(|property| {
                property.val().and_then(|value| {
                    std::str::from_utf8(value)
                        .ok()
                        .map(|value| (property.key().to_owned(), value.to_owned()))
                })
            })
            .collect::<BTreeMap<_, _>>();
        LanResolvedRecord {
            service_id: service.fullname.clone(),
            service_type: service.ty_domain.clone(),
            addresses: service
                .get_addresses()
                .iter()
                .map(mdns_sd::ScopedIp::to_ip_addr)
                .collect(),
            port: service.port,
            properties,
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod implementation {
    use std::collections::BTreeMap;

    use mdns_sd::{
        DaemonEvent, Receiver, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo,
        TryRecvError,
    };

    use super::super::{
        announcement_properties, service_instance_name, LanDiscoveryError, LanResolvedRecord,
        LanSessionAdvertisement, LAN_DISCOVERY_SERVICE_TYPE,
    };

    pub(crate) struct Advertiser {
        daemon: ServiceDaemon,
        monitor: Receiver<DaemonEvent>,
        fullname: String,
        announced: bool,
    }

    impl Advertiser {
        pub(crate) fn start(
            advertisement: &LanSessionAdvertisement,
        ) -> Result<Self, LanDiscoveryError> {
            let daemon = ServiceDaemon::new().map_err(LanDiscoveryError::daemon)?;
            let monitor = daemon.monitor().map_err(LanDiscoveryError::daemon)?;
            let info = service_info(advertisement)?;
            let fullname = info.get_fullname().to_owned();
            daemon.register(info).map_err(LanDiscoveryError::daemon)?;
            Ok(Self {
                daemon,
                monitor,
                fullname,
                announced: false,
            })
        }

        pub(crate) fn poll_health(&mut self) -> Result<(), LanDiscoveryError> {
            loop {
                match self.monitor.try_recv() {
                    Ok(DaemonEvent::Error(error)) => {
                        return Err(LanDiscoveryError::daemon(error));
                    }
                    Ok(DaemonEvent::Announce(fullname, _interface))
                        if fullname == self.fullname =>
                    {
                        self.announced = true;
                    }
                    Ok(_) => {}
                    Err(TryRecvError::Empty) => return Ok(()),
                    Err(TryRecvError::Disconnected) => {
                        return Err(LanDiscoveryError::DaemonStopped);
                    }
                }
            }
        }

        pub(crate) const fn is_announced(&self) -> bool {
            self.announced
        }
    }

    impl Drop for Advertiser {
        fn drop(&mut self) {
            let _status = self.daemon.unregister(&self.fullname);
            let _status = self.daemon.shutdown();
        }
    }

    pub(crate) struct Browser {
        daemon: ServiceDaemon,
        events: Receiver<ServiceEvent>,
        monitor: Receiver<DaemonEvent>,
    }

    impl Browser {
        pub(crate) fn start() -> Result<Self, LanDiscoveryError> {
            let daemon = ServiceDaemon::new().map_err(LanDiscoveryError::daemon)?;
            let monitor = daemon.monitor().map_err(LanDiscoveryError::daemon)?;
            let events = daemon
                .browse(LAN_DISCOVERY_SERVICE_TYPE)
                .map_err(LanDiscoveryError::daemon)?;
            Ok(Self {
                daemon,
                events,
                monitor,
            })
        }

        pub(crate) fn poll_health(&mut self) -> Result<(), LanDiscoveryError> {
            loop {
                match self.monitor.try_recv() {
                    Ok(DaemonEvent::Error(error)) => {
                        return Err(LanDiscoveryError::daemon(error));
                    }
                    Ok(_) => {}
                    Err(TryRecvError::Empty) => return Ok(()),
                    Err(TryRecvError::Disconnected) => {
                        return Err(LanDiscoveryError::DaemonStopped);
                    }
                }
            }
        }

        pub(crate) fn try_recv(&mut self) -> Result<Option<BrowserEvent>, LanDiscoveryError> {
            loop {
                match self.events.try_recv() {
                    Ok(ServiceEvent::ServiceResolved(service)) => {
                        return Ok(Some(BrowserEvent::Resolved(resolved_record(&service))));
                    }
                    Ok(ServiceEvent::ServiceRemoved(_service_type, fullname)) => {
                        return Ok(Some(BrowserEvent::Removed(fullname)));
                    }
                    Ok(_) => {}
                    Err(TryRecvError::Empty) => return Ok(None),
                    Err(TryRecvError::Disconnected) => {
                        return Err(LanDiscoveryError::DaemonStopped);
                    }
                }
            }
        }
    }

    impl Drop for Browser {
        fn drop(&mut self) {
            let _status = self.daemon.stop_browse(LAN_DISCOVERY_SERVICE_TYPE);
            let _status = self.daemon.shutdown();
        }
    }

    pub(crate) enum BrowserEvent {
        Resolved(LanResolvedRecord),
        Removed(String),
    }

    fn service_info(
        advertisement: &LanSessionAdvertisement,
    ) -> Result<ServiceInfo, LanDiscoveryError> {
        let session_hex = super::super::encode_hex(&advertisement.session_instance_id.to_bytes());
        let hostname = format!("hex-{session_hex}.local.");
        let properties = announcement_properties(advertisement);
        ServiceInfo::new(
            LAN_DISCOVERY_SERVICE_TYPE,
            &service_instance_name(advertisement),
            &hostname,
            "",
            advertisement.connection_code.endpoint.port(),
            properties.as_slice(),
        )
        .map(ServiceInfo::enable_addr_auto)
        .map_err(LanDiscoveryError::daemon)
    }

    fn resolved_record(service: &ResolvedService) -> LanResolvedRecord {
        let properties = service
            .get_properties()
            .iter()
            .filter_map(|property| {
                property.val().and_then(|value| {
                    std::str::from_utf8(value)
                        .ok()
                        .map(|value| (property.key().to_owned(), value.to_owned()))
                })
            })
            .collect::<BTreeMap<_, _>>();
        LanResolvedRecord {
            service_id: service.fullname.clone(),
            service_type: service.ty_domain.clone(),
            addresses: service
                .get_addresses()
                .iter()
                .map(mdns_sd::ScopedIp::to_ip_addr)
                .collect(),
            port: service.port,
            properties,
        }
    }
}

pub(super) use implementation::{Advertiser, Browser, BrowserEvent};

//! SystemConfiguration sessions hold the native preferences lock across comparison and commit.
use super::*;
use core_foundation::data::CFData;
use core_foundation::{
    base::{CFRelease, TCFType},
    dictionary::CFDictionaryRef,
    propertylist,
    string::{CFString, CFStringRef},
};
use security_framework_sys::authorization::{
    AuthorizationCreate, AuthorizationFree, AuthorizationRef,
};
use std::ptr;
use system_configuration_sys::preferences::*;

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCPreferencesPathSetValue(
        prefs: SCPreferencesRef,
        path: CFStringRef,
        value: CFDictionaryRef,
    ) -> u8;
    fn SCPreferencesPathRemoveValue(prefs: SCPreferencesRef, path: CFStringRef) -> u8;
}

fn proxy_path(service: &str) -> Result<String> {
    let valid = service.len() == 36
        && service.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        });
    if !valid {
        return Err("expected an existing network service UUID".into());
    }
    Ok(format!("/NetworkServices/{service}/Proxies"))
}

pub(super) struct Session {
    prefs: SCPreferencesRef,
    auth: AuthorizationRef,
    locked: bool,
    private: bool,
}
impl Session {
    pub fn open(writing: bool) -> Result<Self> {
        Self::create(writing, None)
    }
    fn create(writing: bool, private_id: Option<&str>) -> Result<Self> {
        if writing && std::env::var_os("HPS_FORBID_HOST_NETWORK_CHANGES").is_some() {
            return Err(
                "macOS network preference writes disabled for this development session".into(),
            );
        }
        // Host tests may read preferences but must never acquire network write access.
        #[cfg(test)]
        if writing {
            return Err("macOS network preference writes are forbidden in host tests".into());
        }
        let name = CFString::new("HPS local system proxy");
        let private = private_id.map(CFString::new);
        let prefs_id = private
            .as_ref()
            .map_or(ptr::null(), TCFType::as_concrete_TypeRef);
        let mut auth = ptr::null_mut();
        // Only native OS authorization may prompt. No WebView or daemon is elevated.
        if writing {
            let error = unsafe { AuthorizationCreate(ptr::null(), ptr::null(), 0, &mut auth) };
            if error != 0 {
                return Err(format!("OS authorization unavailable: {error}"));
            }
        }
        // One native constructor for both modes: the preference scope can never
        // diverge merely because an authorization reference became available.
        let prefs = unsafe {
            SCPreferencesCreateWithAuthorization(
                ptr::null(),
                name.as_concrete_TypeRef(),
                prefs_id,
                auth.cast(),
            )
        };
        if prefs.is_null() {
            if !auth.is_null() {
                unsafe {
                    AuthorizationFree(auth, 0);
                }
            }
            return Err("cannot open SystemConfiguration preferences".into());
        }
        let mut session = Self {
            prefs,
            auth,
            locked: false,
            private: private.is_some(),
        };
        if writing {
            if unsafe { SCPreferencesLock(prefs, 0) } == 0 {
                return Err("OS network preferences are busy or authorization was declined".into());
            }
            session.locked = true;
        }
        Ok(session)
    }
    fn network(&self) -> Result<plist::Value> {
        let key = CFString::new("NetworkServices");
        let value = unsafe { SCPreferencesGetValue(self.prefs, key.as_concrete_TypeRef()) };
        if value.is_null() {
            return Ok(plist::Value::Dictionary(plist::Dictionary::new()));
        }
        let data = propertylist::create_data(value, propertylist::kCFPropertyListXMLFormat_v1_0)
            .map_err(|_| "cannot serialize network preferences")?;
        plist::Value::from_reader_xml(std::io::Cursor::new(data.bytes()))
            .map_err(|_| "invalid network preferences".into())
    }
    fn commit_proxy(&mut self, service: &str, value: Option<&plist::Value>) -> Result<()> {
        #[cfg(test)]
        {
            let _ = (service, value);
            Err("macOS network preference writes are forbidden in host tests".into())
        }
        #[cfg(not(test))]
        self.commit_authorized(service, value)
    }
    #[cfg(not(test))]
    fn commit_authorized(&mut self, service: &str, value: Option<&plist::Value>) -> Result<()> {
        if !self.locked {
            return Err("a writable preferences session is required".into());
        }
        self.stage_proxy(service, value)?;
        if unsafe { SCPreferencesCommitChanges(self.prefs) } == 0 {
            return Err("OS proxy commit refused; original backup retained".into());
        }
        if !self.private && unsafe { SCPreferencesApplyChanges(self.prefs) } == 0 {
            return Err(
                "OS proxy commit succeeded but apply failed; original backup retained".into(),
            );
        }
        Ok(())
    }
    /// Only edits this session's in-memory dictionary. Persistent publication is
    /// separate and is never compiled into the host test build.
    fn stage_proxy(&mut self, service: &str, value: Option<&plist::Value>) -> Result<()> {
        #[cfg(test)]
        if !self.private {
            return Err("host tests cannot stage default network preferences".into());
        }
        let path = CFString::new(&proxy_path(service)?);
        let existing = self.read(service)?; // Never creates or replaces a network service.
        if value.is_none() && existing.is_none() {
            return Ok(());
        }
        let changed = if let Some(value) = value {
            if value.as_dictionary().is_none() {
                return Err("proxy settings must be a dictionary".into());
            }
            let mut bytes = Vec::new();
            value
                .to_writer_xml(&mut bytes)
                .map_err(|_| "cannot serialize proxy settings")?;
            let (cf_value, _) = propertylist::create_with_data(CFData::from_buffer(&bytes), 0)
                .map_err(|_| "cannot create proxy settings")?;
            let result = unsafe {
                SCPreferencesPathSetValue(self.prefs, path.as_concrete_TypeRef(), cf_value.cast())
            };
            unsafe {
                CFRelease(cf_value);
            }
            result
        } else {
            unsafe { SCPreferencesPathRemoveValue(self.prefs, path.as_concrete_TypeRef()) }
        };
        if changed == 0 {
            return Err("OS proxy staging refused; no persistent commit performed".into());
        }
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if self.locked {
                SCPreferencesUnlock(self.prefs);
            }
            CFRelease(self.prefs);
            if !self.auth.is_null() {
                AuthorizationFree(self.auth, 0);
            }
        }
    }
}
impl Backend for Session {
    fn services(&self) -> Result<Vec<NetworkService>> {
        Ok(self
            .network()?
            .as_dictionary()
            .ok_or("invalid network services")?
            .iter()
            .filter(|(id, _)| proxy_path(id).is_ok())
            .map(|(id, value)| NetworkService {
                id: id.clone(),
                name: value
                    .as_dictionary()
                    .and_then(|v| v.get("UserDefinedName"))
                    .and_then(plist::Value::as_string)
                    .unwrap_or(id)
                    .into(),
            })
            .collect())
    }
    fn read(&self, service: &str) -> Result<Option<plist::Value>> {
        let network = self.network()?;
        let value = network
            .as_dictionary()
            .and_then(|n| n.get(service))
            .and_then(plist::Value::as_dictionary)
            .ok_or("OS network service disappeared; backup retained")?;
        Ok(value.get("Proxies").cloned())
    }
    fn write(&mut self, service: &str, value: Option<&plist::Value>) -> Result<()> {
        self.commit_proxy(service, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_tests_refuse_all_writable_preferences_sessions() {
        assert!(Session::create(true, None).is_err());
        assert!(Session::create(true, Some("/tmp/hps-isolated-network.plist")).is_err());
        let mut readonly = Session::open(false).unwrap();
        assert!(
            readonly
                .write("00000000-0000-4000-8000-000000000001", None)
                .is_err()
        );
    }
    #[test]
    fn explicit_readonly_preferences_path_never_falls_back_to_system_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("readonly-preferences.plist");
        let mut service = plist::Dictionary::new();
        service.insert(
            "UserDefinedName".into(),
            plist::Value::String("Read only fixture".into()),
        );
        let mut services = plist::Dictionary::new();
        services.insert(
            "00000000-0000-4000-8000-000000000003".into(),
            plist::Value::Dictionary(service),
        );
        let mut fixture = plist::Dictionary::new();
        fixture.insert("NetworkServices".into(), plist::Value::Dictionary(services));
        plist::Value::Dictionary(fixture)
            .to_file_xml(&path)
            .unwrap();
        let readonly = Session::create(false, Some(path.to_str().unwrap())).unwrap();
        let services = readonly.services().unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].id, "00000000-0000-4000-8000-000000000003");
    }
    #[test]
    fn native_proxy_path_stages_and_restores_only_in_private_session_memory() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("readonly-native-path.plist");
        let selected = "00000000-0000-4000-8000-000000000001";
        let other = "00000000-0000-4000-8000-000000000002";
        let mut service = plist::Dictionary::new();
        service.insert(
            "DNS".into(),
            plist::Value::String("preserve-dns-fixture".into()),
        );
        service.insert(
            "IPv4".into(),
            plist::Value::String("preserve-address-fixture".into()),
        );
        let mut services = plist::Dictionary::new();
        services.insert(selected.into(), plist::Value::Dictionary(service.clone()));
        services.insert(other.into(), plist::Value::Dictionary(service));
        let mut fixture = plist::Dictionary::new();
        fixture.insert("NetworkServices".into(), plist::Value::Dictionary(services));
        let fixture = plist::Value::Dictionary(fixture);
        fixture.to_file_xml(&path).unwrap();
        let original_file = std::fs::read(&path).unwrap();
        let mut session = Session::create(false, Some(path.to_str().unwrap())).unwrap();
        let original = session.network().unwrap();
        let input = Apply {
            network_service: selected.into(),
            http: Some(18983),
            socks5: None,
        };
        let value = applied(None, &input).unwrap();
        session.stage_proxy(selected, Some(&value)).unwrap();
        assert_eq!(session.read(selected).unwrap(), Some(value));
        assert_eq!(session.read(other).unwrap(), None);
        let current = session.network().unwrap();
        let selected_before = original.as_dictionary().unwrap()[selected]
            .as_dictionary()
            .unwrap();
        let selected_after = current.as_dictionary().unwrap()[selected]
            .as_dictionary()
            .unwrap();
        assert_eq!(selected_before.get("DNS"), selected_after.get("DNS"));
        assert_eq!(selected_before.get("IPv4"), selected_after.get("IPv4"));
        assert!(session.stage_proxy("../other", None).is_err());
        session.stage_proxy(selected, None).unwrap();
        assert_eq!(session.network().unwrap(), original);
        drop(session);
        assert_eq!(std::fs::read(&path).unwrap(), original_file);
        let reopened = Session::create(false, Some(path.to_str().unwrap())).unwrap();
        assert_eq!(reopened.read(selected).unwrap(), None);
    }
    #[test]
    fn actual_network_services_are_readable_without_elevation() {
        let session = Session::open(false).unwrap();
        let services = session.services().unwrap();
        assert!(!services.is_empty());
        for service in services {
            let _ = session.read(&service.id).unwrap();
        }
    }
}

//! SystemConfiguration sessions hold the native preferences lock across comparison and commit.
use super::*;
#[cfg(not(test))]
use core_foundation::data::CFData;
use core_foundation::{
    base::{CFRelease, TCFType},
    propertylist,
    string::CFString,
};
use security_framework_sys::authorization::{
    AuthorizationCreate, AuthorizationFree, AuthorizationRef,
};
use std::ptr;
use system_configuration_sys::preferences::*;

pub(super) struct Session {
    prefs: SCPreferencesRef,
    auth: AuthorizationRef,
    locked: bool,
    #[cfg(not(test))]
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
        let prefs = unsafe {
            if auth.is_null() {
                SCPreferencesCreate(ptr::null(), name.as_concrete_TypeRef(), prefs_id)
            } else {
                SCPreferencesCreateWithAuthorization(
                    ptr::null(),
                    name.as_concrete_TypeRef(),
                    prefs_id,
                    auth.cast(),
                )
            }
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
            #[cfg(not(test))]
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
    fn commit(&mut self, network: &plist::Value) -> Result<()> {
        #[cfg(test)]
        {
            let _ = network;
            Err("macOS network preference writes are forbidden in host tests".into())
        }
        #[cfg(not(test))]
        self.commit_authorized(network)
    }
    #[cfg(not(test))]
    fn commit_authorized(&mut self, network: &plist::Value) -> Result<()> {
        if !self.locked {
            return Err("a writable preferences session is required".into());
        }
        let mut bytes = Vec::new();
        network
            .to_writer_xml(&mut bytes)
            .map_err(|_| "cannot serialize OS settings")?;
        let (value, _) = propertylist::create_with_data(CFData::from_buffer(&bytes), 0)
            .map_err(|_| "cannot create OS settings")?;
        let key = CFString::new("NetworkServices");
        let set = unsafe { SCPreferencesSetValue(self.prefs, key.as_concrete_TypeRef(), value) };
        unsafe {
            CFRelease(value);
        }
        if set == 0 || unsafe { SCPreferencesCommitChanges(self.prefs) } == 0 {
            return Err("OS proxy commit refused; original backup retained".into());
        }
        if !self.private && unsafe { SCPreferencesApplyChanges(self.prefs) } == 0 {
            return Err(
                "OS proxy commit succeeded but apply failed; original backup retained".into(),
            );
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
        let mut network = self.network()?;
        let dictionary = network
            .as_dictionary_mut()
            .and_then(|n| n.get_mut(service))
            .and_then(plist::Value::as_dictionary_mut)
            .ok_or("OS network service disappeared; backup retained")?;
        if let Some(value) = value {
            dictionary.insert("Proxies".into(), value.clone());
        } else {
            dictionary.remove("Proxies");
        }
        self.commit(&network)
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
                .commit(&plist::Value::Dictionary(plist::Dictionary::new()))
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
            "hps-readonly-fixture".into(),
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
        assert_eq!(services[0].id, "hps-readonly-fixture");
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

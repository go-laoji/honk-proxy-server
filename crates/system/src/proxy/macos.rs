//! SystemConfiguration sessions hold the native preferences lock across comparison and commit.
use super::*;
use core_foundation::{
    base::{CFRelease, TCFType},
    data::CFData,
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
    private: bool,
}
impl Session {
    pub fn open(writing: bool) -> Result<Self> {
        Self::create(writing, None)
    }
    fn create(writing: bool, private_id: Option<&str>) -> Result<Self> {
        let name = CFString::new("HPS local system proxy");
        let private = private_id.map(CFString::new);
        let mut auth = ptr::null_mut();
        // Only native OS authorization may prompt. No WebView or daemon is elevated.
        if writing && private.is_none() {
            let error = unsafe { AuthorizationCreate(ptr::null(), ptr::null(), 0, &mut auth) };
            if error != 0 {
                return Err(format!("OS authorization unavailable: {error}"));
            }
        }
        let prefs = unsafe {
            if auth.is_null() {
                SCPreferencesCreate(
                    ptr::null(),
                    name.as_concrete_TypeRef(),
                    private
                        .as_ref()
                        .map_or(ptr::null(), TCFType::as_concrete_TypeRef),
                )
            } else {
                SCPreferencesCreateWithAuthorization(
                    ptr::null(),
                    name.as_concrete_TypeRef(),
                    ptr::null(),
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
    #[ignore = "requires an administrator for an isolated SCPreferences file; never modifies active network preferences"]
    fn real_systemconfiguration_private_preferences_commit_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let id = dir.path().join("isolated-network.plist");
        let mut backend = Session::create(true, Some(id.to_str().unwrap())).unwrap();
        let mut services = plist::Dictionary::new();
        let mut service = plist::Dictionary::new();
        service.insert(
            "UserDefinedName".into(),
            plist::Value::String("Isolated validation".into()),
        );
        services.insert("isolated".into(), plist::Value::Dictionary(service));
        backend.commit(&plist::Value::Dictionary(services)).unwrap();
        let input = Apply {
            network_service: "isolated".into(),
            http: Some(18983),
            socks5: Some(18984),
        };
        apply_with(dir.path(), &mut backend, "http://127.0.0.1:17983/", &input).unwrap();
        drop(backend);
        let mut backend = Session::create(true, Some(id.to_str().unwrap())).unwrap();
        assert_eq!(status_with(dir.path(), &backend).unwrap().state, "owned");
        restore_with(dir.path(), &mut backend).unwrap();
        assert_eq!(backend.read("isolated").unwrap(), None);
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

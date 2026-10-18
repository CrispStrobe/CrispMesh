//! Finding a shared computer on the local network without typing its address: while "Let other devices use this
//! computer" is on, the desktop app announces `_crispmesh._tcp` (Bonjour, through the system's own mDNS service);
//! the phone lists what it finds. The announcement carries no token: pairing still needs the code shown on the
//! computer (share.rs, `POST /api/pair`). Apple platforms only (DNS-SD C API in libSystem; on iOS the app
//! declares the service type in NSBonjourServices); elsewhere nothing is announced and nothing is found.

// Phones only look; announcing is for the desktop app's sharing.
#![cfg_attr(mobile, allow(dead_code, unused_imports))]

use serde::Serialize;

/// The service type announced and looked for.
pub const SERVICE: &str = "_crispmesh._tcp";

/// A computer found on the network: its Bonjour name (the computer's name) and the engine's address.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Found {
    pub name: String,
    pub url: String,
}

/// TXT record of the announcement: `v=1` and the address the computer would use (`addr=192.168.1.20`).
pub fn txt_record(address: Option<&str>) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in std::iter::once("v=1".to_string()).chain(address.map(|a| format!("addr={a}"))) {
        out.push(entry.len().min(255) as u8);
        out.extend_from_slice(&entry.as_bytes()[..entry.len().min(255)]);
    }
    out
}

/// The value of `key` in a TXT record.
pub fn txt_value(record: &[u8], key: &str) -> Option<String> {
    let mut at = 0;
    while at < record.len() {
        let length = record[at] as usize;
        let entry = record.get(at + 1..at + 1 + length)?;
        if let Some((name, value)) = std::str::from_utf8(entry).ok()?.split_once('=') {
            if name.eq_ignore_ascii_case(key) {
                return Some(value.to_string());
            }
        }
        at += 1 + length;
    }
    None
}

#[cfg(target_vendor = "apple")]
mod dnssd {
    use super::{txt_value, Found, SERVICE};
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::time::{Duration, Instant};

    type Ref = *mut c_void;
    type Error = i32;
    const ADD: u32 = 0x2;

    type BrowseReply = extern "C" fn(Ref, u32, u32, Error, *const c_char, *const c_char, *const c_char, *mut c_void);
    type ResolveReply = extern "C" fn(Ref, u32, u32, Error, *const c_char, *const c_char, u16, u16, *const u8, *mut c_void);
    type RegisterReply = extern "C" fn(Ref, u32, Error, *const c_char, *const c_char, *const c_char, *mut c_void);

    extern "C" {
        #[allow(clippy::too_many_arguments)]
        fn DNSServiceRegister(
            sd: *mut Ref,
            flags: u32,
            interface: u32,
            name: *const c_char,
            kind: *const c_char,
            domain: *const c_char,
            host: *const c_char,
            port: u16,
            txt_length: u16,
            txt: *const c_void,
            reply: Option<RegisterReply>,
            context: *mut c_void,
        ) -> Error;
        fn DNSServiceBrowse(sd: *mut Ref, flags: u32, interface: u32, kind: *const c_char, domain: *const c_char, reply: BrowseReply, context: *mut c_void) -> Error;
        #[allow(clippy::too_many_arguments)]
        fn DNSServiceResolve(
            sd: *mut Ref,
            flags: u32,
            interface: u32,
            name: *const c_char,
            kind: *const c_char,
            domain: *const c_char,
            reply: ResolveReply,
            context: *mut c_void,
        ) -> Error;
        fn DNSServiceRefSockFD(sd: Ref) -> c_int;
        fn DNSServiceProcessResult(sd: Ref) -> Error;
        fn DNSServiceRefDeallocate(sd: Ref);
    }

    /// The announcement; withdrawn when dropped.
    pub struct Advert(Ref);

    // The reference is only handed to DNSServiceRefDeallocate, once.
    unsafe impl Send for Advert {}

    impl Drop for Advert {
        fn drop(&mut self) {
            unsafe { DNSServiceRefDeallocate(self.0) };
        }
    }

    /// Announces the engine at `port` under the computer's own name.
    pub fn announce(port: u16, txt: &[u8]) -> Result<Advert, String> {
        let kind = CString::new(SERVICE).unwrap();
        let mut sd: Ref = std::ptr::null_mut();
        let error = unsafe {
            DNSServiceRegister(
                &mut sd,
                0,
                0,
                std::ptr::null(),
                kind.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                port.to_be(),
                txt.len() as u16,
                txt.as_ptr() as *const c_void,
                None,
                std::ptr::null_mut(),
            )
        };
        if error != 0 || sd.is_null() {
            return Err(format!("the network announcement failed ({error})"));
        }
        Ok(Advert(sd))
    }

    /// Hands results to the callbacks until `until` or until `done` says so.
    fn process(sd: Ref, until: Instant, done: &dyn Fn() -> bool) {
        let fd = unsafe { DNSServiceRefSockFD(sd) };
        if fd < 0 {
            return;
        }
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() || done() {
                return;
            }
            let mut poll = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            let ready = unsafe { libc::poll(&mut poll, 1, left.as_millis().min(i32::MAX as u128) as c_int) };
            if ready <= 0 || unsafe { DNSServiceProcessResult(sd) } != 0 {
                return;
            }
        }
    }

    struct Names(Vec<(String, String, u32)>);

    extern "C" fn browsed(_: Ref, flags: u32, interface: u32, error: Error, name: *const c_char, _: *const c_char, domain: *const c_char, context: *mut c_void) {
        let names = unsafe { &mut *(context as *mut Names) };
        if error == 0 && flags & ADD != 0 && !name.is_null() && !domain.is_null() {
            let (name, domain) = unsafe { (CStr::from_ptr(name).to_string_lossy().into_owned(), CStr::from_ptr(domain).to_string_lossy().into_owned()) };
            if !names.0.iter().any(|(n, d, _)| *n == name && *d == domain) {
                names.0.push((name, domain, interface));
            }
        }
    }

    struct Resolved(Option<(String, u16, Option<String>)>);

    extern "C" fn resolved(_: Ref, _: u32, _: u32, error: Error, _: *const c_char, host: *const c_char, port: u16, txt_length: u16, txt: *const u8, context: *mut c_void) {
        let out = unsafe { &mut *(context as *mut Resolved) };
        if error == 0 && !host.is_null() && out.0.is_none() {
            let host = unsafe { CStr::from_ptr(host).to_string_lossy().trim_end_matches('.').to_string() };
            let record = if txt.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(txt, txt_length as usize) } };
            out.0 = Some((host, u16::from_be(port), txt_value(record, "addr")));
        }
    }

    /// Computers announcing the service, looked for during about `wait` (then each one's address).
    pub fn find(wait: Duration) -> Vec<Found> {
        let kind = CString::new(SERVICE).unwrap();
        let mut names = Names(Vec::new());
        let mut sd: Ref = std::ptr::null_mut();
        let error = unsafe { DNSServiceBrowse(&mut sd, 0, 0, kind.as_ptr(), std::ptr::null(), browsed, &mut names as *mut Names as *mut c_void) };
        if error != 0 || sd.is_null() {
            return Vec::new();
        }
        // Answers trickle in over the whole wait.
        process(sd, Instant::now() + wait, &|| false);
        unsafe { DNSServiceRefDeallocate(sd) };
        let mut found = Vec::new();
        for (name, domain, interface) in names.0 {
            let (Ok(c_name), Ok(c_domain)) = (CString::new(name.clone()), CString::new(domain)) else { continue };
            let mut out = Resolved(None);
            let mut sd: Ref = std::ptr::null_mut();
            let error = unsafe {
                DNSServiceResolve(&mut sd, 0, interface, c_name.as_ptr(), kind.as_ptr(), c_domain.as_ptr(), resolved, &mut out as *mut Resolved as *mut c_void)
            };
            if error != 0 || sd.is_null() {
                continue;
            }
            let flag = &out as *const Resolved;
            process(sd, Instant::now() + Duration::from_secs(2), &|| unsafe { (*flag).0.is_some() });
            unsafe { DNSServiceRefDeallocate(sd) };
            if let Some((host, port, address)) = out.0 {
                let host = address.unwrap_or(host);
                let host = if host.contains(':') { format!("[{host}]") } else { host };
                found.push(Found { name, url: format!("http://{host}:{port}") });
            }
        }
        found
    }
}

#[cfg(target_vendor = "apple")]
pub use dnssd::{announce, find, Advert};

/// Elsewhere: no announcement.
#[cfg(not(target_vendor = "apple"))]
pub struct Advert;

#[cfg(not(target_vendor = "apple"))]
pub fn announce(_port: u16, _txt: &[u8]) -> Result<Advert, String> {
    Err("announcing on the network needs macOS or iOS".into())
}

#[cfg(not(target_vendor = "apple"))]
pub fn find(_wait: std::time::Duration) -> Vec<Found> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txt_records_round_trip() {
        let record = txt_record(Some("192.168.1.20"));
        assert_eq!(record[0] as usize, "v=1".len());
        assert_eq!(txt_value(&record, "addr").as_deref(), Some("192.168.1.20"));
        assert_eq!(txt_value(&record, "v").as_deref(), Some("1"));
        assert_eq!(txt_value(&txt_record(None), "addr"), None);
        assert_eq!(txt_value(&[9, b'a'], "addr"), None);
    }

    /// Announces and finds itself through the system's mDNS service (macOS, iOS).
    #[cfg(target_vendor = "apple")]
    #[test]
    fn announces_and_finds_itself() {
        let advert = announce(48123, &txt_record(Some("127.0.0.1"))).unwrap();
        let found = find(std::time::Duration::from_secs(3));
        drop(advert);
        assert!(found.iter().any(|f| f.url == "http://127.0.0.1:48123"), "{found:?}");
    }
}

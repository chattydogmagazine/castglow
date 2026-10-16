//! Minimal CoreFoundation bindings (no external crates, no build script).
#![allow(non_upper_case_globals, non_snake_case, dead_code)]

use std::ffi::{c_char, c_void, CString};

pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFNumberRef = *const c_void;
pub type CFSetRef = *const c_void;
pub type CFMutableDictionaryRef = *mut c_void;
pub type CFAllocatorRef = *const c_void;
pub type CFIndex = isize;
pub type CFTypeID = usize;

#[repr(C)]
pub struct CFDictionaryKeyCallBacks {
    version: CFIndex,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
    hash: *const c_void,
}

#[repr(C)]
pub struct CFDictionaryValueCallBacks {
    version: CFIndex,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
}

pub const kCFNumberSInt32Type: CFIndex = 3;
pub const kCFStringEncodingUTF8: u32 = 0x0800_0100;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFTypeDictionaryKeyCallBacks: CFDictionaryKeyCallBacks;
    pub static kCFTypeDictionaryValueCallBacks: CFDictionaryValueCallBacks;

    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    pub fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    pub fn CFNumberGetTypeID() -> CFTypeID;
    pub fn CFStringGetTypeID() -> CFTypeID;
    pub fn CFDictionaryCreateMutable(
        alloc: CFAllocatorRef,
        capacity: CFIndex,
        key_cb: *const CFDictionaryKeyCallBacks,
        value_cb: *const CFDictionaryValueCallBacks,
    ) -> CFMutableDictionaryRef;
    pub fn CFDictionarySetValue(d: CFMutableDictionaryRef, key: *const c_void, value: *const c_void);
    pub fn CFNumberCreate(alloc: CFAllocatorRef, the_type: CFIndex, value: *const c_void) -> CFNumberRef;
    pub fn CFNumberGetValue(n: CFNumberRef, the_type: CFIndex, value: *mut c_void) -> u8;
    pub fn CFStringCreateWithCString(alloc: CFAllocatorRef, s: *const c_char, enc: u32) -> CFStringRef;
    pub fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: CFIndex, enc: u32) -> u8;
    pub fn CFSetGetCount(s: CFSetRef) -> CFIndex;
    pub fn CFSetGetValues(s: CFSetRef, values: *mut *const c_void);
}

/// Owned CFString created from a Rust string.
pub struct CfStr(pub CFStringRef);

impl CfStr {
    pub fn new(s: &str) -> CfStr {
        let c = CString::new(s).expect("no NUL in key");
        CfStr(unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), kCFStringEncodingUTF8) })
    }
}

impl Drop for CfStr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

pub fn number_i32(v: i32) -> CFNumberRef {
    unsafe { CFNumberCreate(std::ptr::null(), kCFNumberSInt32Type, &v as *const i32 as *const c_void) }
}

/// Reads a CFNumber as i32. Returns None for null or non-number values.
pub fn to_i32(v: CFTypeRef) -> Option<i32> {
    if v.is_null() || unsafe { CFGetTypeID(v) != CFNumberGetTypeID() } {
        return None;
    }
    let mut out: i32 = 0;
    let ok = unsafe { CFNumberGetValue(v, kCFNumberSInt32Type, &mut out as *mut i32 as *mut c_void) };
    (ok != 0).then_some(out)
}

/// Copies a CFString into a Rust String. Does not release `s`.
pub fn to_string(s: CFStringRef) -> Option<String> {
    if s.is_null() || unsafe { CFGetTypeID(s) != CFStringGetTypeID() } {
        return None;
    }
    let mut buf = vec![0 as c_char; 512];
    let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as CFIndex, kCFStringEncodingUTF8) };
    if ok == 0 {
        return None;
    }
    let bytes: Vec<u8> = buf.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8(bytes).ok()
}

//! Kata OS StorageManager component support.

// Code here binds the camkes component to the rust code.
#![no_std]

extern crate alloc;
use core::slice;
use cstr_core::CStr;
use kata_os_common::allocator;
use kata_os_common::logger::KataLogger;
use kata_storage_interface::KeyValueData;
use kata_storage_interface::StorageManagerError;
use kata_storage_interface::StorageManagerInterface;
use kata_storage_manager::KATA_STORAGE;
use log::trace;

#[no_mangle]
pub extern "C" fn pre_init() {
    static KATA_LOGGER: KataLogger = KataLogger;
    log::set_logger(&KATA_LOGGER).unwrap();
    // NB: set to max; the LoggerInterface will filter
    log::set_max_level(log::LevelFilter::Trace);

    // TODO(sleffler): temp until we integrate with seL4
    static mut HEAP_MEMORY: [u8; 8 * 1024] = [0; 8 * 1024];
    unsafe {
        allocator::ALLOCATOR.init(HEAP_MEMORY.as_mut_ptr() as usize, HEAP_MEMORY.len());
        trace!(
            "setup heap: start_addr {:p} size {}",
            HEAP_MEMORY.as_ptr(),
            HEAP_MEMORY.len()
        );
    }
}

// StorageInterface glue stubs.
#[no_mangle]
pub extern "C" fn storage_read(
    c_key: *const cstr_core::c_char,
    c_raw_value: *mut KeyValueData,
) -> StorageManagerError {
    unsafe {
        match CStr::from_ptr(c_key).to_str() {
            Ok(key) => {
                // TODO(sleffler): de-badge reply cap to get bundle_id
                match KATA_STORAGE.read("fubar", key) {
                    Ok(value) => {
                        // NB: no serialization, returns raw data
                        (*c_raw_value).copy_from_slice(&value);
                        StorageManagerError::SmeSuccess
                    }
                    Err(e) => StorageManagerError::from(e),
                }
            }
            Err(_) => StorageManagerError::SmeKeyInvalid,
        }
    }
}

#[no_mangle]
pub extern "C" fn storage_write(
    c_key: *const cstr_core::c_char,
    c_raw_value_len: usize,
    c_raw_value: *const u8,
) -> StorageManagerError {
    match unsafe { CStr::from_ptr(c_key).to_str() } {
        Ok(key) => {
            // TODO(sleffler): de-badge reply cap to get bundle_id
            unsafe {
                KATA_STORAGE.write(
                    "fubar",
                    key,
                    slice::from_raw_parts(c_raw_value, c_raw_value_len),
                )
            }
            .into()
        }
        Err(_) => StorageManagerError::SmeKeyInvalid,
    }
}

#[no_mangle]
pub extern "C" fn storage_delete(c_key: *const cstr_core::c_char) -> StorageManagerError {
    match unsafe { CStr::from_ptr(c_key).to_str() } {
        Ok(key) => {
            // TODO(sleffler): de-badge reply cap to get bundle_id
            unsafe { KATA_STORAGE.delete("fubar", key) }.into()
        }
        Err(_) => StorageManagerError::SmeKeyInvalid,
    }
}

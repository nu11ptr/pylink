//! Python 3.14+ initialization through an opaque, Python-owned configuration.
//! No Python structure layout is reproduced or accessed from Rust.
//! https://docs.python.org/3.14/c-api/init_config.html#pyinitconfig-c-api

use std::ffi::{c_char, c_int, c_void};

#[repr(C)]
pub(crate) struct PyInitConfig {
    _private: [u8; 0],
}

unsafe extern "C" {
    pub(crate) fn Py_IsInitialized() -> c_int;
    pub(crate) fn Py_GetVersion() -> *const c_char;
    pub(crate) fn PyInitConfig_Create() -> *mut PyInitConfig;
    pub(crate) fn PyInitConfig_Free(config: *mut PyInitConfig);
    pub(crate) fn PyInitConfig_SetInt(
        config: *mut PyInitConfig,
        name: *const c_char,
        value: i64,
    ) -> c_int;
    pub(crate) fn PyInitConfig_SetStr(
        config: *mut PyInitConfig,
        name: *const c_char,
        value: *const c_char,
    ) -> c_int;
    pub(crate) fn PyInitConfig_GetError(
        config: *mut PyInitConfig,
        error: *mut *const c_char,
    ) -> c_int;
    pub(crate) fn Py_InitializeFromInitConfig(config: *mut PyInitConfig) -> c_int;
    // The returned PyThreadState pointer is opaque and never dereferenced here.
    pub(crate) fn PyEval_SaveThread() -> *mut c_void;
}

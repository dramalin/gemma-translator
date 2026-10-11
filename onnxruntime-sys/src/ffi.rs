//! The few `OrtApi` entries this crate calls. `OrtApi` is a table of function
//! pointers that only ever grows at the end, so an entry's slot is fixed:
//! the slots below are `offsetof(struct OrtApi, Name) / sizeof(void *)` in
//! `onnxruntime_c_api.h` v1.23.2, and the same in v1.17.0. Re-check them
//! against the header on a version bump.

use std::os::raw::{c_char, c_int, c_void};

/// The oldest API version that has every slot below; a newer library hands
/// out the same table.
pub const ORT_API_VERSION: u32 = 17;

pub type Status = *mut c_void;

#[repr(C)]
pub struct OrtApiBase {
    pub get_api: unsafe extern "C" fn(version: u32) -> *const *const c_void,
    pub get_version_string: unsafe extern "C" fn() -> *const c_char,
}

pub type GetApiBaseFn = unsafe extern "C" fn() -> *const OrtApiBase;

pub const GET_ERROR_MESSAGE: usize = 2;
pub type GetErrorMessageFn = unsafe extern "C" fn(status: Status) -> *const c_char;

pub const CREATE_ENV: usize = 3;
pub type CreateEnvFn =
    unsafe extern "C" fn(log_level: c_int, log_id: *const c_char, out: *mut *mut c_void) -> Status;

/// `model_path` is `char *` everywhere but Windows.
pub const CREATE_SESSION: usize = 7;
pub type CreateSessionFn = unsafe extern "C" fn(
    env: *const c_void,
    model_path: *const c_char,
    options: *const c_void,
    out: *mut *mut c_void,
) -> Status;

pub const RUN: usize = 9;
pub type RunFn = unsafe extern "C" fn(
    session: *mut c_void,
    run_options: *const c_void,
    input_names: *const *const c_char,
    inputs: *const *const c_void,
    input_count: usize,
    output_names: *const *const c_char,
    output_count: usize,
    outputs: *mut *mut c_void,
) -> Status;

pub const CREATE_SESSION_OPTIONS: usize = 10;
pub type CreateSessionOptionsFn = unsafe extern "C" fn(out: *mut *mut c_void) -> Status;

pub const SET_SESSION_GRAPH_OPTIMIZATION_LEVEL: usize = 23;
pub const SET_INTRA_OP_NUM_THREADS: usize = 24;
pub const SET_INTER_OP_NUM_THREADS: usize = 25;
/// The three setters above share this shape.
pub type SetSessionOptionFn = unsafe extern "C" fn(options: *mut c_void, value: c_int) -> Status;

pub const CREATE_TENSOR_WITH_DATA_AS_ORT_VALUE: usize = 49;
pub type CreateTensorWithDataFn = unsafe extern "C" fn(
    memory_info: *const c_void,
    data: *mut c_void,
    data_bytes: usize,
    shape: *const i64,
    shape_len: usize,
    element_type: c_int,
    out: *mut *mut c_void,
) -> Status;

pub const GET_TENSOR_MUTABLE_DATA: usize = 51;
pub type GetTensorMutableDataFn =
    unsafe extern "C" fn(value: *mut c_void, out: *mut *mut c_void) -> Status;

pub const GET_TENSOR_ELEMENT_TYPE: usize = 60;
pub type GetTensorElementTypeFn =
    unsafe extern "C" fn(info: *const c_void, out: *mut c_int) -> Status;

pub const GET_TENSOR_SHAPE_ELEMENT_COUNT: usize = 64;
pub type GetTensorShapeElementCountFn =
    unsafe extern "C" fn(info: *const c_void, out: *mut usize) -> Status;

pub const GET_TENSOR_TYPE_AND_SHAPE: usize = 65;
pub type GetTensorTypeAndShapeFn =
    unsafe extern "C" fn(value: *const c_void, out: *mut *mut c_void) -> Status;

pub const CREATE_CPU_MEMORY_INFO: usize = 69;
pub type CreateCpuMemoryInfoFn =
    unsafe extern "C" fn(allocator: c_int, memory_type: c_int, out: *mut *mut c_void) -> Status;

pub const RELEASE_STATUS: usize = 93;
pub const RELEASE_MEMORY_INFO: usize = 94;
pub const RELEASE_SESSION: usize = 95;
pub const RELEASE_VALUE: usize = 96;
pub const RELEASE_TENSOR_TYPE_AND_SHAPE_INFO: usize = 99;
pub const RELEASE_SESSION_OPTIONS: usize = 100;
/// Every `Release*` entry has this shape.
pub type ReleaseFn = unsafe extern "C" fn(object: *mut c_void);

pub const LOGGING_LEVEL_WARNING: c_int = 2;
pub const GRAPH_OPTIMIZATION_ENABLE_ALL: c_int = 99;
pub const ELEMENT_TYPE_FLOAT: c_int = 1;
pub const ELEMENT_TYPE_INT64: c_int = 7;
pub const ARENA_ALLOCATOR: c_int = 1;
pub const MEMORY_TYPE_DEFAULT: c_int = 0;

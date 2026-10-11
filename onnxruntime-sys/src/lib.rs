//! Run-time bindings to the ONNX Runtime library that ships with the
//! moonshine-voice wheel (`libonnxruntime.1.23.2.dylib` on macOS,
//! `libonnxruntime-<hash>.so.1` on Linux), for running small CPU models
//! such as the Silero voice activity detector. Only what that needs: one
//! session per model, float and int64 inputs, float outputs.

mod ffi;

use std::ffi::{CStr, CString};
use std::mem::ManuallyDrop;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::rc::Rc;

#[derive(Debug, thiserror::Error)]
pub enum OrtError {
    #[error("no ONNX Runtime library (libonnxruntime*) in {0}")]
    NotFound(PathBuf),
    #[error("cannot load {path}: {message}")]
    Load { path: PathBuf, message: String },
    #[error("{0}")]
    Api(String),
    #[error("{function} failed: {message}")]
    Call {
        function: &'static str,
        message: String,
    },
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

/// Whether `name` is the ONNX Runtime library on this platform.
fn is_library_file(name: &str) -> bool {
    name.starts_with("libonnxruntime")
        && if cfg!(target_os = "macos") {
            name.ends_with(".dylib")
        } else {
            name.contains(".so")
        }
}

/// The loaded library, its function table and the process's one ORT
/// environment. Neither the library nor the environment is ever released:
/// libmoonshine uses the same library, and `CreateEnv` hands every caller
/// the same environment, so releasing ours would pull it out from under the
/// speech recognizer. Not `Send`; it lives on the thread that loaded it.
pub struct OrtLibrary {
    api: *const *const c_void,
    env: *mut c_void,
    version: String,
    _library: ManuallyDrop<libloading::Library>,
}

impl OrtLibrary {
    /// Loads the library from `directory` (the native library directory).
    pub fn load(directory: &Path) -> Result<Rc<OrtLibrary>, OrtError> {
        let mut names: Vec<String> = std::fs::read_dir(directory)
            .map_err(|error| OrtError::Load {
                path: directory.to_path_buf(),
                message: error.to_string(),
            })?
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| is_library_file(name))
            .collect();
        names.sort();
        let path = directory.join(
            names
                .first()
                .ok_or_else(|| OrtError::NotFound(directory.to_path_buf()))?,
        );
        let load_error = |message: String| OrtError::Load {
            path: path.clone(),
            message,
        };
        // SAFETY: loading runs ONNX Runtime's initialisers, which have no
        // preconditions; libmoonshine loads the same library.
        let library = unsafe { libloading::Library::new(&path) }
            .map_err(|error| load_error(error.to_string()))?;
        // SAFETY: `OrtGetApiBase` has this signature in every ORT version and
        // returns a pointer to a static struct.
        let base = unsafe {
            let get_base: ffi::GetApiBaseFn = *library
                .get::<ffi::GetApiBaseFn>(b"OrtGetApiBase\0")
                .map_err(|error| load_error(error.to_string()))?;
            &*get_base()
        };
        // SAFETY: both return static data; the version string is NUL-terminated.
        let (api, version) = unsafe {
            let version = CStr::from_ptr((base.get_version_string)())
                .to_string_lossy()
                .into_owned();
            ((base.get_api)(ffi::ORT_API_VERSION), version)
        };
        if api.is_null() {
            return Err(load_error(format!(
                "version {version} does not offer API version {}",
                ffi::ORT_API_VERSION
            )));
        }
        let mut library = OrtLibrary {
            api,
            env: ptr::null_mut(),
            version,
            _library: ManuallyDrop::new(library),
        };
        let mut env = ptr::null_mut();
        // SAFETY: slot and signature from ffi.rs; the log id is a valid C string.
        unsafe {
            let create_env: ffi::CreateEnvFn = library.entry(ffi::CREATE_ENV);
            library.check(
                "CreateEnv",
                create_env(ffi::LOGGING_LEVEL_WARNING, c"translator".as_ptr(), &mut env),
            )?;
        }
        library.env = env;
        Ok(Rc::new(library))
    }

    /// The library's version string, such as "1.23.2".
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The function in `slot` of the `OrtApi` table.
    ///
    /// # Safety
    /// `T` must be the exact signature of that slot (see ffi.rs).
    unsafe fn entry<T: Copy>(&self, slot: usize) -> T {
        std::mem::transmute_copy(&*self.api.add(slot))
    }

    /// Turns a returned `OrtStatus` into a result, releasing it.
    fn check(&self, function: &'static str, status: ffi::Status) -> Result<(), OrtError> {
        if status.is_null() {
            return Ok(());
        }
        // SAFETY: a non-NULL status is owned by the caller until released;
        // its message is NUL-terminated and valid until then.
        let message = unsafe {
            let get_message: ffi::GetErrorMessageFn = self.entry(ffi::GET_ERROR_MESSAGE);
            let message = CStr::from_ptr(get_message(status))
                .to_string_lossy()
                .into_owned();
            self.release(ffi::RELEASE_STATUS, status);
            message
        };
        Err(OrtError::Call { function, message })
    }

    /// # Safety
    /// `object` must be an owned, live object of the kind `slot` releases.
    unsafe fn release(&self, slot: usize, object: *mut c_void) {
        let release: ffi::ReleaseFn = self.entry(slot);
        release(object);
    }
}

/// One input tensor: data in row-major order and its shape. An empty shape
/// is a scalar.
pub enum Input<'a> {
    F32 { data: &'a [f32], shape: &'a [i64] },
    I64 { data: &'a [i64], shape: &'a [i64] },
}

/// An `OrtValue` released on drop.
struct Value<'a> {
    library: &'a OrtLibrary,
    pointer: *mut c_void,
}

impl Drop for Value<'_> {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            // SAFETY: the value was created by ORT and is released once.
            unsafe { self.library.release(ffi::RELEASE_VALUE, self.pointer) }
        }
    }
}

/// A model loaded for single-threaded CPU inference. Not `Send`.
pub struct Session {
    library: Rc<OrtLibrary>,
    session: *mut c_void,
    memory_info: *mut c_void,
}

impl Session {
    /// Loads the ONNX model at `model` with one intra- and one inter-op thread.
    pub fn load(library: &Rc<OrtLibrary>, model: &Path) -> Result<Session, OrtError> {
        let path = model.to_str().ok_or_else(|| {
            OrtError::InvalidArgument(format!("{} is not UTF-8", model.display()))
        })?;
        let path = CString::new(path)
            .map_err(|_| OrtError::InvalidArgument(format!("{path:?} contains a NUL byte")))?;
        let mut options = ptr::null_mut();
        let mut session = ptr::null_mut();
        let mut memory_info = ptr::null_mut();
        // SAFETY: slots and signatures from ffi.rs; every pointer passed is
        // valid for the call, and the options are released once.
        unsafe {
            let create_options: ffi::CreateSessionOptionsFn =
                library.entry(ffi::CREATE_SESSION_OPTIONS);
            library.check("CreateSessionOptions", create_options(&mut options))?;
            let mut configure = || -> Result<(), OrtError> {
                for (function, slot, value) in [
                    ("SetIntraOpNumThreads", ffi::SET_INTRA_OP_NUM_THREADS, 1),
                    ("SetInterOpNumThreads", ffi::SET_INTER_OP_NUM_THREADS, 1),
                    (
                        "SetSessionGraphOptimizationLevel",
                        ffi::SET_SESSION_GRAPH_OPTIMIZATION_LEVEL,
                        ffi::GRAPH_OPTIMIZATION_ENABLE_ALL,
                    ),
                ] {
                    let set: ffi::SetSessionOptionFn = library.entry(slot);
                    library.check(function, set(options, value))?;
                }
                let create_session: ffi::CreateSessionFn = library.entry(ffi::CREATE_SESSION);
                library.check(
                    "CreateSession",
                    create_session(library.env, path.as_ptr(), options, &mut session),
                )
            };
            let created = configure();
            library.release(ffi::RELEASE_SESSION_OPTIONS, options);
            created?;
            let create_memory_info: ffi::CreateCpuMemoryInfoFn =
                library.entry(ffi::CREATE_CPU_MEMORY_INFO);
            if let Err(error) = library.check(
                "CreateCpuMemoryInfo",
                create_memory_info(
                    ffi::ARENA_ALLOCATOR,
                    ffi::MEMORY_TYPE_DEFAULT,
                    &mut memory_info,
                ),
            ) {
                library.release(ffi::RELEASE_SESSION, session);
                return Err(error);
            }
        }
        Ok(Session {
            library: Rc::clone(library),
            session,
            memory_info,
        })
    }

    /// Runs the model and returns each of `outputs` as floats.
    pub fn run(
        &mut self,
        inputs: &[(&CStr, Input<'_>)],
        outputs: &[&CStr],
    ) -> Result<Vec<Vec<f32>>, OrtError> {
        let library = &*self.library;
        let mut input_values = Vec::with_capacity(inputs.len());
        for (name, input) in inputs {
            let (data, bytes, shape, element_type, count) = match input {
                Input::F32 { data, shape } => (
                    data.as_ptr().cast::<c_void>(),
                    std::mem::size_of_val(*data),
                    *shape,
                    ffi::ELEMENT_TYPE_FLOAT,
                    data.len(),
                ),
                Input::I64 { data, shape } => (
                    data.as_ptr().cast::<c_void>(),
                    std::mem::size_of_val(*data),
                    *shape,
                    ffi::ELEMENT_TYPE_INT64,
                    data.len(),
                ),
            };
            if shape.iter().product::<i64>() != count as i64 {
                return Err(OrtError::InvalidArgument(format!(
                    "{name:?}: shape {shape:?} does not hold {count} elements"
                )));
            }
            let mut value = Value {
                library,
                pointer: ptr::null_mut(),
            };
            // SAFETY: slot and signature from ffi.rs. ORT only reads an input
            // tensor, so handing it a pointer to shared data is sound; the
            // data outlives the value, which is released before returning.
            unsafe {
                let create: ffi::CreateTensorWithDataFn =
                    library.entry(ffi::CREATE_TENSOR_WITH_DATA_AS_ORT_VALUE);
                library.check(
                    "CreateTensorWithDataAsOrtValue",
                    create(
                        self.memory_info,
                        data.cast_mut(),
                        bytes,
                        shape.as_ptr(),
                        shape.len(),
                        element_type,
                        &mut value.pointer,
                    ),
                )?;
            }
            input_values.push(value);
        }
        let input_names: Vec<*const c_char> =
            inputs.iter().map(|(name, _)| name.as_ptr()).collect();
        let input_pointers: Vec<*const c_void> = input_values
            .iter()
            .map(|value| value.pointer.cast_const())
            .collect();
        let output_names: Vec<*const c_char> = outputs.iter().map(|name| name.as_ptr()).collect();
        let mut output_pointers = vec![ptr::null_mut(); outputs.len()];
        // SAFETY: slot and signature from ffi.rs; the name and value arrays
        // have the counts passed, and ORT fills one owned value per output.
        let status = unsafe {
            let run: ffi::RunFn = library.entry(ffi::RUN);
            run(
                self.session,
                ptr::null(),
                input_names.as_ptr(),
                input_pointers.as_ptr(),
                inputs.len(),
                output_names.as_ptr(),
                outputs.len(),
                output_pointers.as_mut_ptr(),
            )
        };
        let output_values: Vec<Value> = output_pointers
            .into_iter()
            .map(|pointer| Value { library, pointer })
            .collect();
        library.check("Run", status)?;
        output_values
            .iter()
            .zip(outputs)
            .map(|(value, name)| self.read_floats(value, name))
            .collect()
    }

    fn read_floats(&self, value: &Value, name: &CStr) -> Result<Vec<f32>, OrtError> {
        let library = &*self.library;
        let mut info = ptr::null_mut();
        let mut element_type: c_int = 0;
        let mut count = 0usize;
        let mut data = ptr::null_mut();
        // SAFETY: slots and signatures from ffi.rs; `value` is a live tensor,
        // `info` is released once, and `data` points to `count` elements of
        // `element_type` while `value` lives.
        unsafe {
            let get_info: ffi::GetTensorTypeAndShapeFn =
                library.entry(ffi::GET_TENSOR_TYPE_AND_SHAPE);
            library.check("GetTensorTypeAndShape", get_info(value.pointer, &mut info))?;
            let get_type: ffi::GetTensorElementTypeFn = library.entry(ffi::GET_TENSOR_ELEMENT_TYPE);
            let get_count: ffi::GetTensorShapeElementCountFn =
                library.entry(ffi::GET_TENSOR_SHAPE_ELEMENT_COUNT);
            let described = library
                .check("GetTensorElementType", get_type(info, &mut element_type))
                .and_then(|()| {
                    library.check("GetTensorShapeElementCount", get_count(info, &mut count))
                });
            library.release(ffi::RELEASE_TENSOR_TYPE_AND_SHAPE_INFO, info);
            described?;
            if element_type != ffi::ELEMENT_TYPE_FLOAT {
                return Err(OrtError::Api(format!(
                    "output {name:?} has element type {element_type}, not float"
                )));
            }
            let get_data: ffi::GetTensorMutableDataFn = library.entry(ffi::GET_TENSOR_MUTABLE_DATA);
            library.check("GetTensorMutableData", get_data(value.pointer, &mut data))?;
            if count == 0 {
                return Ok(Vec::new());
            }
            Ok(std::slice::from_raw_parts(data.cast::<f32>(), count).to_vec())
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: both were created in `load` and are released once.
        unsafe {
            self.library
                .release(ffi::RELEASE_MEMORY_INFO, self.memory_info);
            self.library.release(ffi::RELEASE_SESSION, self.session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_file_name_matches_the_platform() {
        if cfg!(target_os = "macos") {
            assert!(is_library_file("libonnxruntime.1.23.2.dylib"));
            assert!(!is_library_file("libonnxruntime-1a2b3c4d.so.1"));
        } else {
            assert!(is_library_file("libonnxruntime-1a2b3c4d.so.1"));
            assert!(!is_library_file("libonnxruntime.1.23.2.dylib"));
        }
        assert!(!is_library_file("libmoonshine.so"));
        assert!(!is_library_file("libmoonshine.dylib"));
    }

    #[test]
    fn a_directory_without_the_library_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("libmoonshine.so"), b"").unwrap();
        let error = OrtLibrary::load(directory.path()).err().unwrap();
        assert!(matches!(error, OrtError::NotFound(_)), "{error}");
    }
}

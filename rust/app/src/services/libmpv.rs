//! libmpv, loaded at runtime instead of linked.
//!
//! Linking libmpv makes the dynamic loader map and relocate its whole
//! dependency tree (ffmpeg, libplacebo, codecs: about 200 libraries) before
//! `main` runs. Measured on this machine: 71 ms and 86 MB resident on every
//! launch, paid even by a session that only browses. Loading it with `dlopen`
//! the first time playback is asked for moves that cost off startup, onto a
//! background thread, and away from sessions that never play.
//!
//! Only the slice of the client and render API this app uses is declared
//! here; the layouts follow mpv's `client.h`, `render.h` and `render_gl.h`
//! (client API 2.x).

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::{self, NonNull};
use std::sync::OnceLock;

use libloading::Library;

#[repr(C)]
pub struct mpv_handle {
    _opaque: [u8; 0],
}

#[repr(C)]
pub struct mpv_render_context {
    _opaque: [u8; 0],
}

#[repr(C)]
struct mpv_event {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct mpv_event_property {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct mpv_event_end_file {
    reason: c_int,
    error: c_int,
}

#[repr(C)]
pub struct mpv_render_param {
    pub kind: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_opengl_init_params {
    pub get_proc_address:
        Option<unsafe extern "C" fn(context: *mut c_void, name: *const c_char) -> *mut c_void>,
    pub get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
pub struct mpv_opengl_fbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

#[repr(C)]
pub struct mpv_render_frame_info {
    pub flags: u64,
    pub target_time: i64,
}

pub type RenderUpdateFn = unsafe extern "C" fn(context: *mut c_void);

const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;

const EVENT_NONE: c_int = 0;
const EVENT_SHUTDOWN: c_int = 1;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_IDLE: c_int = 11;
const EVENT_PLAYBACK_RESTART: c_int = 21;
const EVENT_PROPERTY_CHANGE: c_int = 22;

pub const END_FILE_REASON_EOF: i32 = 0;
pub const END_FILE_REASON_ERROR: i32 = 4;

pub const RENDER_PARAM_INVALID: c_int = 0;
pub const RENDER_PARAM_API_TYPE: c_int = 1;
pub const RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
pub const RENDER_PARAM_OPENGL_FBO: c_int = 3;
pub const RENDER_PARAM_FLIP_Y: c_int = 4;
pub const RENDER_PARAM_X11_DISPLAY: c_int = 8;
pub const RENDER_PARAM_WL_DISPLAY: c_int = 9;
pub const RENDER_PARAM_ADVANCED_CONTROL: c_int = 10;
pub const RENDER_PARAM_NEXT_FRAME_INFO: c_int = 11;
pub const RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
pub const RENDER_API_TYPE_OPENGL: &CStr = c"opengl";
pub const RENDER_UPDATE_FRAME: u64 = 1 << 0;
pub const RENDER_FRAME_INFO_REDRAW: u64 = 1 << 1;

/// The client API major version this module's layouts are written for.
const API_MAJOR: u32 = 2;

/// The resolved entry points. Lives for the rest of the process once loaded:
/// mpv's threads and callbacks may run until the very end of teardown.
pub struct Api {
    _library: Library,
    client_api_version: unsafe extern "C" fn() -> std::ffi::c_ulong,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    free: unsafe extern "C" fn(*mut c_void),
    create: unsafe extern "C" fn() -> *mut mpv_handle,
    create_client: unsafe extern "C" fn(*mut mpv_handle, *const c_char) -> *mut mpv_handle,
    initialize: unsafe extern "C" fn(*mut mpv_handle) -> c_int,
    destroy: unsafe extern "C" fn(*mut mpv_handle),
    set_option_string: unsafe extern "C" fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int,
    command: unsafe extern "C" fn(*mut mpv_handle, *mut *const c_char) -> c_int,
    set_property: unsafe extern "C" fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int,
    get_property: unsafe extern "C" fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int,
    observe_property: unsafe extern "C" fn(*mut mpv_handle, u64, *const c_char, c_int) -> c_int,
    request_event: unsafe extern "C" fn(*mut mpv_handle, c_int, c_int) -> c_int,
    wait_event: unsafe extern "C" fn(*mut mpv_handle, f64) -> *mut mpv_event,
    wakeup: unsafe extern "C" fn(*mut mpv_handle),
    get_time_ns: unsafe extern "C" fn(*mut mpv_handle) -> i64,
    pub render_context_create: unsafe extern "C" fn(
        *mut *mut mpv_render_context,
        *mut mpv_handle,
        *mut mpv_render_param,
    ) -> c_int,
    pub render_context_set_update_callback:
        unsafe extern "C" fn(*mut mpv_render_context, Option<RenderUpdateFn>, *mut c_void),
    pub render_context_update: unsafe extern "C" fn(*mut mpv_render_context) -> u64,
    pub render_context_render:
        unsafe extern "C" fn(*mut mpv_render_context, *mut mpv_render_param) -> c_int,
    pub render_context_report_swap: unsafe extern "C" fn(*mut mpv_render_context),
    pub render_context_get_info:
        unsafe extern "C" fn(*mut mpv_render_context, mpv_render_param) -> c_int,
    pub render_context_free: unsafe extern "C" fn(*mut mpv_render_context),
}

static API: OnceLock<Result<Api, String>> = OnceLock::new();

/// The loaded library. The first call loads it (~70 ms, blocking), so it
/// belongs on a background thread; later calls are a single atomic load.
pub fn api() -> Result<&'static Api, Error> {
    API.get_or_init(load)
        .as_ref()
        .map_err(|message| Error::Load(message.clone()))
}

/// The library if it has been loaded already, without loading it.
pub fn loaded() -> Option<&'static Api> {
    API.get().and_then(|api| api.as_ref().ok())
}

#[cfg(target_os = "linux")]
const CANDIDATES: &[&str] = &[
    // The packaged layout, mirroring the rpath in build.rs.
    "$ORIGIN/../lib/jellybridge/lib/libmpv.so.2",
    "libmpv.so.2",
];
#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &[
    // The self-contained bundle (scripts/package-rust-app-personal.sh).
    "$ORIGIN/../Frameworks/libmpv.2.dylib",
    "/opt/homebrew/lib/libmpv.2.dylib",
    "/usr/local/lib/libmpv.2.dylib",
];
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const CANDIDATES: &[&str] = &["libmpv-2.dll", "mpv-2.dll"];

fn load() -> Result<Api, String> {
    let started = std::time::Instant::now();
    let origin = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let mut errors = Vec::new();
    let library = CANDIDATES.iter().find_map(|candidate| {
        let path = candidate.replace("$ORIGIN", &origin);
        if candidate.starts_with("$ORIGIN") && !std::path::Path::new(&path).exists() {
            return None;
        }
        // SAFETY: loading libmpv runs its library constructors, which are
        // ordinary C initialisers with no requirements on the caller.
        match unsafe { Library::new(&path) } {
            Ok(library) => Some(library),
            Err(error) => {
                errors.push(format!("{path}: {error}"));
                None
            }
        }
    });
    let Some(library) = library else {
        return Err(format!("libmpv was not found ({})", errors.join("; ")));
    };
    let api =
        unsafe { resolve(library) }.map_err(|error| format!("libmpv is incomplete: {error}"))?;
    // `c_ulong` is 64-bit here but 32-bit on Windows.
    #[allow(clippy::useless_conversion)]
    let version = u64::from(unsafe { (api.client_api_version)() });
    let major = (version >> 16) as u32;
    if major != API_MAJOR {
        return Err(format!(
            "libmpv client API {major}.{} is not supported (needs {API_MAJOR}.x)",
            version & 0xffff
        ));
    }
    tracing::info!(
        ms = started.elapsed().as_secs_f64() * 1000.0,
        api = format!("{major}.{}", version & 0xffff),
        "libmpv loaded"
    );
    Ok(api)
}

/// SAFETY: every symbol is declared with the signature mpv's headers give it.
unsafe fn resolve(library: Library) -> Result<Api, libloading::Error> {
    macro_rules! symbol {
        ($name:literal) => {
            *unsafe { library.get(concat!($name, "\0").as_bytes()) }?
        };
    }
    Ok(Api {
        client_api_version: symbol!("mpv_client_api_version"),
        error_string: symbol!("mpv_error_string"),
        free: symbol!("mpv_free"),
        create: symbol!("mpv_create"),
        create_client: symbol!("mpv_create_client"),
        initialize: symbol!("mpv_initialize"),
        destroy: symbol!("mpv_destroy"),
        set_option_string: symbol!("mpv_set_option_string"),
        command: symbol!("mpv_command"),
        set_property: symbol!("mpv_set_property"),
        get_property: symbol!("mpv_get_property"),
        observe_property: symbol!("mpv_observe_property"),
        request_event: symbol!("mpv_request_event"),
        wait_event: symbol!("mpv_wait_event"),
        wakeup: symbol!("mpv_wakeup"),
        get_time_ns: symbol!("mpv_get_time_ns"),
        render_context_create: symbol!("mpv_render_context_create"),
        render_context_set_update_callback: symbol!("mpv_render_context_set_update_callback"),
        render_context_update: symbol!("mpv_render_context_update"),
        render_context_render: symbol!("mpv_render_context_render"),
        render_context_report_swap: symbol!("mpv_render_context_report_swap"),
        render_context_get_info: symbol!("mpv_render_context_get_info"),
        render_context_free: symbol!("mpv_render_context_free"),
        _library: library,
    })
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Load(String),
    #[error("{message} ({code})")]
    Mpv { code: i32, message: String },
    #[error("argument contains a NUL byte")]
    Nul,
}

impl From<std::ffi::NulError> for Error {
    fn from(_: std::ffi::NulError) -> Self {
        Self::Nul
    }
}

impl Api {
    pub fn check(&self, status: c_int) -> Result<(), Error> {
        if status >= 0 {
            return Ok(());
        }
        let message = unsafe { CStr::from_ptr((self.error_string)(status)) }
            .to_string_lossy()
            .into_owned();
        Err(Error::Mpv {
            code: status,
            message,
        })
    }

    /// Interrupts a `wait_event` on `handle`. Callable from any thread.
    ///
    /// # Safety
    /// `handle` must be alive.
    pub unsafe fn wakeup(&self, handle: *mut mpv_handle) {
        unsafe { (self.wakeup)(handle) }
    }

    /// mpv's clock, the base of render frame target times.
    ///
    /// # Safety
    /// `handle` must be alive.
    pub unsafe fn time_ns(&self, handle: *mut mpv_handle) -> i64 {
        unsafe { (self.get_time_ns)(handle) }
    }

    /// # Safety
    /// `handle` must be alive and not used again afterwards.
    pub unsafe fn destroy(&self, handle: *mut mpv_handle) {
        unsafe { (self.destroy)(handle) }
    }
}

/// A value read from or written to an mpv property.
pub enum Value<'a> {
    Flag(bool),
    Int(i64),
    Double(f64),
    Str(&'a str),
}

pub enum Event<'a> {
    Shutdown,
    FileLoaded,
    /// `MPV_END_FILE_REASON_*`.
    EndFile(i32),
    PlaybackRestart,
    PropertyChange {
        name: &'a str,
        value: Option<Value<'a>>,
    },
    Other,
}

/// Owner of the primary client handle. Not `Clone`; dropping it destroys the
/// handle (the core lives on until every other handle is gone too).
pub struct Mpv {
    api: &'static Api,
    handle: NonNull<mpv_handle>,
}

// SAFETY: libmpv's client API is thread-safe; the handle is used from one
// thread at a time by construction (it moves into the player worker).
unsafe impl Send for Mpv {}

impl Mpv {
    /// Loads libmpv if needed and creates an uninitialized core.
    pub fn create() -> Result<Self, Error> {
        let api = api()?;
        let handle = NonNull::new(unsafe { (api.create)() }).ok_or_else(|| Error::Mpv {
            code: -1,
            message: "mpv_create returned NULL".into(),
        })?;
        Ok(Self { api, handle })
    }

    pub fn handle(&self) -> *mut mpv_handle {
        self.handle.as_ptr()
    }

    pub fn set_option(&self, name: &str, value: &str) -> Result<(), Error> {
        let name = CString::new(name)?;
        let value = CString::new(value)?;
        self.api.check(unsafe {
            (self.api.set_option_string)(self.handle(), name.as_ptr(), value.as_ptr())
        })
    }

    pub fn initialize(&self) -> Result<(), Error> {
        self.api
            .check(unsafe { (self.api.initialize)(self.handle()) })
    }

    /// A second client of the same core (for the render context).
    pub fn create_client(&self, name: &CStr) -> Option<NonNull<mpv_handle>> {
        NonNull::new(unsafe { (self.api.create_client)(self.handle(), name.as_ptr()) })
    }

    pub fn command(&self, args: &[&str]) -> Result<(), Error> {
        let owned = args
            .iter()
            .map(|arg| CString::new(*arg))
            .collect::<Result<Vec<_>, _>>()?;
        let mut pointers = owned.iter().map(|arg| arg.as_ptr()).collect::<Vec<_>>();
        pointers.push(ptr::null());
        self.api
            .check(unsafe { (self.api.command)(self.handle(), pointers.as_mut_ptr()) })
    }

    pub fn set_property(&self, name: &str, value: Value<'_>) -> Result<(), Error> {
        let name = CString::new(name)?;
        let status = unsafe {
            match value {
                Value::Flag(flag) => {
                    let mut raw = c_int::from(flag);
                    (self.api.set_property)(
                        self.handle(),
                        name.as_ptr(),
                        FORMAT_FLAG,
                        (&mut raw as *mut c_int).cast(),
                    )
                }
                Value::Int(mut raw) => (self.api.set_property)(
                    self.handle(),
                    name.as_ptr(),
                    FORMAT_INT64,
                    (&mut raw as *mut i64).cast(),
                ),
                Value::Double(mut raw) => (self.api.set_property)(
                    self.handle(),
                    name.as_ptr(),
                    FORMAT_DOUBLE,
                    (&mut raw as *mut f64).cast(),
                ),
                Value::Str(text) => {
                    let text = CString::new(text)?;
                    let mut raw = text.as_ptr();
                    (self.api.set_property)(
                        self.handle(),
                        name.as_ptr(),
                        FORMAT_STRING,
                        (&mut raw as *mut *const c_char).cast(),
                    )
                }
            }
        };
        self.api.check(status)
    }

    pub fn get_f64(&self, name: &str) -> Result<f64, Error> {
        let mut value = 0.0_f64;
        self.get_raw(name, FORMAT_DOUBLE, (&mut value as *mut f64).cast())?;
        Ok(value)
    }

    pub fn get_i64(&self, name: &str) -> Result<i64, Error> {
        let mut value = 0_i64;
        self.get_raw(name, FORMAT_INT64, (&mut value as *mut i64).cast())?;
        Ok(value)
    }

    pub fn get_string(&self, name: &str) -> Result<String, Error> {
        let mut value: *mut c_char = ptr::null_mut();
        self.get_raw(name, FORMAT_STRING, (&mut value as *mut *mut c_char).cast())?;
        if value.is_null() {
            return Ok(String::new());
        }
        let text = unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.api.free)(value.cast()) };
        Ok(text)
    }

    fn get_raw(&self, name: &str, format: c_int, data: *mut c_void) -> Result<(), Error> {
        let name = CString::new(name)?;
        self.api
            .check(unsafe { (self.api.get_property)(self.handle(), name.as_ptr(), format, data) })
    }

    pub fn observe_flag(&self, name: &str, id: u64) -> Result<(), Error> {
        self.observe(name, FORMAT_FLAG, id)
    }

    pub fn observe_double(&self, name: &str, id: u64) -> Result<(), Error> {
        self.observe(name, FORMAT_DOUBLE, id)
    }

    pub fn observe_string(&self, name: &str, id: u64) -> Result<(), Error> {
        self.observe(name, FORMAT_STRING, id)
    }

    fn observe(&self, name: &str, format: c_int, id: u64) -> Result<(), Error> {
        let name = CString::new(name)?;
        self.api
            .check(unsafe { (self.api.observe_property)(self.handle(), id, name.as_ptr(), format) })
    }

    /// Turns off `MPV_EVENT_IDLE`, which is deprecated and only ever noise.
    pub fn disable_deprecated_events(&self) -> Result<(), Error> {
        self.api
            .check(unsafe { (self.api.request_event)(self.handle(), EVENT_IDLE, 0) })
    }

    /// The next event, waiting up to `timeout` seconds (negative: forever).
    /// `None` when the wait ended without one (timeout or `wakeup`). The
    /// event borrows mpv's buffer, valid until the next call.
    pub fn wait_event(&self, timeout: f64) -> Option<Result<Event<'_>, Error>> {
        let event = unsafe { &*(self.api.wait_event)(self.handle(), timeout) };
        if event.event_id == EVENT_NONE {
            return None;
        }
        if let Err(error) = self.api.check(event.error) {
            return Some(Err(error));
        }
        Some(Ok(match event.event_id {
            EVENT_SHUTDOWN => Event::Shutdown,
            EVENT_FILE_LOADED => Event::FileLoaded,
            EVENT_PLAYBACK_RESTART => Event::PlaybackRestart,
            EVENT_END_FILE if !event.data.is_null() => {
                let end = unsafe { &*event.data.cast::<mpv_event_end_file>() };
                Event::EndFile(end.reason)
            }
            EVENT_PROPERTY_CHANGE if !event.data.is_null() => {
                let property = unsafe { &*event.data.cast::<mpv_event_property>() };
                let name = unsafe { CStr::from_ptr(property.name) }
                    .to_str()
                    .unwrap_or("");
                Event::PropertyChange {
                    name,
                    value: unsafe { property_value(property) },
                }
            }
            _ => Event::Other,
        }))
    }
}

/// SAFETY: `property` comes from the event mpv just returned.
unsafe fn property_value<'a>(property: &'a mpv_event_property) -> Option<Value<'a>> {
    if property.data.is_null() {
        return None;
    }
    unsafe {
        match property.format {
            FORMAT_FLAG => Some(Value::Flag(*property.data.cast::<c_int>() != 0)),
            FORMAT_INT64 => Some(Value::Int(*property.data.cast::<i64>())),
            FORMAT_DOUBLE => Some(Value::Double(*property.data.cast::<f64>())),
            FORMAT_STRING => {
                let text = *property.data.cast::<*const c_char>();
                (!text.is_null()).then(|| Value::Str(CStr::from_ptr(text).to_str().unwrap_or("")))
            }
            _ => None,
        }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe { (self.api.destroy)(self.handle()) };
    }
}

//! Keeps the computer awake while a video plays: no screen lock, no display
//! blanking and no suspend from idling, as a browser does for a playing
//! video. The hold is released the moment playback pauses, ends or the
//! player is left, so a paused film still lets the machine lock and sleep.
//!
//! Each desktop has its own way to ask, tried in this order:
//!
//! - Wayland: an idle inhibitor on the window's surface
//!   (`zwp_idle_inhibit_manager_v1`), which every compositor and idle daemon
//!   honours (Hyprland and hypridle, Sway, GNOME, KDE) without any daemon
//!   having to be running.
//! - Otherwise on Linux (X11, or a compositor without the protocol): the
//!   `org.freedesktop.ScreenSaver` D-Bus interface (KDE, GNOME's proxy,
//!   hypridle, xfce), then the desktop portal's `Inhibit`.
//! - Windows: `SetThreadExecutionState`, the same request a video player
//!   or presentation makes.
//! - macOS: an IOKit power assertion against display sleep.
//!
//! All of it happens on one small thread of its own, so a slow D-Bus
//! answer never holds the window, and so Windows' request, which is bound
//! to the thread that made it, has a thread that lives as long as the hold.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

const REASON: &str = "Playing a video";

/// The app's single wake lock. Owned by the window; tell it each frame
/// whether a video is playing and it does the rest.
pub struct KeepAwake {
    sender: Option<Sender<bool>>,
    thread: Option<JoinHandle<()>>,
    playing: bool,
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::new()
    }
}

impl KeepAwake {
    pub fn new() -> Self {
        Self {
            sender: None,
            thread: None,
            playing: false,
        }
    }

    /// Whether a video is playing right now. Cheap to call every frame: only
    /// a change does anything. The thread starts on the first play, with the
    /// window's handles for the Wayland path.
    pub fn set_playing(
        &mut self,
        playing: bool,
        window: &(impl HasWindowHandle + HasDisplayHandle),
    ) {
        if playing == self.playing {
            return;
        }
        self.playing = playing;
        if self.sender.is_none() {
            if !playing {
                return;
            }
            let handles = WindowHandles::of(window);
            let (sender, receiver) = mpsc::channel();
            match std::thread::Builder::new()
                .name("jellybridge-keep-awake".to_owned())
                .spawn(move || run(&receiver, &handles))
            {
                Ok(thread) => {
                    self.sender = Some(sender);
                    self.thread = Some(thread);
                }
                Err(error) => {
                    tracing::warn!(%error, "could not start the keep-awake thread");
                    return;
                }
            }
        }
        if let Some(sender) = &self.sender {
            let _ = sender.send(playing);
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        // Closing the channel ends the thread, which releases any hold on
        // its way out; the join makes sure that happened before the window
        // (whose surface the Wayland inhibitor references) goes away.
        self.sender = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(receiver: &Receiver<bool>, handles: &WindowHandles) {
    let mut held: Option<Hold> = None;
    let mut warned = false;
    while let Ok(playing) = receiver.recv() {
        if playing {
            if held.is_none() {
                held = Hold::acquire(handles);
                if held.is_none() && !warned {
                    warned = true;
                    tracing::warn!(
                        "no way to keep the computer awake was found; it may lock or \
                         sleep during playback"
                    );
                }
            }
        } else {
            held = None;
        }
    }
}

/// What the Wayland path needs from the window: its display and surface.
/// Empty on X11 and on the other platforms.
struct WindowHandles {
    #[cfg(target_os = "linux")]
    wayland: Option<wayland::Handles>,
}

impl WindowHandles {
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    fn of(window: &(impl HasWindowHandle + HasDisplayHandle)) -> Self {
        Self {
            #[cfg(target_os = "linux")]
            wayland: wayland::Handles::of(window),
        }
    }
}

/// A hold on the computer staying awake, kept only for its `Drop`, which
/// lets go.
enum Hold {
    #[cfg(target_os = "linux")]
    Wayland { _inhibitor: wayland::Inhibitor },
    #[cfg(target_os = "linux")]
    ScreenSaver { _lock: dbus::ScreenSaver },
    #[cfg(target_os = "linux")]
    Portal { _lock: dbus::Portal },
    #[cfg(windows)]
    Windows { _request: windows::Request },
    #[cfg(target_os = "macos")]
    Mac { _assertion: mac::Assertion },
}

impl Hold {
    #[cfg(target_os = "linux")]
    fn acquire(handles: &WindowHandles) -> Option<Self> {
        if let Some(handles) = &handles.wayland {
            match wayland::Inhibitor::acquire(handles) {
                Ok(inhibitor) => {
                    tracing::debug!(method = "wayland", "keeping the computer awake");
                    return Some(Self::Wayland {
                        _inhibitor: inhibitor,
                    });
                }
                Err(error) => tracing::debug!(%error, "no Wayland idle inhibitor"),
            }
        }
        match dbus::ScreenSaver::acquire() {
            Ok(lock) => {
                tracing::debug!(method = "screensaver", "keeping the computer awake");
                return Some(Self::ScreenSaver { _lock: lock });
            }
            Err(error) => tracing::debug!(%error, "no screensaver inhibit"),
        }
        match dbus::Portal::acquire() {
            Ok(lock) => {
                tracing::debug!(method = "portal", "keeping the computer awake");
                Some(Self::Portal { _lock: lock })
            }
            Err(error) => {
                tracing::debug!(%error, "no portal inhibit");
                None
            }
        }
    }

    #[cfg(windows)]
    fn acquire(_handles: &WindowHandles) -> Option<Self> {
        let request = windows::Request::acquire()?;
        tracing::debug!(method = "execution-state", "keeping the computer awake");
        Some(Self::Windows { _request: request })
    }

    #[cfg(target_os = "macos")]
    fn acquire(_handles: &WindowHandles) -> Option<Self> {
        let assertion = mac::Assertion::acquire()?;
        tracing::debug!(method = "power-assertion", "keeping the computer awake");
        Some(Self::Mac {
            _assertion: assertion,
        })
    }

    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    fn acquire(_handles: &WindowHandles) -> Option<Self> {
        None
    }
}

#[cfg(target_os = "linux")]
mod wayland {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_registry::WlRegistry, wl_surface::WlSurface};
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
    use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
    use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1;

    /// The window's `wl_display` and `wl_surface`, as winit hands them out.
    pub struct Handles {
        display: NonNull<c_void>,
        surface: NonNull<c_void>,
    }

    // SAFETY: libwayland-client is thread-safe for proxies used from their
    // own event queue, which is all the thread does with these; the window
    // that owns both outlives the thread (see `KeepAwake::drop`).
    unsafe impl Send for Handles {}

    impl Handles {
        pub fn of(window: &(impl HasWindowHandle + HasDisplayHandle)) -> Option<Self> {
            let display = match window.display_handle().ok()?.as_raw() {
                RawDisplayHandle::Wayland(handle) => handle.display,
                _ => return None,
            };
            let surface = match window.window_handle().ok()?.as_raw() {
                RawWindowHandle::Wayland(handle) => handle.surface,
                _ => return None,
            };
            Some(Self { display, surface })
        }
    }

    /// Nothing the compositor sends about these objects matters: the
    /// manager and the inhibitor have no events, and the globals were read
    /// once at setup.
    struct Noop;

    impl Dispatch<WlRegistry, GlobalListContents> for Noop {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: <WlRegistry as Proxy>::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    delegate_noop!(Noop: ignore ZwpIdleInhibitManagerV1);
    delegate_noop!(Noop: ignore ZwpIdleInhibitorV1);

    /// A `zwp_idle_inhibitor_v1` on the window's surface, on an event queue
    /// of our own beside winit's.
    pub struct Inhibitor {
        connection: Connection,
        _queue: EventQueue<Noop>,
        manager: ZwpIdleInhibitManagerV1,
        inhibitor: ZwpIdleInhibitorV1,
    }

    impl Inhibitor {
        pub fn acquire(handles: &Handles) -> Result<Self, String> {
            // SAFETY: the pointer is the live `wl_display` of the window's
            // connection; a foreign-display backend never closes it.
            let backend = unsafe { Backend::from_foreign_display(handles.display.as_ptr().cast()) };
            let connection = Connection::from_backend(backend);
            let (globals, queue) =
                registry_queue_init::<Noop>(&connection).map_err(|error| error.to_string())?;
            let manager: ZwpIdleInhibitManagerV1 = globals
                .bind(&queue.handle(), 1..=1, ())
                .map_err(|error| error.to_string())?;
            // SAFETY: the pointer is the window's live `wl_surface` proxy;
            // `from_ptr` checks that it really is one.
            let surface_id = unsafe {
                ObjectId::from_ptr(WlSurface::interface(), handles.surface.as_ptr().cast())
            }
            .map_err(|error| error.to_string())?;
            let surface =
                WlSurface::from_id(&connection, surface_id).map_err(|error| error.to_string())?;
            let inhibitor = manager.create_inhibitor(&surface, &queue.handle(), ());
            connection.flush().map_err(|error| error.to_string())?;
            Ok(Self {
                connection,
                _queue: queue,
                manager,
                inhibitor,
            })
        }
    }

    impl Drop for Inhibitor {
        fn drop(&mut self) {
            self.inhibitor.destroy();
            self.manager.destroy();
            let _ = self.connection.flush();
        }
    }
}

#[cfg(target_os = "linux")]
mod dbus {
    use std::collections::HashMap;

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, Value};

    use super::REASON;

    const PORTAL: &str = "org.freedesktop.portal.Desktop";
    /// The portal's `Inhibit` flag for idling (the others are logout, user
    /// switch and suspend, which a video has no business blocking).
    const PORTAL_IDLE: u32 = 8;

    /// `org.freedesktop.ScreenSaver`'s inhibit cookie. The connection stays
    /// open with it: the inhibit belongs to the connection.
    pub struct ScreenSaver {
        connection: Connection,
        cookie: u32,
    }

    impl ScreenSaver {
        pub fn acquire() -> Result<Self, zbus::Error> {
            let connection = Connection::session()?;
            let cookie: u32 =
                Self::proxy(&connection)?.call("Inhibit", &("JellyBridge", REASON))?;
            Ok(Self { connection, cookie })
        }

        fn proxy(connection: &Connection) -> Result<Proxy<'static>, zbus::Error> {
            Proxy::new(
                connection,
                "org.freedesktop.ScreenSaver",
                "/org/freedesktop/ScreenSaver",
                "org.freedesktop.ScreenSaver",
            )
        }
    }

    impl Drop for ScreenSaver {
        fn drop(&mut self) {
            if let Ok(proxy) = Self::proxy(&self.connection) {
                let _: Result<(), _> = proxy.call("UnInhibit", &(self.cookie,));
            }
        }
    }

    /// The desktop portal's inhibit request, closed to release.
    pub struct Portal {
        connection: Connection,
        request: OwnedObjectPath,
    }

    impl Portal {
        pub fn acquire() -> Result<Self, zbus::Error> {
            let connection = Connection::session()?;
            let proxy = Proxy::new(
                &connection,
                PORTAL,
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.Inhibit",
            )?;
            let mut options: HashMap<&str, Value> = HashMap::new();
            options.insert("reason", Value::from(REASON));
            let request: OwnedObjectPath = proxy.call("Inhibit", &("", PORTAL_IDLE, options))?;
            Ok(Self {
                connection,
                request,
            })
        }
    }

    impl Drop for Portal {
        fn drop(&mut self) {
            if let Ok(proxy) = Proxy::new(
                &self.connection,
                PORTAL,
                self.request.as_str(),
                "org.freedesktop.portal.Request",
            ) {
                let _: Result<(), _> = proxy.call("Close", &());
            }
        }
    }
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

    /// A continuous display-and-system-required request. It is bound to the
    /// thread that made it, which is why the keep-awake thread exists.
    pub struct Request;

    impl Request {
        pub fn acquire() -> Option<Self> {
            // SAFETY: a plain Win32 call with constant flags.
            let previous = unsafe {
                SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED)
            };
            (previous != 0).then_some(Self)
        }
    }

    impl Drop for Request {
        fn drop(&mut self) {
            // SAFETY: as above; `ES_CONTINUOUS` alone clears the requirements.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{CString, c_char, c_void};

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFAllocatorRef = *const c_void;

    const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const ASSERTION_LEVEL_ON: u32 = 255;
    const SUCCESS: i32 = 0;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: CFAllocatorRef,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            assertion_level: u32,
            assertion_name: CFStringRef,
            assertion_id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(assertion_id: u32) -> i32;
    }

    /// A `PreventUserIdleDisplaySleep` assertion, which also keeps the
    /// system from idle sleep; it shows under "Preventing Sleep" in
    /// Activity Monitor with the name given here.
    pub struct Assertion(u32);

    impl Assertion {
        pub fn acquire() -> Option<Self> {
            let kind = CString::new("PreventUserIdleDisplaySleep").ok()?;
            let name = CString::new(format!("JellyBridge: {}", super::REASON)).ok()?;
            let mut id = 0u32;
            // SAFETY: both C strings outlive the calls, the allocator null
            // means the default one, and every CF object made is released.
            let result = unsafe {
                let kind = CFStringCreateWithCString(
                    std::ptr::null(),
                    kind.as_ptr(),
                    CF_STRING_ENCODING_UTF8,
                );
                let name = CFStringCreateWithCString(
                    std::ptr::null(),
                    name.as_ptr(),
                    CF_STRING_ENCODING_UTF8,
                );
                if kind.is_null() || name.is_null() {
                    if !kind.is_null() {
                        CFRelease(kind);
                    }
                    if !name.is_null() {
                        CFRelease(name);
                    }
                    return None;
                }
                let result = IOPMAssertionCreateWithName(kind, ASSERTION_LEVEL_ON, name, &mut id);
                CFRelease(kind);
                CFRelease(name);
                result
            };
            (result == SUCCESS).then_some(Self(id))
        }
    }

    impl Drop for Assertion {
        fn drop(&mut self) {
            // SAFETY: releases the id this assertion was created with.
            unsafe { IOPMAssertionRelease(self.0) };
        }
    }
}

#![cfg(windows)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{
    BOOL, E_NOINTERFACE, E_POINTER, GlobalFree, HWND, POINT, POINTL, S_OK,
};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::Com::{
    FORMATETC, IAdviseSink, IDataObject, IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM, STGMEDIUM_0,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
};
use windows_sys::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE,
};
use windows_sys::Win32::UI::Shell::DROPFILES;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow};
use windows_sys::core::{GUID, HRESULT, IUnknown};

mod dpi {
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct PhysicalPosition<T> {
        pub x: T,
        pub y: T,
    }

    impl<T> PhysicalPosition<T> {
        pub const fn new(x: T, y: T) -> Self {
            Self { x, y }
        }
    }
}

mod event {
    use std::marker::PhantomData;
    use std::path::PathBuf;

    use crate::dpi::PhysicalPosition;
    use crate::window::WindowId;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Event<T> {
        WindowEvent {
            window_id: WindowId,
            event: WindowEvent,
        },
        _Marker(PhantomData<T>),
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum WindowEvent {
        CursorMoved {
            device_id: (),
            position: PhysicalPosition<f64>,
        },
        HoveredFile(PathBuf),
        HoveredFileCancelled,
        DroppedFile(PathBuf),
    }
}

mod window {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct WindowId(pub(crate) crate::platform_impl::platform::WindowId);
}

#[rustfmt::skip]
#[path = "../vendor/winit/src/platform_impl/windows/definitions.rs"]
pub mod definitions;

pub const DEVICE_ID: () = ();

pub mod platform_impl {
    pub mod platform {
        use windows_sys::Win32::Foundation::HWND;

        pub use crate::definitions;

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct WindowId(pub HWND);
    }
}

#[rustfmt::skip]
#[path = "../vendor/winit/src/platform_impl/windows/drop_handler.rs"]
mod drop_handler;

use drop_handler::FileDropHandler;
use event::{Event, WindowEvent};

#[repr(C)]
struct RawDataObjectVtbl {
    parent: definitions::IUnknownVtbl,
    get_data:
        unsafe extern "system" fn(*mut IDataObject, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    get_data_here:
        unsafe extern "system" fn(*mut IDataObject, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    query_get_data: unsafe extern "system" fn(*mut IDataObject, *const FORMATETC) -> HRESULT,
    get_canonical_format_etc:
        unsafe extern "system" fn(*mut IDataObject, *const FORMATETC, *mut FORMATETC) -> HRESULT,
    set_data: unsafe extern "system" fn(
        *mut IDataObject,
        *const FORMATETC,
        *const FORMATETC,
        BOOL,
    ) -> HRESULT,
    enum_format_etc:
        unsafe extern "system" fn(*mut IDataObject, u32, *mut *mut IEnumFORMATETC) -> HRESULT,
    d_advise: unsafe extern "system" fn(
        *mut IDataObject,
        *const FORMATETC,
        u32,
        *const IAdviseSink,
        *mut u32,
    ) -> HRESULT,
    d_unadvise: unsafe extern "system" fn(*mut IDataObject, u32) -> HRESULT,
    enum_d_advise:
        unsafe extern "system" fn(*mut IDataObject, *const *const IEnumSTATDATA) -> HRESULT,
}

#[repr(C)]
struct Releaser {
    vtbl: *const definitions::IUnknownVtbl,
    releases: Arc<AtomicUsize>,
}

unsafe extern "system" fn releaser_query_interface(
    _this: *mut IUnknown,
    _riid: *const GUID,
    _out: *mut *mut c_void,
) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn releaser_add_ref(_this: *mut IUnknown) -> u32 {
    1
}

unsafe extern "system" fn releaser_release(this: *mut IUnknown) -> u32 {
    unsafe {
        (*(this as *mut Releaser))
            .releases
            .fetch_add(1, Ordering::SeqCst)
    };
    1
}

static RELEASER_VTBL: definitions::IUnknownVtbl = definitions::IUnknownVtbl {
    QueryInterface: releaser_query_interface,
    AddRef: releaser_add_ref,
    Release: releaser_release,
};

#[repr(C)]
struct FakeDataObject {
    vtbl: *const RawDataObjectVtbl,
    hglobal: *mut c_void,
    releaser: Box<Releaser>,
}

unsafe extern "system" fn fake_query_interface(
    _this: *mut IUnknown,
    _riid: *const GUID,
    _out: *mut *mut c_void,
) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn fake_add_ref(_this: *mut IUnknown) -> u32 {
    1
}

unsafe extern "system" fn fake_release(_this: *mut IUnknown) -> u32 {
    1
}

unsafe extern "system" fn fake_get_data(
    this: *mut IDataObject,
    format: *const FORMATETC,
    medium: *mut STGMEDIUM,
) -> HRESULT {
    let object = unsafe { &mut *(this as *mut FakeDataObject) };
    let format = unsafe { &*format };
    assert_eq!(format.cfFormat, CF_HDROP);
    unsafe {
        *medium = STGMEDIUM {
            tymed: windows_sys::Win32::System::Com::TYMED_HGLOBAL as u32,
            u: STGMEDIUM_0 {
                hGlobal: object.hglobal,
            },
            pUnkForRelease: object.releaser.as_mut() as *mut Releaser as *mut c_void,
        };
    }
    S_OK
}

unsafe extern "system" fn unsupported_get_data_here(
    _this: *mut IDataObject,
    _format: *const FORMATETC,
    _medium: *mut STGMEDIUM,
) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn unsupported_query_get_data(
    _this: *mut IDataObject,
    _format: *const FORMATETC,
) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn unsupported_canonical_format(
    _this: *mut IDataObject,
    _input: *const FORMATETC,
    _output: *mut FORMATETC,
) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn unsupported_set_data(
    _this: *mut IDataObject,
    _format: *const FORMATETC,
    _medium: *const FORMATETC,
    _release: BOOL,
) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn unsupported_enum_format(
    _this: *mut IDataObject,
    _direction: u32,
    _result: *mut *mut IEnumFORMATETC,
) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn unsupported_advise(
    _this: *mut IDataObject,
    _format: *const FORMATETC,
    _flags: u32,
    _sink: *const IAdviseSink,
    _connection: *mut u32,
) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn unsupported_unadvise(
    _this: *mut IDataObject,
    _connection: u32,
) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn unsupported_enum_advise(
    _this: *mut IDataObject,
    _result: *const *const IEnumSTATDATA,
) -> HRESULT {
    E_NOINTERFACE
}

static DATA_OBJECT_VTBL: RawDataObjectVtbl = RawDataObjectVtbl {
    parent: definitions::IUnknownVtbl {
        QueryInterface: fake_query_interface,
        AddRef: fake_add_ref,
        Release: fake_release,
    },
    get_data: fake_get_data,
    get_data_here: unsupported_get_data_here,
    query_get_data: unsupported_query_get_data,
    get_canonical_format_etc: unsupported_canonical_format,
    set_data: unsupported_set_data,
    enum_format_etc: unsupported_enum_format,
    d_advise: unsupported_advise,
    d_unadvise: unsupported_unadvise,
    enum_d_advise: unsupported_enum_advise,
};

impl FakeDataObject {
    fn new(paths: &[PathBuf], releases: Arc<AtomicUsize>) -> Self {
        let mut encoded = Vec::new();
        for path in paths {
            encoded.extend(path.as_os_str().encode_wide());
            encoded.push(0);
        }
        encoded.push(0);

        let bytes = std::mem::size_of::<DROPFILES>() + encoded.len() * std::mem::size_of::<u16>();
        let hglobal = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes) };
        assert!(
            !hglobal.is_null(),
            "GlobalAlloc must provide the CF_HDROP payload"
        );
        let memory = unsafe { GlobalLock(hglobal) };
        assert!(
            !memory.is_null(),
            "GlobalLock must provide the CF_HDROP payload"
        );
        unsafe {
            ptr::write(
                memory.cast::<DROPFILES>(),
                DROPFILES {
                    pFiles: std::mem::size_of::<DROPFILES>() as u32,
                    pt: POINT { x: 0, y: 0 },
                    fNC: 0,
                    fWide: 1,
                },
            );
            ptr::copy_nonoverlapping(
                encoded.as_ptr(),
                memory
                    .cast::<u8>()
                    .add(std::mem::size_of::<DROPFILES>())
                    .cast::<u16>(),
                encoded.len(),
            );
            GlobalUnlock(hglobal);
        }

        Self {
            vtbl: &DATA_OBJECT_VTBL,
            hglobal,
            releaser: Box::new(Releaser {
                vtbl: &RELEASER_VTBL,
                releases,
            }),
        }
    }

    fn interface(&mut self) -> *const IDataObject {
        self as *mut Self as *const IDataObject
    }
}

impl Drop for FakeDataObject {
    fn drop(&mut self) {
        unsafe { GlobalFree(self.hglobal) };
    }
}

struct TestWindow(HWND);

impl TestWindow {
    fn new() -> Self {
        let class = [
            b'S' as u16,
            b'T' as u16,
            b'A' as u16,
            b'T' as u16,
            b'I' as u16,
            b'C' as u16,
            0,
        ];
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                113,
                79,
                320,
                240,
                0,
                0,
                0,
                ptr::null(),
            )
        };
        assert_ne!(hwnd, 0, "the built-in STATIC window must be created");
        Self(hwnd)
    }

    fn screen_point(&self, x: i32, y: i32) -> POINTL {
        let mut point = POINT { x, y };
        assert_ne!(
            unsafe { ClientToScreen(self.0, &mut point) },
            0,
            "ClientToScreen must translate the test point"
        );
        POINTL {
            x: point.x,
            y: point.y,
        }
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        assert_ne!(
            unsafe { DestroyWindow(self.0) },
            0,
            "the test window must be destroyed"
        );
    }
}

fn new_handler(hwnd: HWND) -> (FileDropHandler, Arc<Mutex<Vec<Event<()>>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&events);
    let handler = FileDropHandler::new(
        hwnd,
        Box::new(move |event| recorded.lock().unwrap().push(event)),
    );
    (handler, events)
}

fn target_vtbl(handler: &FileDropHandler) -> &definitions::IDropTargetVtbl {
    unsafe { &*(*handler.data).interface.lpVtbl }
}

fn iid(data1: u32) -> GUID {
    GUID {
        data1,
        data2: 0,
        data3: 0,
        data4: [0xc0, 0, 0, 0, 0, 0, 0, 0x46],
    }
}

#[test]
fn native_ole_callbacks_translate_points_order_events_and_release_media() {
    let window = TestWindow::new();
    let paths = vec![
        PathBuf::from(r"C:\資料\一.txt"),
        PathBuf::from(format!(r"C:\long\{}\файл.txt", "z".repeat(320))),
        PathBuf::from(r"C:\資料\比較フォルダ"),
    ];
    let releases = Arc::new(AtomicUsize::new(0));
    let mut object = FakeDataObject::new(&paths, Arc::clone(&releases));
    let (handler, events) = new_handler(window.0);
    let vtbl = target_vtbl(&handler);

    let enter = window.screen_point(17, -9);
    let mut effect = DROPEFFECT_COPY;
    assert_eq!(
        unsafe {
            (vtbl.DragEnter)(
                handler.data.cast(),
                object.interface(),
                0,
                enter,
                &mut effect,
            )
        },
        S_OK
    );
    assert_eq!(effect, DROPEFFECT_COPY);
    assert_eq!(releases.load(Ordering::SeqCst), 1);

    for (x, y) in [(-31, 24), (220, -47)] {
        let point = window.screen_point(x, y);
        effect = DROPEFFECT_COPY;
        assert_eq!(
            unsafe { (vtbl.DragOver)(handler.data.cast(), 0, point, &mut effect) },
            S_OK
        );
        assert_eq!(effect, DROPEFFECT_COPY);
    }

    let dropped_at = window.screen_point(-5, 191);
    effect = DROPEFFECT_COPY;
    assert_eq!(
        unsafe {
            (vtbl.Drop)(
                handler.data.cast(),
                object.interface(),
                0,
                dropped_at,
                &mut effect,
            )
        },
        S_OK
    );
    assert_eq!(effect, DROPEFFECT_COPY);
    assert_eq!(
        releases.load(Ordering::SeqCst),
        2,
        "every successful GetData medium is released"
    );

    let events = events.lock().unwrap();
    assert_eq!(events.len(), 10);
    assert!(
        matches!(events[0], Event::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, .. } if position.x == 17.0 && position.y == -9.0)
    );
    assert!(
        matches!(events[1], Event::WindowEvent { event: WindowEvent::HoveredFile(ref path), .. } if path == &paths[0])
    );
    assert!(
        matches!(events[2], Event::WindowEvent { event: WindowEvent::HoveredFile(ref path), .. } if path == &paths[1])
    );
    assert!(
        matches!(events[3], Event::WindowEvent { event: WindowEvent::HoveredFile(ref path), .. } if path == &paths[2])
    );
    assert!(
        matches!(events[4], Event::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, .. } if position.x == -31.0 && position.y == 24.0)
    );
    assert!(
        matches!(events[5], Event::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, .. } if position.x == 220.0 && position.y == -47.0)
    );
    assert!(
        matches!(events[6], Event::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, .. } if position.x == -5.0 && position.y == 191.0)
    );
    assert!(
        matches!(events[7], Event::WindowEvent { event: WindowEvent::DroppedFile(ref path), .. } if path == &paths[0])
    );
    assert!(
        matches!(events[8], Event::WindowEvent { event: WindowEvent::DroppedFile(ref path), .. } if path == &paths[1])
    );
    assert!(
        matches!(events[9], Event::WindowEvent { event: WindowEvent::DroppedFile(ref path), .. } if path == &paths[2])
    );
}

#[test]
fn copy_permission_leave_cancellation_drop_cleanup_and_query_interface_are_native() {
    let window = TestWindow::new();
    let paths = vec![PathBuf::from(r"C:\資料\一.txt")];
    let releases = Arc::new(AtomicUsize::new(0));
    let mut object = FakeDataObject::new(&paths, Arc::clone(&releases));
    let (handler, events) = new_handler(window.0);
    let vtbl = target_vtbl(&handler);
    let point = window.screen_point(4, 7);

    let mut effect = DROPEFFECT_MOVE;
    assert_eq!(
        unsafe {
            (vtbl.DragEnter)(
                handler.data.cast(),
                object.interface(),
                0,
                point,
                &mut effect,
            )
        },
        S_OK
    );
    assert_eq!(
        effect, DROPEFFECT_NONE,
        "a source that forbids COPY must not receive COPY"
    );
    assert_eq!(unsafe { (vtbl.DragLeave)(handler.data.cast()) }, S_OK);

    effect = DROPEFFECT_COPY;
    assert_eq!(
        unsafe {
            (vtbl.DragEnter)(
                handler.data.cast(),
                object.interface(),
                0,
                point,
                &mut effect,
            )
        },
        S_OK
    );
    assert_eq!(
        unsafe {
            (vtbl.Drop)(
                handler.data.cast(),
                object.interface(),
                0,
                point,
                &mut effect,
            )
        },
        S_OK
    );
    assert_eq!(unsafe { (vtbl.DragLeave)(handler.data.cast()) }, S_OK);
    assert_eq!(releases.load(Ordering::SeqCst), 3);

    let mut result = ptr::null_mut();
    let unknown = iid(0);
    assert_eq!(
        unsafe { (vtbl.parent.QueryInterface)(handler.data.cast(), &unknown, &mut result) },
        S_OK
    );
    assert_eq!(result, handler.data.cast());
    assert_eq!(unsafe { (vtbl.parent.Release)(result.cast()) }, 1);

    result = ptr::null_mut();
    let drop_target = iid(0x0000_0122);
    assert_eq!(
        unsafe { (vtbl.parent.QueryInterface)(handler.data.cast(), &drop_target, &mut result) },
        S_OK
    );
    assert_eq!(result, handler.data.cast());
    assert_eq!(unsafe { (vtbl.parent.Release)(result.cast()) }, 1);

    result = 1usize as *mut c_void;
    let unsupported = iid(0xdead_beef);
    assert_eq!(
        unsafe { (vtbl.parent.QueryInterface)(handler.data.cast(), &unsupported, &mut result) },
        E_NOINTERFACE
    );
    assert!(result.is_null());

    let wrong_drop_target = GUID {
        data1: 0x0000_0122,
        data2: 0,
        data3: 0,
        data4: [0; 8],
    };
    result = 1usize as *mut c_void;
    assert_eq!(
        unsafe {
            (vtbl.parent.QueryInterface)(handler.data.cast(), &wrong_drop_target, &mut result)
        },
        E_NOINTERFACE
    );
    assert!(result.is_null());

    assert_eq!(
        unsafe { (vtbl.parent.QueryInterface)(handler.data.cast(), &unknown, ptr::null_mut()) },
        E_POINTER
    );
    result = 1usize as *mut c_void;
    assert_eq!(
        unsafe { (vtbl.parent.QueryInterface)(handler.data.cast(), ptr::null(), &mut result) },
        E_POINTER
    );
    assert!(result.is_null());

    let events = events.lock().unwrap();
    assert!(matches!(
        events.last(),
        Some(Event::WindowEvent {
            event: WindowEvent::DroppedFile(_),
            ..
        })
    ));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                Event::WindowEvent {
                    event: WindowEvent::HoveredFileCancelled,
                    ..
                }
            ))
            .count(),
        1
    );
}

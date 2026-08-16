//! Minimal generated-style bindings for `DesktopWindowXamlSource`.

use std::ffi::c_void;
use windows_core::{HRESULT, IInspectable, IInspectable_Vtbl, IUnknown, Interface};

#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DesktopWindowXamlSource(IUnknown);

windows_core::imp::interface_hierarchy!(DesktopWindowXamlSource, IUnknown, IInspectable);

impl DesktopWindowXamlSource {
    pub(super) fn new() -> windows_core::Result<Self> {
        Self::factory(|factory| unsafe {
            let mut inner = std::ptr::null_mut();
            let mut result = std::ptr::null_mut();
            (Interface::vtable(factory).create_instance)(
                Interface::as_raw(factory),
                std::ptr::null_mut(),
                &mut inner,
                &mut result,
            )
            .ok()?;
            let inner: IUnknown = windows_core::Type::from_abi(inner)?;
            drop(inner);
            windows_core::Type::from_abi(result)
        })
    }

    pub(super) fn set_content(&self, content: &IInspectable) -> windows_core::Result<()> {
        unsafe {
            (Interface::vtable(self).set_content)(
                Interface::as_raw(self),
                Interface::as_raw(content),
            )
            .ok()
        }
    }

    pub(super) fn initialize(&self, parent: WindowId) -> windows_core::Result<()> {
        unsafe { (Interface::vtable(self).initialize)(Interface::as_raw(self), parent).ok() }
    }

    pub(super) fn site_bridge(&self) -> windows_core::Result<DesktopChildSiteBridge> {
        unsafe {
            let mut result = std::ptr::null_mut();
            (Interface::vtable(self).site_bridge)(Interface::as_raw(self), &mut result).ok()?;
            windows_core::Type::from_abi(result)
        }
    }

    fn factory<R>(
        callback: impl FnOnce(&IDesktopWindowXamlSourceFactory) -> windows_core::Result<R>,
    ) -> windows_core::Result<R> {
        static SHARED: windows_core::imp::FactoryCache<
            DesktopWindowXamlSource,
            IDesktopWindowXamlSourceFactory,
        > = windows_core::imp::FactoryCache::new();
        SHARED.call(callback)
    }
}

impl windows_core::RuntimeType for DesktopWindowXamlSource {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IDesktopWindowXamlSource>();
}

unsafe impl Interface for DesktopWindowXamlSource {
    type Vtable = IDesktopWindowXamlSource_Vtbl;
    const IID: windows_core::GUID = IDesktopWindowXamlSource::IID;
}

impl windows_core::RuntimeName for DesktopWindowXamlSource {
    const NAME: &'static str = "Microsoft.UI.Xaml.Hosting.DesktopWindowXamlSource";
}

windows_core::imp::define_interface!(
    IDesktopWindowXamlSource,
    IDesktopWindowXamlSource_Vtbl,
    0x553af92c_1381_51d6_bee0_f34beb042ea8
);

impl windows_core::RuntimeType for IDesktopWindowXamlSource {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
    const NAME: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::from_slice(
        b"Microsoft.UI.Xaml.Hosting.IDesktopWindowXamlSource",
    );
}

#[repr(C)]
pub struct IDesktopWindowXamlSource_Vtbl {
    base__: IInspectable_Vtbl,
    content: usize,
    set_content: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    has_focus: usize,
    system_backdrop: usize,
    set_system_backdrop: usize,
    site_bridge: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    take_focus_requested: usize,
    remove_take_focus_requested: usize,
    got_focus: usize,
    remove_got_focus: usize,
    navigate_focus: usize,
    initialize: unsafe extern "system" fn(*mut c_void, WindowId) -> HRESULT,
}

windows_core::imp::define_interface!(
    IDesktopWindowXamlSourceFactory,
    IDesktopWindowXamlSourceFactory_Vtbl,
    0x7d2db617_14e7_5d49_aeec_ae10887e595d
);

#[repr(C)]
pub struct IDesktopWindowXamlSourceFactory_Vtbl {
    base__: IInspectable_Vtbl,
    create_instance: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DesktopChildSiteBridge(IUnknown);

windows_core::imp::interface_hierarchy!(DesktopChildSiteBridge, IUnknown, IInspectable);

unsafe impl Interface for DesktopChildSiteBridge {
    type Vtable = IDesktopChildSiteBridge_Vtbl;
    const IID: windows_core::GUID = IDesktopChildSiteBridge::IID;
}

impl DesktopChildSiteBridge {
    pub(super) fn window_id(&self) -> windows_core::Result<WindowId> {
        let bridge = self.cast::<IDesktopSiteBridge>()?;
        unsafe {
            let mut result = WindowId::default();
            (Interface::vtable(&bridge).window_id)(Interface::as_raw(&bridge), &mut result).ok()?;
            Ok(result)
        }
    }

    pub(super) fn move_and_resize(&self, rect: RectInt32) -> windows_core::Result<()> {
        let bridge = self.cast::<IDesktopSiteBridge>()?;
        unsafe {
            (Interface::vtable(&bridge).move_and_resize)(Interface::as_raw(&bridge), rect).ok()
        }
    }

    pub(super) fn move_in_z_order_at_top(&self) -> windows_core::Result<()> {
        let bridge = self.cast::<IDesktopSiteBridge>()?;
        unsafe {
            (Interface::vtable(&bridge).move_in_z_order_at_top)(Interface::as_raw(&bridge)).ok()
        }
    }

    pub(super) fn show(&self) -> windows_core::Result<()> {
        let bridge = self.cast::<IDesktopSiteBridge>()?;
        unsafe { (Interface::vtable(&bridge).show)(Interface::as_raw(&bridge)).ok() }
    }
}

windows_core::imp::define_interface!(
    IDesktopChildSiteBridge,
    IDesktopChildSiteBridge_Vtbl,
    0xb2f2ff7b_1825_51b0_b80b_7599889c569f
);

#[repr(C)]
pub struct IDesktopChildSiteBridge_Vtbl {
    base__: IInspectable_Vtbl,
    resize_policy: usize,
    set_resize_policy: usize,
}

windows_core::imp::define_interface!(
    IDesktopSiteBridge,
    IDesktopSiteBridge_Vtbl,
    0xf0ae8750_905c_50a2_8a12_4545c6245bb4
);

#[repr(C)]
pub struct IDesktopSiteBridge_Vtbl {
    base__: IInspectable_Vtbl,
    is_enabled: usize,
    is_visible: usize,
    window_id: unsafe extern "system" fn(*mut c_void, *mut WindowId) -> HRESULT,
    connect: usize,
    disable: usize,
    enable: usize,
    hide: usize,
    move_and_resize: unsafe extern "system" fn(*mut c_void, RectInt32) -> HRESULT,
    move_in_z_order_at_bottom: usize,
    move_in_z_order_at_top: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    move_in_z_order_below: usize,
    show: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RectInt32 {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) width: i32,
    pub(super) height: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct WindowId {
    pub(super) value: u64,
}

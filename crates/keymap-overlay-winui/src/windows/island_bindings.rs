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
            let mut result = std::ptr::null_mut();
            (Interface::vtable(factory).create_instance)(
                Interface::as_raw(factory),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut result,
            )
            .ok()?;
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
    site_bridge: usize,
    take_focus_requested: usize,
    remove_take_focus_requested: usize,
    got_focus: usize,
    remove_got_focus: usize,
    navigate_focus: usize,
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

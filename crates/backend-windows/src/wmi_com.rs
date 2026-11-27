use omvix_core::{OmvixError, Result};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr;
use windows::core::{IUnknown, Interface, BSTR, HRESULT, HSTRING, PCWSTR, VARIANT};
use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, RPC_E_TOO_LATE, S_FALSE, S_OK};
use windows::Win32::Security::PSECURITY_DESCRIPTOR;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoInitializeSecurity, CoSetProxyBlanket, CoUninitialize,
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE, SAFEARRAY,
};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayCreateVector, SafeArrayDestroy, SafeArrayGetDim,
    SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::{
    VariantToUInt32, VT_ARRAY, VT_I4, VT_TYPEMASK, VT_UI1, VT_UI4,
};
use windows::Win32::System::Wmi::{
    IEnumWbemClassObject, IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator,
    WBEM_E_ACCESS_DENIED, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
    WBEM_GENERIC_FLAG_TYPE, WBEM_S_TIMEDOUT,
};

const WMI_QUERY_TIMEOUT_MS: i32 = 10_000;

const RPC_C_AUTHN_WINNT: u32 = 10;
const RPC_C_AUTHZ_NONE: u32 = 0;

struct ComApartmentGuard {
    should_uninit: bool,
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        if self.should_uninit {
            unsafe { CoUninitialize() };
        }
    }
}

thread_local! {
    static WMI_CONN_CACHE_DEPTH: Cell<usize> = const { Cell::new(0) };
    static ROOT_WMI_CONN_CACHE: RefCell<Option<WmiConnection>> = const { RefCell::new(None) };
}

pub(crate) fn set_root_wmi_cache_enabled(enabled: bool) {
    let next_depth = WMI_CONN_CACHE_DEPTH.with(|c| {
        let cur = c.get();
        let next = if enabled {
            cur.saturating_add(1)
        } else {
            cur.saturating_sub(1)
        };
        c.set(next);
        next
    });
    if next_depth == 0 {
        ROOT_WMI_CONN_CACHE.with(|cell| {
            if let Ok(mut slot) = cell.try_borrow_mut() {
                *slot = None;
            }
        });
    }
}

/// Safe COM/WMI session wrapper for querying namespaces and invoking WMI methods.
/// Field order matters: `services` MUST drop before `_com` calls `CoUninitialize`.
pub struct WmiConnection {
    services: IWbemServices,
    namespace: String,
    _com: ComApartmentGuard,
}

impl WmiConnection {
    /// Executes `f` using a persistent `root\WMI` connection when thread caching is enabled.
    pub(crate) fn with_cached_root_wmi<T>(f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let enabled = WMI_CONN_CACHE_DEPTH.with(|c| c.get() > 0);
        if !enabled {
            let conn = Self::connect(r"root\WMI")?;
            return f(&conn);
        }

        ROOT_WMI_CONN_CACHE.with(|cell| {
            let Ok(mut slot) = cell.try_borrow_mut() else {
                let conn = Self::connect(r"root\WMI")?;
                return f(&conn);
            };
            if slot.is_none() {
                *slot = Some(Self::connect(r"root\WMI")?);
            }
            let res = f(slot.as_ref().expect("root\\WMI connection populated"));
            if matches!(&res, Err(OmvixError::WmiUnavailable(_))) {
                *slot = None;
            }
            res
        })
    }

    pub fn connect(namespace: &str) -> Result<Self> {
        unsafe {
            let init_hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            let should_uninit = if init_hr == S_OK || init_hr == S_FALSE {
                true
            } else if init_hr == RPC_E_CHANGED_MODE {
                false
            } else {
                return Err(OmvixError::WmiUnavailable(format!(
                    "CoInitializeEx failed: 0x{:08X}",
                    init_hr.0 as u32
                )));
            };
            let com = ComApartmentGuard { should_uninit };

            if let Err(err) = CoInitializeSecurity(
                PSECURITY_DESCRIPTOR::default(),
                -1,
                None,
                None,
                RPC_C_AUTHN_LEVEL_DEFAULT,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
                None,
            ) {
                if err.code() != RPC_E_TOO_LATE {
                    tracing::debug!(
                        code = %format!("0x{:08X}", err.code().0 as u32),
                        "CoInitializeSecurity non-fatal status: {err}"
                    );
                }
            }

            let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| {
                    OmvixError::WmiUnavailable(format!("Failed to create IWbemLocator: {e}"))
                })?;

            let services = locator
                .ConnectServer(
                    &BSTR::from(namespace),
                    &BSTR::new(),
                    &BSTR::new(),
                    &BSTR::new(),
                    0,
                    &BSTR::new(),
                    None,
                )
                .map_err(|e| map_wmi_err(namespace, "ConnectServer", &e))?;

            CoSetProxyBlanket(
                &services,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )
            .map_err(|e| {
                OmvixError::WmiUnavailable(format!("CoSetProxyBlanket({namespace}) failed: {e}"))
            })?;

            Ok(Self {
                services,
                namespace: namespace.to_string(),
                _com: com,
            })
        }
    }

    pub fn query(&self, wql: &str) -> Result<Vec<IWbemClassObject>> {
        unsafe {
            let enumerator = self
                .services
                .ExecQuery(
                    &BSTR::from("WQL"),
                    &BSTR::from(wql),
                    WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                    None,
                )
                .map_err(|e| map_wmi_err(&self.namespace, wql, &e))?;

            let mut results = Vec::new();
            loop {
                let mut row: [Option<IWbemClassObject>; 1] = [None];
                let mut returned = 0u32;
                let hr: HRESULT = enumerator.Next(WMI_QUERY_TIMEOUT_MS, &mut row, &mut returned);
                if hr.0 == WBEM_E_ACCESS_DENIED.0 {
                    return Err(OmvixError::PermissionDenied(format!(
                        "Access denied querying `{wql}` in `{}`",
                        self.namespace
                    )));
                }
                if hr.0 == WBEM_S_TIMEDOUT.0 {
                    return Err(OmvixError::WmiUnavailable(format!(
                        "WMI query `{wql}` timed out after {}s in `{}`",
                        WMI_QUERY_TIMEOUT_MS / 1000,
                        self.namespace
                    )));
                }
                if hr.is_err() {
                    return Err(OmvixError::WmiUnavailable(format!(
                        "IEnumWbemClassObject::Next failed (0x{:08X}) for `{wql}`",
                        hr.0 as u32
                    )));
                }
                if returned == 0 {
                    break;
                }
                if let Some(obj) = row[0].take() {
                    results.push(obj);
                }
                if hr == S_FALSE {
                    break;
                }
            }
            Ok(results)
        }
    }

    pub fn get_object(&self, path: &str) -> Result<IWbemClassObject> {
        unsafe {
            let mut obj = None;
            self.services
                .GetObject(
                    &BSTR::from(path),
                    WBEM_GENERIC_FLAG_TYPE(0),
                    None,
                    Some(&mut obj),
                    None,
                )
                .map_err(|e| map_wmi_err(&self.namespace, path, &e))?;
            obj.ok_or_else(|| {
                OmvixError::WmiUnavailable(format!(
                    "WMI object `{path}` returned null in `{}`",
                    self.namespace
                ))
            })
        }
    }

    pub fn class_exists(&self, class_name: &str) -> bool {
        self.get_object(class_name).is_ok()
    }

    pub fn exec_notification_query(&self, wql: &str) -> Result<IEnumWbemClassObject> {
        unsafe {
            self.services
                .ExecNotificationQuery(
                    &BSTR::from("WQL"),
                    &BSTR::from(wql),
                    WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                    None,
                )
                .map_err(|e| map_wmi_err(&self.namespace, wql, &e))
        }
    }

    pub fn exec_method(
        &self,
        object_path: &str,
        method_name: &str,
        in_params: Option<&IWbemClassObject>,
    ) -> Result<Option<IWbemClassObject>> {
        unsafe {
            let mut out_params = None;
            self.services
                .ExecMethod(
                    &BSTR::from(object_path),
                    &BSTR::from(method_name),
                    WBEM_GENERIC_FLAG_TYPE(0),
                    None,
                    in_params,
                    Some(&mut out_params),
                    None,
                )
                .map_err(|e| map_wmi_err(&self.namespace, method_name, &e))?;
            Ok(out_params)
        }
    }
}

fn map_wmi_err(namespace: &str, op: &str, err: &windows::core::Error) -> OmvixError {
    if err.code().0 == WBEM_E_ACCESS_DENIED.0 {
        OmvixError::PermissionDenied(format!(
            "Administrator privileges required for `{op}` in `{namespace}`"
        ))
    } else {
        OmvixError::WmiUnavailable(format!("WMI `{op}` in `{namespace}` failed: {err}"))
    }
}

pub fn get_property(obj: &IWbemClassObject, name: &str) -> Option<VARIANT> {
    let key = HSTRING::from(name);
    let mut val = VARIANT::new();
    unsafe {
        if obj
            .Get(PCWSTR(key.as_ptr()), 0, &mut val, None, None)
            .is_ok()
            && !val.is_empty()
        {
            Some(val)
        } else {
            None
        }
    }
}

pub fn get_string(obj: &IWbemClassObject, name: &str) -> Option<String> {
    let val = get_property(obj, name)?;
    BSTR::try_from(&val).ok().map(|b| b.to_string())
}

pub fn get_u32(obj: &IWbemClassObject, name: &str) -> Option<u32> {
    let val = get_property(obj, name)?;
    unsafe { VariantToUInt32(&val).ok() }
}

pub fn get_u32_array(obj: &IWbemClassObject, name: &str) -> Option<Vec<u32>> {
    let val = get_property(obj, name)?;
    unsafe {
        let vt = val.as_raw().Anonymous.Anonymous.vt;
        let elem_vt = vt & VT_TYPEMASK.0;
        if (vt & VT_ARRAY.0) == 0 || (elem_vt != VT_I4.0 && elem_vt != VT_UI4.0) {
            return None;
        }
        let psa = val.as_raw().Anonymous.Anonymous.Anonymous.parray as *const SAFEARRAY;
        if psa.is_null() || SafeArrayGetDim(psa) != 1 {
            return None;
        }
        let lbound = SafeArrayGetLBound(psa, 1).ok()?;
        let ubound = SafeArrayGetUBound(psa, 1).ok()?;
        let span = i64::from(ubound) - i64::from(lbound) + 1;
        if span <= 0 {
            return Some(Vec::new());
        }
        let Ok(count) = usize::try_from(span) else {
            return None;
        };
        let mut data_ptr: *mut c_void = ptr::null_mut();
        SafeArrayAccessData(psa, &mut data_ptr).ok()?;
        let out = if data_ptr.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(data_ptr as *const u32, count).to_vec()
        };
        let _ = SafeArrayUnaccessData(psa);
        Some(out)
    }
}

pub fn get_u8_array(obj: &IWbemClassObject, name: &str) -> Option<Vec<u8>> {
    let val = get_property(obj, name)?;
    unsafe {
        let vt = val.as_raw().Anonymous.Anonymous.vt;
        if (vt & VT_ARRAY.0) == 0 || (vt & VT_TYPEMASK.0) != VT_UI1.0 {
            return None;
        }
        let psa = val.as_raw().Anonymous.Anonymous.Anonymous.parray as *const SAFEARRAY;
        if psa.is_null() || SafeArrayGetDim(psa) != 1 {
            return None;
        }
        let lbound = SafeArrayGetLBound(psa, 1).ok()?;
        let ubound = SafeArrayGetUBound(psa, 1).ok()?;
        let span = i64::from(ubound) - i64::from(lbound) + 1;
        if span <= 0 {
            return Some(Vec::new());
        }
        let Ok(count) = usize::try_from(span) else {
            return None;
        };
        let mut data_ptr: *mut c_void = ptr::null_mut();
        SafeArrayAccessData(psa, &mut data_ptr).ok()?;
        let out = if data_ptr.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(data_ptr as *const u8, count).to_vec()
        };
        let _ = SafeArrayUnaccessData(psa);
        Some(out)
    }
}

pub fn get_object(obj: &IWbemClassObject, name: &str) -> Option<IWbemClassObject> {
    let val = get_property(obj, name)?;
    let unk = IUnknown::try_from(&val).ok()?;
    unk.cast::<IWbemClassObject>().ok()
}

pub fn variant_from_u32(val: u32) -> VARIANT {
    VARIANT::from(val as i32)
}

pub fn variant_from_u8_array(bytes: &[u8]) -> Result<VARIANT> {
    unsafe {
        let psa = SafeArrayCreateVector(VT_UI1, 0, bytes.len() as u32);
        if psa.is_null() {
            return Err(OmvixError::WmiUnavailable(
                "SafeArrayCreateVector(VT_UI1) returned null".to_string(),
            ));
        }

        if !bytes.is_empty() {
            let mut data_ptr: *mut c_void = ptr::null_mut();
            if let Err(err) = SafeArrayAccessData(psa, &mut data_ptr) {
                let _ = SafeArrayDestroy(psa);
                return Err(OmvixError::WmiUnavailable(format!(
                    "SafeArrayAccessData failed: {err}"
                )));
            }
            if data_ptr.is_null() {
                let _ = SafeArrayUnaccessData(psa);
                let _ = SafeArrayDestroy(psa);
                return Err(OmvixError::WmiUnavailable(
                    "SafeArrayAccessData returned null data pointer".into(),
                ));
            }
            ptr::copy_nonoverlapping(bytes.as_ptr(), data_ptr as *mut u8, bytes.len());
            let _ = SafeArrayUnaccessData(psa);
        }

        let mut raw: windows::core::imp::VARIANT = std::mem::zeroed();
        raw.Anonymous.Anonymous.vt = VT_ARRAY.0 | VT_UI1.0;
        raw.Anonymous.Anonymous.Anonymous.parray = psa as *mut _;
        Ok(VARIANT::from_raw(raw))
    }
}

pub fn put_variant(obj: &IWbemClassObject, name: &str, val: &VARIANT) -> Result<()> {
    let key = HSTRING::from(name);
    unsafe {
        obj.Put(PCWSTR(key.as_ptr()), 0, val, 0)
            .map_err(|e| OmvixError::WmiUnavailable(format!("IWbemClassObject::Put({name}): {e}")))
    }
}

pub fn get_method_in_params(
    class_obj: &IWbemClassObject,
    method_name: &str,
) -> Result<IWbemClassObject> {
    let method = HSTRING::from(method_name);
    let mut in_sig = None;
    unsafe {
        class_obj
            .GetMethod(PCWSTR(method.as_ptr()), 0, &mut in_sig, ptr::null_mut())
            .map_err(|e| {
                OmvixError::WmiUnavailable(format!("GetMethod({method_name}) failed: {e}"))
            })?;
        let sig = in_sig.ok_or_else(|| {
            OmvixError::WmiUnavailable(format!("Method `{method_name}` has no input signature"))
        })?;
        sig.SpawnInstance(0).map_err(|e| {
            OmvixError::WmiUnavailable(format!("SpawnInstance({method_name}) failed: {e}"))
        })
    }
}

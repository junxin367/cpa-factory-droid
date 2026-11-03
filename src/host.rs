use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    ptr,
    sync::{Mutex, OnceLock},
};

use crate::{Fault, Result};

#[repr(C)]
pub struct Buffer {
    pub ptr: *mut u8,
    pub len: usize,
}
type HostCall =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const u8, usize, *mut Buffer) -> i32;
type HostFree = unsafe extern "C" fn(*mut c_void, usize);
#[repr(C)]
pub struct HostApi {
    pub abi_version: u32,
    pub host_ctx: *mut c_void,
    pub call: Option<HostCall>,
    pub free_buffer: Option<HostFree>,
}
#[repr(C)]
pub struct PluginApi {
    pub abi_version: u32,
    pub call: Option<unsafe extern "C" fn(*const c_char, *const u8, usize, *mut Buffer) -> i32>,
    pub free_buffer: Option<unsafe extern "C" fn(*mut c_void, usize)>,
    pub shutdown: Option<unsafe extern "C" fn()>,
}
#[derive(Clone, Copy)]
pub struct Host {
    context: usize,
    call: HostCall,
    free: HostFree,
}
static HOST: OnceLock<Mutex<Option<Host>>> = OnceLock::new();

impl Host {
    pub fn get() -> Result<Self> {
        HOST.get()
            .and_then(|slot| slot.lock().ok().and_then(|host| *host))
            .ok_or_else(|| Fault::new(503, "host_unavailable", "CPA 宿主回调尚未就绪"))
    }
    pub fn call(self, method: &str, request: &Value) -> Result<Value> {
        let method = CString::new(method).map_err(|_| Fault::internal())?;
        let request = serde_json::to_vec(request).map_err(|_| Fault::internal())?;
        let mut out = Buffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        let rc = unsafe {
            (self.call)(
                self.context as *mut c_void,
                method.as_ptr(),
                request.as_ptr(),
                request.len(),
                &mut out,
            )
        };
        let parsed = if !out.ptr.is_null() && out.len <= 128 * 1024 * 1024 {
            let slice = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
            serde_json::from_slice::<Value>(slice).ok()
        } else {
            None
        };
        if !out.ptr.is_null() {
            unsafe { (self.free)(out.ptr.cast(), out.len) };
        }
        // Callback diagnostics can contain provider payloads. Never forward them to clients.
        if rc != 0 {
            return Err(Fault::new(
                502,
                "host_callback_failed",
                "CPA 宿主操作失败或请求已取消",
            ));
        }
        let parsed = parsed
            .ok_or_else(|| Fault::new(502, "host_protocol_error", "CPA 宿主返回了无效数据"))?;
        if parsed["ok"].as_bool() != Some(true) {
            return Err(Fault::new(
                502,
                "host_callback_failed",
                "CPA 宿主操作失败或请求已取消",
            ));
        }
        Ok(parsed.get("result").cloned().unwrap_or_else(|| json!({})))
    }
    pub fn log(self, message: &str) {
        let _ = self.call("host.log", &json!({"level":"warn","message":message}));
    }
}

pub fn encode(bytes: impl AsRef<[u8]>) -> String {
    STANDARD.encode(bytes)
}
pub fn decode(v: &Value) -> Result<Vec<u8>> {
    match v.as_str() {
        Some(s) if s.len() <= 96 * 1024 * 1024 => STANDARD
            .decode(s)
            .map_err(|_| Fault::input("Base64 数据无效")),
        None | Some("") => Ok(Vec::new()),
        _ => Err(Fault::new(413, "body_too_large", "请求超过大小限制")),
    }
}

#[no_mangle]
/// # Safety
/// CPA must provide ABI-1 function tables valid for the plugin lifetime.
pub unsafe extern "C" fn cliproxy_plugin_init(host: *const HostApi, plugin: *mut PluginApi) -> i32 {
    if host.is_null() || plugin.is_null() || (*host).abi_version != 1 {
        return 1;
    }
    let (Some(call), Some(free)) = ((*host).call, (*host).free_buffer) else {
        return 1;
    };
    let Ok(mut slot) = HOST.get_or_init(|| Mutex::new(None)).lock() else {
        return 1;
    };
    if slot.is_some() {
        return 1;
    }
    *slot = Some(Host {
        context: (*host).host_ctx as usize,
        call,
        free,
    });
    *plugin = PluginApi {
        abi_version: 1,
        call: Some(plugin_call),
        free_buffer: Some(plugin_free),
        shutdown: Some(plugin_shutdown),
    };
    0
}
unsafe extern "C" fn plugin_call(
    method: *const c_char,
    request: *const u8,
    len: usize,
    response: *mut Buffer,
) -> i32 {
    if response.is_null() {
        return 1;
    }
    *response = Buffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    if method.is_null() || (request.is_null() && len > 0) {
        return 1;
    }
    let result = std::panic::catch_unwind(|| {
        if len > 96 * 1024 * 1024 {
            return Err(Fault::new(413, "body_too_large", "请求超过大小限制"));
        }
        let name = CStr::from_ptr(method)
            .to_str()
            .map_err(|_| Fault::input("RPC 方法编码无效"))?;
        let bytes = if len == 0 {
            &[][..]
        } else {
            std::slice::from_raw_parts(request, len)
        };
        let data = if bytes.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(bytes).map_err(|_| Fault::input("RPC JSON 无效"))?
        };
        crate::dispatch(name, &data)
    })
    .unwrap_or_else(|_| Err(Fault::internal()));
    let envelope = match result {
        Ok(v) => json!({"ok":true,"result":v}),
        Err(e) => json!({"ok":false,"error":e.json()}),
    };
    let bytes = envelope.to_string().into_bytes().into_boxed_slice();
    (*response).len = bytes.len();
    (*response).ptr = Box::into_raw(bytes).cast::<u8>();
    0
}
unsafe extern "C" fn plugin_free(pointer: *mut c_void, len: usize) {
    if !pointer.is_null() {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            pointer.cast::<u8>(),
            len,
        )));
    }
}
unsafe extern "C" fn plugin_shutdown() {
    let _ = std::panic::catch_unwind(|| {
        crate::shutdown();
        // CPA 会复用仍映射的 DLL；旧运行时退出后才允许绑定新的宿主回调。
        if let Some(slot) = HOST.get() {
            if let Ok(mut host) = slot.lock() {
                *host = None;
            }
        }
    });
}

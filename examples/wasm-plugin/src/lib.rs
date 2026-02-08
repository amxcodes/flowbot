use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::os::raw::c_char;

/// Tool arguments
#[derive(Deserialize)]
struct Args {
    message: Option<String>,
}

/// Tool result
#[derive(Serialize)]
struct Result {
    output: String,
}

// Global allocator for WASM
static mut BUFFER: Vec<u8> = Vec::new();

/// Allocate memory (called from host)
#[no_mangle]
pub extern "C" fn alloc(size: i32) -> *mut u8 {
    unsafe {
        BUFFER = vec![0; size as usize];
        BUFFER.as_mut_ptr()
    }
}

/// Execute the tool (called from host)
#[no_mangle]
pub extern "C" fn execute(args_ptr: *const c_char, args_len: i32) -> *const c_char {
    // Read args from memory
    let args_slice = unsafe {
        std::slice::from_raw_parts(args_ptr as *const u8, args_len as usize)
    };
    
    let args_str = String::from_utf8_lossy(args_slice);
    
    // Parse args
    let args: Args = match serde_json::from_str(&args_str) {
        Ok(a) => a,
        Err(e) => {
            let error = Result {
                output: format!("Error parsing args: {}", e),
            };
            let json = serde_json::to_string(&error).unwrap();
            return CString::new(json).unwrap().into_raw();
        }
    };
    
    // Execute tool logic
    let message = args.message.unwrap_or_else(|| "Hello from WASM!".to_string());
    let output = format!("🎉 WASM Tool says: {}", message);
    
    // Create result
    let result = Result { output };
    let json = serde_json::to_string(&result).unwrap();
    
    // Return pointer to result
    CString::new(json).unwrap().into_raw()
}

/// Free memory (called from host after reading result)
#[no_mangle]
pub extern "C" fn dealloc(ptr: *mut c_char) {
    unsafe {
        if !ptr.is_null() {
            let _ = CString::from_raw(ptr);
        }
    }
}

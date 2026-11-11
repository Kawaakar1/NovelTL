//! Шифрование секретов (ключ DeepL, пароль прокси) через Windows DPAPI.
//! Формат: "dpapi:" + base64(blob). Привязка к учётной записи Windows.

/// Префикс зашифрованного значения.
pub const PREFIX: &str = "dpapi:";

#[cfg(windows)]
mod imp {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    /// Доп. энтропия для DPAPI.
    const ENTROPY: &[u8] = b"NovelTL";
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            p_data_in: *const DataBlob,
            sz_data_descr: *const u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut core::ffi::c_void,
            p_prompt_struct: *mut core::ffi::c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            p_data_in: *const DataBlob,
            ppsz_data_descr: *mut *mut u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut core::ffi::c_void,
            p_prompt_struct: *mut core::ffi::c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }

    fn blob(data: &[u8]) -> DataBlob {
        DataBlob { cb_data: data.len() as u32, pb_data: data.as_ptr() as *mut u8 }
    }

    /// Копирует выходной буфер DPAPI в Vec и освобождает его через LocalFree.
    unsafe fn take(out: DataBlob) -> Vec<u8> {
        if out.pb_data.is_null() {
            return Vec::new();
        }
        let v = std::slice::from_raw_parts(out.pb_data, out.cb_data as usize).to_vec();
        LocalFree(out.pb_data as *mut core::ffi::c_void);
        v
    }

    fn dpapi_protect(data: &[u8]) -> Option<Vec<u8>> {
        let input = blob(data);
        let entropy = blob(ENTROPY);
        let mut out = DataBlob { cb_data: 0, pb_data: std::ptr::null_mut() };
        // SAFETY: указатели валидны на время вызова, входные буферы не изменяются.
        unsafe {
            let ok = CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            );
            if ok == 0 {
                return None;
            }
            Some(take(out))
        }
    }

    fn dpapi_unprotect(data: &[u8]) -> Option<Vec<u8>> {
        let input = blob(data);
        let entropy = blob(ENTROPY);
        let mut out = DataBlob { cb_data: 0, pb_data: std::ptr::null_mut() };
        // SAFETY: см. dpapi_protect.
        unsafe {
            let ok = CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            );
            if ok == 0 {
                return None;
            }
            Some(take(out))
        }
    }

    pub fn protect(plain: &str) -> String {
        if plain.is_empty() {
            return String::new();
        }
        match dpapi_protect(plain.as_bytes()) {
            Some(b) => format!("{}{}", super::PREFIX, STANDARD.encode(b)),
            // лучше сохранить открыто, чем потерять ключ
            None => plain.to_string(),
        }
    }

    pub fn unprotect(stored: &str) -> String {
        let Some(rest) = stored.strip_prefix(super::PREFIX) else {
            return stored.to_string();
        };
        STANDARD
            .decode(rest)
            .ok()
            .and_then(|b| dpapi_unprotect(&b))
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default()
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn protect(plain: &str) -> String {
        plain.to_string()
    }

    pub fn unprotect(stored: &str) -> String {
        if stored.starts_with(super::PREFIX) {
            String::new()
        } else {
            stored.to_string()
        }
    }
}

/// Зашифровать строку. Пустая -> пустая; при ошибке DPAPI возвращает исходную.
pub fn protect(plain: &str) -> String {
    imp::protect(plain)
}

/// Расшифровать. Без префикса -> как есть (старые файлы); неудача -> пустая строка.
pub fn unprotect(stored: &str) -> String {
    imp::unprotect(stored)
}

/// Значение зашифровано (а не сохранено открытым текстом из-за сбоя DPAPI).
pub fn is_protected(stored: &str) -> bool {
    stored.starts_with(PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn roundtrip() {
        for s in ["ключ:fx", "секрет-ключ:fx", "abc:fx"] {
            let p = protect(s);
            assert!(p.starts_with(PREFIX));
            assert!(!p.contains(s));
            assert_eq!(unprotect(&p), s);
        }
    }

    #[test]
    fn empty_and_plain() {
        assert_eq!(protect(""), "");
        assert_eq!(unprotect("plain"), "plain");
    }

    #[test]
    fn broken_values_give_empty() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        assert_eq!(unprotect("dpapi:!!!не base64"), "");
        let junk = format!("dpapi:{}", STANDARD.encode([1u8, 2, 3]));
        assert_eq!(unprotect(&junk), "");
    }
}

//! Аватар пользователя: выбор файла (диалог Windows), обработка через WIC и хранение в папке данных.

use std::path::{Path, PathBuf};

use windows::core::{w, PCWSTR};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};

/// Сторона сохранённого аватара, пикселей.
const SIZE: u32 = 256;

/// Инициализация COM; если поток уже инициализирован иначе — это не ошибка.
fn init_com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

/// Стандартный диалог «Открыть» для выбора картинки. None — отмена или ошибка.
pub fn pick_image() -> Option<PathBuf> {
    init_com();
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let spec = [COMDLG_FILTERSPEC {
            pszName: w!("Изображения (*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.webp)"),
            pszSpec: w!("*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.webp"),
        }];
        dlg.SetFileTypes(&spec).ok()?;
        dlg.SetTitle(w!("Выберите аватар")).ok()?;
        dlg.Show(None).ok()?;
        let item = dlg.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        path.map(PathBuf::from)
    }
}

/// Читает картинку через WIC в RGBA8 (прямая альфа): (ширина, высота, пиксели).
fn decode(src: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    init_com();
    let wide: Vec<u16> = src.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let dec = factory
            .CreateDecoderFromFilename(
                PCWSTR(wide.as_ptr()),
                None,
                windows::Win32::Foundation::GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(|e| e.to_string())?;
        let frame = dec.GetFrame(0).map_err(|e| e.to_string())?;
        let conv = factory.CreateFormatConverter().map_err(|e| e.to_string())?;
        conv.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .map_err(|e| e.to_string())?;
        let (mut w, mut h) = (0u32, 0u32);
        conv.GetSize(&mut w, &mut h).map_err(|e| e.to_string())?;
        if w == 0 || h == 0 {
            return Err("пустая картинка".into());
        }
        let mut buf = vec![0u8; w as usize * h as usize * 4];
        conv.CopyPixels(std::ptr::null(), w * 4, &mut buf).map_err(|e| e.to_string())?;
        Ok((w, h, buf))
    }
}

/// Обрезка по центру до квадрата и масштаб до `SIZE`×`SIZE` (усреднение по площади, с учётом альфы).
fn square_resize(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
    let side = w.min(h) as f32;
    let x0 = (w as f32 - side) / 2.0;
    let y0 = (h as f32 - side) / 2.0;
    let k = side / SIZE as f32;
    let mut out = vec![0u8; (SIZE * SIZE * 4) as usize];
    for oy in 0..SIZE {
        let sy0 = y0 + oy as f32 * k;
        let sy1 = sy0 + k;
        for ox in 0..SIZE {
            let sx0 = x0 + ox as f32 * k;
            let sx1 = sx0 + k;
            let (mut r, mut g, mut b, mut a, mut wsum) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
            let (iy0, iy1) = (sy0.floor() as i64, (sy1.ceil() as i64).max(sy0.floor() as i64 + 1));
            let (ix0, ix1) = (sx0.floor() as i64, (sx1.ceil() as i64).max(sx0.floor() as i64 + 1));
            for iy in iy0..iy1 {
                let wy = (sy1.min(iy as f32 + 1.0) - sy0.max(iy as f32)).max(0.0);
                let cy = iy.clamp(0, h as i64 - 1) as usize;
                for ix in ix0..ix1 {
                    let wx = (sx1.min(ix as f32 + 1.0) - sx0.max(ix as f32)).max(0.0);
                    let wt = wx * wy;
                    if wt <= 0.0 {
                        continue;
                    }
                    let cx = ix.clamp(0, w as i64 - 1) as usize;
                    let i = (cy * w as usize + cx) * 4;
                    let al = px[i + 3] as f32 / 255.0;
                    r += px[i] as f32 * al * wt;
                    g += px[i + 1] as f32 * al * wt;
                    b += px[i + 2] as f32 * al * wt;
                    a += al * wt;
                    wsum += wt;
                }
            }
            let o = ((oy * SIZE + ox) * 4) as usize;
            if a > 0.0 && wsum > 0.0 {
                out[o] = (r / a).round().clamp(0.0, 255.0) as u8;
                out[o + 1] = (g / a).round().clamp(0.0, 255.0) as u8;
                out[o + 2] = (b / a).round().clamp(0.0, 255.0) as u8;
                out[o + 3] = (a / wsum * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())
}

/// Сохраняет аватар в `dir` под уникальным именем; возвращает имя файла.
fn store_avatar_to(dir: &Path, src: &Path) -> Result<String, String> {
    let (w, h, px) = decode(src).map_err(|e| format!("Не удалось открыть картинку: {e}"))?;
    let out = square_resize(w, h, &px);
    let mut ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = loop {
        let n = format!("avatar_{ms}.png");
        if !dir.join(&n).exists() {
            break n;
        }
        ms += 1;
    };
    write_png(&dir.join(&name), SIZE, SIZE, &out).map_err(|e| format!("Не удалось сохранить аватар: {e}"))?;
    Ok(name)
}

/// Обрезает картинку до квадрата, уменьшает до 256×256 и сохраняет в папку данных.
pub fn store_avatar(src: &Path) -> Result<String, String> {
    store_avatar_to(&crate::paths::data_dir(), src)
}

fn load_rgba_from(dir: &Path, file_name: &str) -> Option<(u32, u32, Vec<u8>)> {
    if file_name.is_empty() || file_name.contains(['/', '\\']) {
        return None;
    }
    let file = std::fs::File::open(dir.join(file_name)).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

/// Читает сохранённый аватар (RGBA8) из папки данных.
pub fn load_rgba(file_name: &str) -> Option<(u32, u32, Vec<u8>)> {
    load_rgba_from(&crate::paths::data_dir(), file_name)
}

fn cleanup_in(dir: &Path, keep: &str) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("avatar_") && name.ends_with(".png") && name != keep {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Удаляет из папки данных все `avatar_*.png`, кроме `keep` (пусто — удалить все).
pub fn cleanup(keep: &str) {
    cleanup_in(&crate::paths::data_dir(), keep);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_load_cleanup() {
        let dir = std::env::temp_dir().join(format!("noveltl_avatar_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // исходник 300x200
        let mut px = Vec::new();
        for y in 0..200u32 {
            for x in 0..300u32 {
                px.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128, 255]);
            }
        }
        let src = dir.join("src.png");
        write_png(&src, 300, 200, &px).unwrap();

        let name = store_avatar_to(&dir, &src).unwrap();
        assert!(dir.join(&name).exists());
        let (w, h, data) = load_rgba_from(&dir, &name).unwrap();
        assert_eq!((w, h), (256, 256));
        assert_eq!(data.len(), 256 * 256 * 4);

        let other = store_avatar_to(&dir, &src).unwrap();
        assert_ne!(name, other);
        cleanup_in(&dir, &name);
        assert!(dir.join(&name).exists());
        assert!(!dir.join(&other).exists());
        assert!(src.exists());
        cleanup_in(&dir, "");
        assert!(!dir.join(&name).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

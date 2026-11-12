//! Physical text advances for adaptive Compact sizing.
//!
//! Measurement runs through the same GDI text engine the renderer uses, so a
//! measured box always fits the glyphs `render_text_mask` will draw. The DC is
//! thread-local: layout never crosses the render thread and creating a fresh
//! DC per string would dominate the cost of a frame.

use std::collections::{HashMap, VecDeque};

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::*;

/// Physical sizes the layout has to survive. A theme size is expressed in
/// 96-DPI logical units, so the same string is laid out against every scale
/// the widget may be rendered at.
pub const MEASURE_SCALES: [f64; 7] = [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

const CACHE_CAPACITY: usize = 1024;
/// One logical pixel of slack per scale keeps rounding from clipping a glyph.
const DT_MEASURE: u32 = 0x20 /* DT_SINGLELINE */ | 0x800 /* DT_NOPREFIX */ | 0x400 /* DT_CALCRECT */;

/// Widest advance of `text` in 96-DPI logical pixels, over every supported
/// render scale. Returns `None` for empty text or when GDI cannot measure it,
/// so the caller can fall back to the legacy reserve.
pub fn measure_text(text: &str, family: &str, logical_size: f64, weight: i32) -> Option<f64> {
    if text.is_empty() || !logical_size.is_finite() || logical_size <= 0.0 {
        return None;
    }
    let mut widest: Option<f64> = None;
    for scale in MEASURE_SCALES {
        let physical = (logical_size * scale).round().max(1.0) as i32;
        let advance = measured_advance(text, family, physical, weight)?;
        let logical = (f64::from(advance) / scale).ceil();
        widest = Some(widest.map_or(logical, |current| current.max(logical)));
    }
    widest
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct MeasureKey {
    family: String,
    physical: i32,
    weight: i32,
    text: String,
}

#[derive(Default)]
struct AdvanceCache {
    values: HashMap<MeasureKey, i32>,
    order: VecDeque<MeasureKey>,
}

impl AdvanceCache {
    fn get(&self, key: &MeasureKey) -> Option<i32> {
        self.values.get(key).copied()
    }

    fn insert(&mut self, key: MeasureKey, advance: i32) {
        if self.values.len() >= CACHE_CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.values.remove(&oldest);
            }
        }
        self.order.push_back(key.clone());
        self.values.insert(key, advance);
    }
}

thread_local! {
    static MEASURE_DC: std::cell::RefCell<Option<MeasureDc>> = const { std::cell::RefCell::new(None) };
    static ADVANCE_CACHE: std::cell::RefCell<AdvanceCache> = std::cell::RefCell::new(AdvanceCache::default());
    static GDI_FAILURE_LOGGED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A memory DC backed by a 1x1 DIB. `DT_CALCRECT` never draws, so one pixel
/// is enough and the surface cannot be mistaken for the render target.
struct MeasureDc {
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
}

impl Drop for MeasureDc {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old_bitmap);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

fn measure_dc_failure() {
    GDI_FAILURE_LOGGED.with(|logged| {
        if !logged.replace(true) {
            crate::diagnose::log(
                "text measurement DC unavailable; adaptive sizing uses legacy reserves",
            );
        }
    });
}

fn with_measure_dc<R>(action: impl FnOnce(&MeasureDc) -> Option<R>) -> Option<R> {
    MEASURE_DC.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = unsafe { create_measure_dc() };
            if slot.is_none() {
                measure_dc_failure();
            }
        }
        let dc = slot.as_ref()?;
        action(dc)
    })
}

unsafe fn create_measure_dc() -> Option<MeasureDc> {
    let dc = CreateCompatibleDC(None);
    if dc.is_invalid() {
        return None;
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 1,
            biHeight: -1,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let bitmap =
        CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
    if bitmap.is_invalid() || bits.is_null() {
        let _ = DeleteDC(dc);
        return None;
    }
    let old_bitmap = SelectObject(dc, bitmap.into());
    Some(MeasureDc {
        dc,
        bitmap,
        old_bitmap,
    })
}

fn measured_advance(text: &str, family: &str, physical: i32, weight: i32) -> Option<i32> {
    let key = MeasureKey {
        family: family.to_string(),
        physical,
        weight,
        text: text.to_string(),
    };
    if let Some(cached) = ADVANCE_CACHE.with(|cache| cache.borrow().get(&key)) {
        return Some(cached);
    }
    let advance = with_measure_dc(|measure| draw_advance(measure, text, family, physical, weight))?;
    ADVANCE_CACHE.with(|cache| cache.borrow_mut().insert(key, advance));
    Some(advance)
}

#[cfg(test)]
pub(super) fn physical_advance(
    text: &str,
    family: &str,
    physical: i32,
    weight: i32,
) -> Option<i32> {
    measured_advance(text, family, physical, weight)
}

fn draw_advance(
    measure: &MeasureDc,
    text: &str,
    family: &str,
    physical: i32,
    weight: i32,
) -> Option<i32> {
    unsafe {
        let font_name: Vec<u16> = family.encode_utf16().chain(Some(0)).collect();
        let font = CreateFontW(
            -physical,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            windows::core::PCWSTR::from_raw(font_name.as_ptr()),
        );
        if font.is_invalid() {
            return None;
        }
        let old_font = SelectObject(measure.dc, font.into());
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let drawn_height = DrawTextW(
            measure.dc,
            &mut wide,
            &mut rect,
            DRAW_TEXT_FORMAT(DT_MEASURE),
        );
        SelectObject(measure.dc, old_font);
        let _ = DeleteObject(font.into());
        (drawn_height > 0).then_some(rect.right - rect.left)
    }
}

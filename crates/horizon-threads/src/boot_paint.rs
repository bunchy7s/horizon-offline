//! The game window's boot screen before wgpu's first frame: the renderer's
//! device and pipelines take seconds to build, and winit's window sits white
//! until a frame is presented. The game's boot design (`ui_theme::draw_boot`:
//! pure black, the white wordmark, the console's last lines at the bottom
//! left, wrapping word-wise), painted with GDI and replaced by the wgpu frames
//! when the renderer starts drawing.

use std::sync::Once;

use windows_sys::Win32::Foundation::{COLORREF, HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush,
    DEFAULT_CHARSET, DT_CALCRECT, DT_CENTER, DT_LEFT, DT_NOCLIP, DT_SINGLELINE, DT_VCENTER,
    DT_WORDBREAK, DeleteObject, DrawTextW, FF_DONTCARE, FillRect, GetDC, OUT_DEFAULT_PRECIS,
    ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

/// The wordmark on the pure black.
const WORDMARK: COLORREF = 0x00ff_ffff;
/// The console's lines, `0x9BA1A6` in GDI's `0x00bbggrr`.
const CONSOLE: COLORREF = 0x00a6_a19b;

/// GDI's font table is global to the process: the bundled faces register once
/// across every paint.
static FACES: Once = Once::new();

/// Paints the client area of the window at `hwnd` black, the wordmark centred
/// and the console's last lines at the bottom left — the same look as the
/// game's `ui_theme::draw_boot`, which takes over when the renderer presents
/// its first frame.
///
/// `semibold` and `mono` are the faces' TTF bytes (`Geist SemiBold`,
/// `Geist Mono`), which must outlive the process: `AddFontMemResourceEx`
/// keeps referencing them. Repainting is safe and cheap (a background thread
/// may call this as the console's lines arrive).
pub fn paint(hwnd: isize, semibold: &[u8], mono: &[u8], logs: &[String]) {
    let hwnd = hwnd as HWND;
    FACES.call_once(|| {
        for font in [semibold, mono] {
            let count = 0u32;
            unsafe {
                AddFontMemResourceEx(
                    font.as_ptr().cast(),
                    font.len() as u32,
                    std::ptr::null(),
                    &count,
                );
            }
        }
    });
    unsafe {
        let hdc = GetDC(hwnd);
        if hdc.is_null() {
            return;
        }
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        GetClientRect(hwnd, &mut rect);
        let (w, h) = (
            (rect.right - rect.left) as f32,
            (rect.bottom - rect.top) as f32,
        );
        // The pure black clear.
        let brush = CreateSolidBrush(0);
        FillRect(hdc, &rect, brush);
        let _ = DeleteObject(brush);
        // The type scale of the game's `Ui::new`: physical pixels per dp,
        // then the sizes in dp (the wordmark at `DISPLAY`, the console at
        // `MICRO` with 1.5 line heights).
        let density = (w / 1040.0).min(h / 700.0).clamp(1.0, 4.0);
        let size = ((w / density / 16.0).clamp(40.0, 96.0) * density).round();
        let line = 11.0 * density * 1.5;
        let pad = 16.0 * density;
        let face = |px: i32, weight: i32, name: &str| {
            let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            CreateFontW(
                -px,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                FF_DONTCARE as u32,
                name.as_ptr(),
            )
        };
        // The wordmark with its tagline, centred.
        let small = (size * 0.25).clamp(11.0 * density, 18.0 * density);
        let block = size * 1.45 + small * 0.6 + small * 1.5;
        let top = (h - block) / 2.0;
        let mark = face(size as i32, 600, "Geist SemiBold");
        let old_mark = SelectObject(hdc, mark);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, WORDMARK);
        let text: Vec<u16> = "Horizon".encode_utf16().collect();
        let mut mark_rect = RECT {
            left: 0,
            top: top as i32,
            right: w as i32,
            bottom: (top + size * 1.45) as i32,
        };
        DrawTextW(
            hdc,
            text.as_ptr(),
            text.len() as i32,
            &mut mark_rect,
            DT_CENTER | DT_SINGLELINE | DT_NOCLIP | DT_VCENTER,
        );
        let tagline = face(small as i32, 400, "Geist Mono");
        let old_tagline = SelectObject(hdc, tagline);
        SetTextColor(hdc, CONSOLE);
        let tagline_text: Vec<u16> = "Rust based minecraft client".encode_utf16().collect();
        let tagline_top = top + size * 1.45 + small * 0.6;
        let mut tagline_rect = RECT {
            left: 0,
            top: tagline_top as i32,
            right: w as i32,
            bottom: (tagline_top + small * 1.5) as i32,
        };
        DrawTextW(
            hdc,
            tagline_text.as_ptr(),
            tagline_text.len() as i32,
            &mut tagline_rect,
            DT_CENTER | DT_SINGLELINE | DT_NOCLIP | DT_VCENTER,
        );
        // The console's last lines at the bottom left, the newest at the
        // bottom: long lines wrap word-wise and the block grows upwards, as
        // the game's `draw_boot_log` draws it (capped at 24 sub-lines).
        let console = face(line as i32, 400, "Geist Mono");
        let old_console = SelectObject(hdc, console);
        SetTextColor(hdc, CONSOLE);
        let max_rows = (((h - pad) / line) as usize).clamp(1, 24);
        let mut y = h - pad;
        let mut drawn = 0;
        'lines: for text in logs.iter().rev() {
            if text.is_empty() {
                continue;
            }
            let wide: Vec<u16> = text.encode_utf16().collect();
            // The wrapped height, measured without drawing (DT_CALCRECT);
            // every row is the face's cell height, `line`.
            let mut rect = RECT {
                left: pad as i32,
                top: 0,
                right: (pad + w / 2.0) as i32,
                bottom: 0,
            };
            DrawTextW(
                hdc,
                wide.as_ptr(),
                wide.len() as i32,
                &mut rect,
                DT_LEFT | DT_WORDBREAK | DT_CALCRECT,
            );
            let height = rect.bottom.max(line as i32);
            let rows = ((height as f32 / line).ceil() as usize).max(1);
            // The block may only grow past the top before anything is drawn
            // (a newest line wider than the whole screen on a tiny window);
            // once something is there, older lines stop.
            if drawn + rows > max_rows && drawn > 0 {
                break 'lines;
            }
            rect.top = ((y - height as f32).max(0.0)) as i32;
            rect.bottom = y as i32;
            DrawTextW(
                hdc,
                wide.as_ptr(),
                wide.len() as i32,
                &mut rect,
                DT_LEFT | DT_WORDBREAK,
            );
            y -= height as f32;
            drawn += rows;
        }
        SelectObject(hdc, old_mark);
        SelectObject(hdc, old_tagline);
        SelectObject(hdc, old_console);
        let _ = DeleteObject(mark);
        let _ = DeleteObject(tagline);
        let _ = DeleteObject(console);
        let _ = ReleaseDC(hwnd, hdc);
    }
}

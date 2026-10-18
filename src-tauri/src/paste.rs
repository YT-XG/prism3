//! 剪贴板「回到上一个焦点并粘贴」。
//!
//! 两个窗口共用同一条链路：
//! 1. 把内容写进系统剪贴板（文本走 `arboard`，**图片自己拼 DIB**——`arboard::set_image`
//!    在 Windows 上会报 `Unknown error while interacting with the clipboard: SetClipboard`，
//!    用户看到的就是「粘贴图片失败」，见 `image_clipboard`），并记下写入后的剪贴板序列号
//!    （采集线程据此跳过，不把「粘贴」当成一次新的复制）；
//! 2. 目标窗口：**追踪线程**持续记录「最近一个不属于本应用的前台窗口」——
//!    它就是用户心里的「上一个焦点」；
//! 3. 若此刻前台是本应用（主窗口点了记录，或面板处于临时聚焦模式），
//!    先用 `SetForegroundWindow` 把焦点还给那个窗口（必要时借 `AttachThreadInput`
//!    绕过前台切换限制）并校验；前台本来就是别的应用时不必抢；
//! 4. `SendInput` 发一次 Ctrl+V。
//!
//! 快捷小窗的面板窗口平时 `focusable: false`，**压根不会抢走焦点**：悬停着点一条记录时
//! 目标应用一直是前台，第 3 步自然跳过——这也是「点一下就能粘回去」在面板里最顺的原因。
//!
//! `HWND` 不实现 `Send`，静态量里存 `isize`（与 `focus.rs` 同一处理）。

#[cfg(not(windows))]
use std::borrow::Cow;

use serde::Serialize;

use crate::error::AppResult;

/// 要写进剪贴板的内容。
#[derive(Debug, Clone)]
pub enum PasteContent {
    Text(String),
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

/// 一次粘贴尝试的结果：即使没粘成功，内容也已经进了剪贴板。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteOutcome {
    pub pasted: bool,
    /// 没能自动粘贴时的原因（前端提示用）。
    pub reason: Option<String>,
}

impl PasteOutcome {
    fn ok() -> Self {
        Self {
            pasted: true,
            reason: None,
        }
    }

    fn skipped(reason: impl Into<String>) -> Self {
        Self {
            pasted: false,
            reason: Some(reason.into()),
        }
    }

    /// 只复制、不粘贴（配置里关掉了「点击即粘贴」）：不是失败，也就没有原因。
    pub fn copied() -> Self {
        Self {
            pasted: false,
            reason: None,
        }
    }
}

/// 把内容写进系统剪贴板（跨平台部分，Windows 与非 Windows 共用）。
///
/// 文本走 `arboard`（`set_text` 一直好用）；图片在 Windows 上自己拼 DIB（见
/// `image_clipboard` 的模块注释）。
fn write_clipboard(content: &PasteContent) -> AppResult<()> {
    match content {
        PasteContent::Text(text) => {
            let mut clipboard = arboard::Clipboard::new()?;
            clipboard.set_text(text.clone())?;
            Ok(())
        }
        PasteContent::Image {
            width,
            height,
            rgba,
        } => write_image(*width, *height, rgba),
    }
}

/// 把一张 RGBA 图片写进系统剪贴板。
#[cfg(windows)]
fn write_image(width: u32, height: u32, rgba: &[u8]) -> AppResult<()> {
    image_clipboard::write(width, height, rgba)
}

/// 非 Windows 仍用 `arboard`（本机只发布 Windows，仅为可编译性保留）。
#[cfg(not(windows))]
fn write_image(width: u32, height: u32, rgba: &[u8]) -> AppResult<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    clipboard.set_image(arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: Cow::Borrowed(rgba),
    })?;
    Ok(())
}

#[cfg(windows)]
mod image_clipboard {
    //! 把 RGBA 像素直接写成 Windows 剪贴板里的位图（`CF_DIBV5` + `CF_DIB`）。
    //!
    //! **为什么不复用 `arboard::set_image`**：它在真机上对任何图片都返回
    //! `Unknown error while interacting with the clipboard: SetClipboard`
    //! （内容是 `GlobalAlloc` 成功、`SetClipboardData` 被系统拒绝），用户看到的就是
    //! 「粘贴失败」。既然采集侧（`clipboard.rs` 的 `capture`）已经自己解 DIB，写入侧
    //! 就对称地自己拼 DIB，字节与格式细节全在自己手里，出问题也能给出具体原因。
    //!
    //! **两种格式都放**，覆盖两类程序：
    //! - `CF_DIBV5`：`BITMAPV5HEADER` + 32 bpp + `BI_RGB` + `0xFF000000` alpha 掩码。
    //!   Chrome / Electron 就是这么放的（它们认那个「未使用」的第 4 字节当 alpha），
    //!   本仓库的采集侧也专门为这个形态补 alpha（`clipboard.rs` 的 `patch_alpha_from_dib`）；
    //! - `CF_DIB`：`BITMAPINFOHEADER` + 32 bpp `BI_RGB`（第 4 字节照旧塞 alpha），
    //!   画图 / 记事本一类只认 40 字节头的程序认这个。
    //!
    //! 刻意**不用 `BI_BITFIELDS`**：真把掩码写在 V5 头里的写法（这是 Windows 文档给 V4/V5
    //! 头的形态）会让 `image` crate 的 BMP 解码器以为头后面还跟着 12 字节掩码、从而错位；
    //! 而补齐那 12 字节又会让 Windows 以为像素从 136 字节处开始。`BI_RGB` + alpha 掩码
    //! 是两边都对的写法（Chrome / Electron 的实际做法）。
    //!
    //! 像素一律**自下而上**（高度取正值），这是 Windows DIB 的约定；RGB 顺序是 BGRA。

    use std::time::Duration;

    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    use crate::error::{AppError, AppResult};

    /// 标准位图格式号：`CF_DIB` = 设备无关位图，`CF_DIBV5` = 带 alpha 掩码的 V5 头。
    const CF_DIB: u32 = 8;
    const CF_DIBV5: u32 = 17;
    /// `BITMAPINFOHEADER` / `BITMAPV5HEADER` 的长度。
    const INFO_HEADER_LEN: usize = 40;
    const V5_HEADER_LEN: usize = 124;
    /// `LCS_WINDOWS_COLOR_SPACE`（'Win '）：端点 / 伽马字段忽略。
    const LCS_WINDOWS_COLOR_SPACE: u32 = 0x5769_6E20;
    /// 打开剪贴板的重试次数与间隔（别的进程可能正占着它）。
    const OPEN_ATTEMPTS: u32 = 5;
    const OPEN_RETRY_DELAY: Duration = Duration::from_millis(20);

    /// 写一张 RGBA 图片进系统剪贴板。
    pub(super) fn write(width: u32, height: u32, rgba: &[u8]) -> AppResult<()> {
        if width == 0 || height == 0 {
            return Err(AppError::Other("图片尺寸为 0，无法写入剪贴板".into()));
        }
        let expected = width as usize * height as usize * 4;
        if rgba.len() != expected {
            return Err(AppError::Other(format!(
                "图片像素数据长度不匹配（{} 字节，应为 {expected}）",
                rgba.len()
            )));
        }

        let v5 = dib_v5(width, height, rgba);
        let legacy = dib(width, height, rgba);

        let Some(_guard) = open() else {
            return Err(AppError::Other(
                "打开剪贴板失败（可能有别的程序正占用剪贴板），请稍后再试".into(),
            ));
        };

        // SAFETY: 剪贴板已打开；`EmptyClipboard` 只清空当前内容。
        unsafe {
            EmptyClipboard().map_err(|err| AppError::Other(format!("清空剪贴板失败：{err}")))?;
        }
        set_bytes(CF_DIBV5, &v5)?;
        // 老程序只认 `CF_DIB`；它失败不影响已经写进去的 V5 那份（现代程序照样能粘）。
        if let Err(err) = set_bytes(CF_DIB, &legacy) {
            log::warn!("写入 CF_DIB 失败（CF_DIBV5 已写入，现代程序仍可粘贴）：{err}");
        }
        Ok(())
    }

    /// 打开剪贴板；拿到守卫后离开作用域即关闭，绝不把它一直占着。
    fn open() -> Option<ClipboardGuard> {
        for attempt in 0..OPEN_ATTEMPTS {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Some(ClipboardGuard);
            }
            if attempt + 1 < OPEN_ATTEMPTS {
                std::thread::sleep(OPEN_RETRY_DELAY);
            }
        }
        None
    }

    struct ClipboardGuard;

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            let _ = unsafe { CloseClipboard() };
        }
    }

    /// 把一块内存挂到指定剪贴板格式上。成功后所有权归系统，**绝不能再自己释放**。
    fn set_bytes(format: u32, bytes: &[u8]) -> AppResult<()> {
        // SAFETY: 分配到写入的全程在本函数内经手；只在成功后把句柄交出。
        unsafe {
            let hglobal = GlobalAlloc(GMEM_MOVEABLE, bytes.len())
                .map_err(|err| AppError::Other(format!("分配剪贴板内存失败：{err}")))?;
            let pointer = GlobalLock(hglobal);
            if pointer.is_null() {
                free(hglobal);
                return Err(AppError::Other("锁定剪贴板内存失败".into()));
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer as *mut u8, bytes.len());
            let _ = GlobalUnlock(hglobal);
            if let Err(err) = SetClipboardData(format, Some(HANDLE(hglobal.0))) {
                free(hglobal);
                return Err(AppError::Other(format!("写入剪贴板数据失败：{err}")));
            }
        }
        Ok(())
    }

    /// 释放一块没交给系统的剪贴板内存。
    fn free(hglobal: HGLOBAL) {
        let _ = unsafe { GlobalFree(Some(hglobal)) };
    }

    /// `CF_DIBV5` 的内容：124 字节 V5 头（32 bpp、`BI_RGB`、alpha 掩码）+ 自下而上的像素。
    ///
    /// `bV5Compression` 故意留 `BI_RGB`（0，即不写）：alpha 走掩码字段 + 第 4 字节，
    /// 写 `BI_BITFIELDS` 反而会让 BMP 解码器以为头后面还有 12 字节掩码（见模块注释）。
    fn dib_v5(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let pixels = bgra_bottom_up(width, height, rgba);
        let mut out = vec![0u8; V5_HEADER_LEN + pixels.len()];
        out[0..4].copy_from_slice(&(V5_HEADER_LEN as u32).to_le_bytes());
        out[4..8].copy_from_slice(&(width as i32).to_le_bytes());
        out[8..12].copy_from_slice(&(height as i32).to_le_bytes());
        out[12..14].copy_from_slice(&1u16.to_le_bytes()); // bV5Planes
        out[14..16].copy_from_slice(&32u16.to_le_bytes()); // bV5BitCount
        out[20..24].copy_from_slice(&(pixels.len() as u32).to_le_bytes()); // bV5SizeImage
        out[40..44].copy_from_slice(&0x00FF_0000u32.to_le_bytes()); // bV5RedMask
        out[44..48].copy_from_slice(&0x0000_FF00u32.to_le_bytes()); // bV5GreenMask
        out[48..52].copy_from_slice(&0x0000_00FFu32.to_le_bytes()); // bV5BlueMask
        out[52..56].copy_from_slice(&0xFF00_0000u32.to_le_bytes()); // bV5AlphaMask
        out[56..60].copy_from_slice(&LCS_WINDOWS_COLOR_SPACE.to_le_bytes());
        out[V5_HEADER_LEN..].copy_from_slice(&pixels);
        out
    }

    /// `CF_DIB` 的内容：40 字节 `BITMAPINFOHEADER`（32 bpp `BI_RGB`）+ 像素。
    ///
    /// `biCompression` 同样留 `BI_RGB`（0）：第 4 字节按规范「未使用」，但 Chrome / Electron
    /// 拿它当 alpha，所以像素里照旧塞 alpha。
    fn dib(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let pixels = bgra_bottom_up(width, height, rgba);
        let mut out = vec![0u8; INFO_HEADER_LEN + pixels.len()];
        out[0..4].copy_from_slice(&(INFO_HEADER_LEN as u32).to_le_bytes());
        out[4..8].copy_from_slice(&(width as i32).to_le_bytes());
        out[8..12].copy_from_slice(&(height as i32).to_le_bytes());
        out[12..14].copy_from_slice(&1u16.to_le_bytes()); // biPlanes
        out[14..16].copy_from_slice(&32u16.to_le_bytes()); // biBitCount
        out[20..24].copy_from_slice(&(pixels.len() as u32).to_le_bytes()); // biSizeImage
        out[INFO_HEADER_LEN..].copy_from_slice(&pixels);
        out
    }

    /// RGBA（自上而下）→ BGRA（自下而上），即 DIB 的像素排布。
    fn bgra_bottom_up(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let stride = width as usize * 4;
        let rows = height as usize;
        let mut out = vec![0u8; stride * rows];
        for row in 0..rows {
            let source = &rgba[row * stride..(row + 1) * stride];
            let target_row = rows - 1 - row;
            let target = &mut out[target_row * stride..(target_row + 1) * stride];
            for (source_pixel, target_pixel) in source
                .as_chunks::<4>()
                .0
                .iter()
                .zip(target.as_chunks_mut::<4>().0.iter_mut())
            {
                target_pixel[0] = source_pixel[2];
                target_pixel[1] = source_pixel[1];
                target_pixel[2] = source_pixel[0];
                target_pixel[3] = source_pixel[3];
            }
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn u32_at(bytes: &[u8], offset: usize) -> u32 {
            u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ])
        }

        #[test]
        fn dib_v5_header_describes_32bpp_bi_rgb_with_alpha_mask() {
            let rgba = [255u8, 0, 0, 128, 0, 255, 0, 255]; // 2 像素：半透明红、不透明绿
            let bytes = dib_v5(2, 1, &rgba);
            assert_eq!(bytes.len(), V5_HEADER_LEN + 8);
            assert_eq!(u32_at(&bytes, 0), 124);
            assert_eq!(u32_at(&bytes, 4), 2);
            assert_eq!(u32_at(&bytes, 8), 1);
            assert_eq!(u32_at(&bytes, 14) & 0xFFFF, 32);
            // `BI_RGB`：写 `BI_BITFIELDS` 会让 BMP 解码器以为后面还有 12 字节掩码。
            assert_eq!(u32_at(&bytes, 16), 0);
            assert_eq!(u32_at(&bytes, 20), 8);
            assert_eq!(u32_at(&bytes, 40), 0x00FF_0000);
            assert_eq!(u32_at(&bytes, 44), 0x0000_FF00);
            assert_eq!(u32_at(&bytes, 48), 0x0000_00FF);
            assert_eq!(u32_at(&bytes, 52), 0xFF00_0000);
            assert_eq!(u32_at(&bytes, 56), LCS_WINDOWS_COLOR_SPACE);
            // 像素是 BGRA，自上而下压成自下而上；高度 1 时顺序不变。
            assert_eq!(&bytes[V5_HEADER_LEN..], &[0, 0, 255, 128, 0, 255, 0, 255]);
        }

        #[test]
        fn dib_header_is_40_bytes_bi_rgb_32bpp() {
            let rgba = [10u8, 20, 30, 255];
            let bytes = dib(1, 1, &rgba);
            assert_eq!(bytes.len(), INFO_HEADER_LEN + 4);
            assert_eq!(u32_at(&bytes, 0), 40);
            assert_eq!(u32_at(&bytes, 4), 1);
            assert_eq!(u32_at(&bytes, 8), 1);
            assert_eq!(u32_at(&bytes, 14) & 0xFFFF, 32);
            assert_eq!(u32_at(&bytes, 16), 0); // BI_RGB
            assert_eq!(u32_at(&bytes, 20), 4); // biSizeImage
            assert_eq!(&bytes[INFO_HEADER_LEN..], &[30, 20, 10, 255]);
        }

        #[test]
        fn pixel_rows_are_written_bottom_up() {
            // 2×2：第一行红、第二行绿（RGBA，自上而下）。
            let rgba = [
                255, 0, 0, 255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255,
            ];
            let pixels = bgra_bottom_up(2, 2, &rgba);
            assert_eq!(
                pixels,
                vec![
                    0, 255, 0, 255, 0, 255, 0, 255, // 第二行（画面下方）先写
                    0, 0, 255, 255, 0, 0, 255, 255, // 第一行后写
                ]
            );
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use tauri::{AppHandle, Manager};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        VK_CONTROL,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow,
    };

    use super::{write_clipboard, PasteContent, PasteOutcome};

    /// 追踪「上一个焦点」的轮询间隔。
    const TRACK_INTERVAL: Duration = Duration::from_millis(250);
    /// 归还焦点后的等待：前台切换不是瞬时的，立即发按键会打到错误的窗口。
    const ACTIVATE_DELAY: Duration = Duration::from_millis(40);
    /// 归还焦点的最大等待（40ms × 8 ≈ 320ms）。
    const ACTIVATE_ATTEMPTS: u32 = 8;
    /// `V` 键的虚拟键码。
    const VK_V: VIRTUAL_KEY = VIRTUAL_KEY(0x56);

    /// 最近一个**不属于本应用**的前台窗口。
    static LAST_EXTERNAL: Mutex<Option<isize>> = Mutex::new(None);
    /// 追踪线程的运行标志。
    static TRACKER_RUNNING: AtomicBool = AtomicBool::new(false);
    /// 「是否处于启用状态」——线程退出时据此决定要不要把自己再拉起来。
    static TRACKER_WANTED: AtomicBool = AtomicBool::new(false);

    fn lock() -> std::sync::MutexGuard<'static, Option<isize>> {
        LAST_EXTERNAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 丢弃记录的「上一个焦点」（功能关闭时调用）。
    pub fn forget() {
        *lock() = None;
    }

    /// 起追踪线程（幂等）。剪贴板历史启用期间常驻，开销只是一次极轻量的句柄查询。
    pub fn start_tracker(app: &AppHandle) {
        TRACKER_WANTED.store(true, Ordering::SeqCst);
        if TRACKER_RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        spawn_tracker(app.clone());
    }

    fn spawn_tracker(app: AppHandle) {
        std::thread::spawn(move || {
            while TRACKER_WANTED.load(Ordering::SeqCst) {
                if let Some(foreground) = external_foreground(&app) {
                    *lock() = Some(foreground);
                }
                std::thread::sleep(TRACK_INTERVAL);
            }

            TRACKER_RUNNING.store(false, Ordering::SeqCst);
            // 退出前再确认一次：刚置 false 的瞬间可能又被启用。此时本线程已是「老线程」，
            // 得有人接手；`swap` 保证「接手」在两个线程里只有一个做得成，不会留下两个追踪线程。
            if TRACKER_WANTED.load(Ordering::SeqCst)
                && !TRACKER_RUNNING.swap(true, Ordering::SeqCst)
            {
                spawn_tracker(app);
            }
        });
    }

    /// 停追踪线程（幂等）。
    pub fn stop_tracker() {
        TRACKER_WANTED.store(false, Ordering::SeqCst);
        forget();
    }

    /// 写剪贴板并返回写入后的序列号（供采集线程跳过自己这次写入）。
    pub fn copy(content: &PasteContent) -> crate::error::AppResult<Option<u32>> {
        write_clipboard(content)?;
        // SAFETY: 只读查询序列号，无副作用。
        let seq = unsafe { GetClipboardSequenceNumber() };
        Ok((seq != 0).then_some(seq))
    }

    /// 把焦点还给「上一个焦点」并发一次 Ctrl+V。
    pub fn paste(app: &AppHandle) -> PasteOutcome {
        let Some(target) = *lock() else {
            return PasteOutcome::skipped("没有可粘贴的目标窗口（内容已复制到剪贴板）");
        };
        let target = HWND(target as *mut c_void);
        // SAFETY: 只读查询窗口可见性。
        if !unsafe { IsWindowVisible(target).as_bool() } {
            return PasteOutcome::skipped("上一个窗口已关闭（内容已复制到剪贴板）");
        }

        // 前台本来就是别的应用（面板平时不抢焦点）时不必抢，直接发按键。
        if foreground_is_ours(app) && !activate(target) {
            return PasteOutcome::skipped("无法把焦点还给上一个窗口（内容已复制到剪贴板）");
        }

        if !send_paste_keys() {
            return PasteOutcome::skipped("发送粘贴按键失败（内容已复制到剪贴板）");
        }
        PasteOutcome::ok()
    }

    /// 当前前台窗口是否属于本应用。
    fn foreground_is_ours(app: &AppHandle) -> bool {
        // SAFETY: 只读查询当前前台窗口。
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0.is_null() {
            return false;
        }
        app.webview_windows()
            .values()
            .filter_map(|window| window.hwnd().ok())
            .any(|hwnd| hwnd.0 == foreground.0)
    }

    /// 最近一个不属于本应用的前台窗口。
    fn external_foreground(app: &AppHandle) -> Option<isize> {
        // SAFETY: 只读查询当前前台窗口。
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0.is_null() {
            return None;
        }
        if foreground_is_ours(app) {
            return None;
        }
        Some(foreground.0 as isize)
    }

    /// 把目标窗口切到前台并**校验**结果。
    ///
    /// `SetForegroundWindow` 有准入规则：跨进程切换常常被系统拒绝，因此先
    /// `AttachThreadInput` 把输入队列挂到目标线程上（这是绕过该限制的标准做法），
    /// 切换后再轮询确认前台真的变了，避免把 Ctrl+V 发到错误窗口。
    fn activate(target: HWND) -> bool {
        // SAFETY: 全部是只读查询 + 前台切换，句柄来自本进程记录的系统窗口句柄。
        unsafe {
            let current_thread = GetCurrentThreadId();
            let target_thread = GetWindowThreadProcessId(target, None);
            let attached = target_thread != 0
                && target_thread != current_thread
                && AttachThreadInput(current_thread, target_thread, true).as_bool();

            let _ = SetForegroundWindow(target);

            if attached {
                let _ = AttachThreadInput(current_thread, target_thread, false);
            }

            for _ in 0..ACTIVATE_ATTEMPTS {
                if GetForegroundWindow() == target {
                    return true;
                }
                std::thread::sleep(ACTIVATE_DELAY);
            }
            GetForegroundWindow() == target
        }
    }

    fn key_input(key: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// 发一次 Ctrl+V（按下 Ctrl → 按下 V → 松开 V → 松开 Ctrl）。
    fn send_paste_keys() -> bool {
        let inputs = [
            key_input(VK_CONTROL, false),
            key_input(VK_V, false),
            key_input(VK_V, true),
            key_input(VK_CONTROL, true),
        ];
        // SAFETY: 使用固定长度的合法输入数组调用 SendInput。
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        sent == inputs.len() as u32
    }
}

#[cfg(not(windows))]
mod imp {
    use tauri::AppHandle;

    use super::{write_clipboard, PasteContent, PasteOutcome};

    /// 非 Windows 不做前台窗口追踪（本项目只发布 Windows，仅为可编译性保留）。
    pub fn forget() {}

    pub fn start_tracker(_app: &AppHandle) {}

    pub fn stop_tracker() {}

    pub fn copy(content: &PasteContent) -> crate::error::AppResult<Option<u32>> {
        write_clipboard(content)?;
        Ok(None)
    }

    pub fn paste(_app: &AppHandle) -> PasteOutcome {
        PasteOutcome::skipped("当前平台不支持自动粘贴（内容已复制到剪贴板）")
    }
}

pub use imp::{copy, paste, start_tracker, stop_tracker};

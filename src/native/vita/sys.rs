//! Origine : nouveau fichier, ajouté au miniquad 0.4.8 vendoré (crates.io, git cc6f8d1f).
//! Version : miniquad 0.4.8 + backend PS Vita (spike A, jalon 3, Alice Vita).
//! Modifications pour PS Vita : tout le fichier.
//!
//! Déclarations FFI minimales (écrites à la main, pas de bindgen ni de vitasdk-sys) :
//! vitaGL (contexte, swap, GetProcAddress), SceCtrl, SceTouch, SceKernel.
//! Prototypes et structures recopiés de VitaSDK 2026.08 :
//! `$VITASDK/arm-vita-eabi/include/vitaGL.h` (vitaGL dca4b9d),
//! `psp2common/ctrl.h`, `psp2/touch.h`, `psp2/kernel/processmgr.h`.
//! Free memory information: `psp2/kernel/sysmem.h`.
//!
//! Liaison : tout est statique (la cible n'a pas de liaison dynamique). La liste et l'ordre
//! sont ceux validés par spike-a/triangle/src/vgl.rs (jalon 1), plus SceCtrl/SceTouch/SceLibKernel
//! pour les entrées et le temps. `-bundle` : les .a ne sont PAS recopiés dans la rlib de miniquad,
//! ils sont passés tels quels à l'édition de liens finale (arm-vita-eabi-gcc les trouve dans
//! $VITASDK/arm-vita-eabi/lib), exactement comme dans le crate binaire `triangle`.
#![allow(non_snake_case, non_camel_case_types, dead_code)]

use core::ffi::{c_char, c_int, c_void};

// vitaGL dca4b9d: VGL_MEM_SLOW does not exist; PHYCONT is enum value 2.
pub const VGL_MEM_VRAM: c_int = 0;
pub const VGL_MEM_RAM: c_int = 1;
pub const VGL_MEM_PHYCONT: c_int = 2;
pub const VGL_MEM_BUDGET: c_int = 3;
pub const VGL_MEM_EXTERNAL: c_int = 4;

// newlib malloc.h: ten size_t fields, returned by value (ARM32 size_t = u32).
#[repr(C)]
#[derive(Default)]
pub struct MallInfo {
    pub arena: usize,
    pub ordblks: usize,
    pub smblks: usize,
    pub hblks: usize,
    pub hblkhd: usize,
    pub usmblks: usize,
    pub fsmblks: usize,
    pub uordblks: usize,
    pub fordblks: usize,
    pub keepcost: usize,
}

pub type GLboolean = u8;
pub const GL_FALSE: GLboolean = 0;
pub const GL_TRUE: GLboolean = 1;

// SceGxmMultisampleMode (psp2/gxm.h)
pub const SCE_GXM_MULTISAMPLE_NONE: c_int = 0;
pub const SCE_GXM_MULTISAMPLE_2X: c_int = 1;
pub const SCE_GXM_MULTISAMPLE_4X: c_int = 2;

#[link(name = "vitaGL", kind = "static", modifiers = "-bundle")]
#[link(name = "vitashark", kind = "static", modifiers = "-bundle")]
#[link(name = "SceShaccCgExt", kind = "static", modifiers = "-bundle")]
#[link(name = "mathneon", kind = "static", modifiers = "-bundle")]
#[link(name = "SceShaccCg_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "taihen_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "SceKernelDmacMgr_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "SceGxm_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "SceDisplay_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "SceSysmodule_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "SceCommonDialog_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "SceAppUtil_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "SceIme_stub", kind = "static", modifiers = "-bundle")]
// Requis par le vitaGL actuel (dca4b9d) : sceAppMgrGetBudgetInfo, sceSharedFb* (voir spike-a/JALON1.md).
#[link(name = "SceAppMgr_stub", kind = "static", modifiers = "-bundle")]
// Entrées et temps (backend miniquad).
#[link(name = "SceCtrl_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "SceTouch_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "SceLibKernel_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "SceSysmem_stub", kind = "static", modifiers = "-bundle")]
#[link(name = "stdc++")]
extern "C" {
    // --- vitaGL (vitaGL.h) ---
    /// vitaGL init function with customizable resolution, RAM threshold and MSAA setup.
    pub fn vglInitExtended(
        legacy_pool_size: c_int,
        width: c_int,
        height: c_int,
        ram_threshold: c_int,
        msaa: c_int,
    ) -> GLboolean;
    /// Perform a display buffer swap (équivalent de eglSwapBuffers).
    pub fn vglSwapBuffers(has_commondialog: GLboolean);
    /// Get a GL function address given a function name (lookup.c). NULL si non implémentée.
    pub fn vglGetProcAddress(name: *const c_char) -> *mut c_void;
    /// Simplified function to enable or disable V-Sync.
    pub fn vglWaitVblankStart(enable: GLboolean);
    /// Change the total memory to use for internal circular pools. Default 32 MiB. Avant vglInit*.
    pub fn vglSetCircularPoolSize(size: u32);

    /// Free bytes in a vitaGL pool. EXTERNAL returns 0 in dca4b9d (not measured).
    pub fn vglMemFree(memory_type: c_int) -> usize;
    // --- newlib (malloc.h) ---
    pub fn mallinfo() -> MallInfo;

    // --- SceCtrl (psp2/ctrl.h) ---
    pub fn sceCtrlSetSamplingMode(mode: c_int) -> c_int;
    pub fn sceCtrlPeekBufferPositive(
        port: c_int,
        pad_data: *mut SceCtrlData,
        count: c_int,
    ) -> c_int;

    // --- SceTouch (psp2/touch.h) ---
    pub fn sceTouchSetSamplingState(port: u32, state: u32) -> c_int;
    pub fn sceTouchPeek(port: u32, data: *mut SceTouchData, n_bufs: u32) -> c_int;
    pub fn sceTouchGetPanelInfo(port: u32, info: *mut SceTouchPanelInfo) -> c_int;

    // --- SceKernel (psp2/kernel/processmgr.h) ---
    /// Temps du processus en microsecondes (monotone).
    pub fn sceKernelGetProcessTimeWide() -> u64;
    // --- SceSysmem (psp2/kernel/sysmem.h) ---
    /// Free memory in bytes. Set info.size before calling; returns 0 on success, < 0 on error.
    pub fn sceKernelGetFreeMemorySize(info: *mut SceKernelFreeMemorySizeInfo) -> c_int;
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct SceKernelFreeMemorySizeInfo {
    pub size: c_int,
    pub size_user: c_int,
    pub size_cdram: c_int,
    pub size_phycont: c_int,
}

const _: () = assert!(core::mem::size_of::<SceKernelFreeMemorySizeInfo>() == 0x10);

// SceCtrlPadInputMode
pub const SCE_CTRL_MODE_DIGITAL: c_int = 0;
pub const SCE_CTRL_MODE_ANALOG: c_int = 1;
pub const SCE_CTRL_MODE_ANALOG_WIDE: c_int = 2;

// SceCtrlButtons (psp2common/ctrl.h)
pub const SCE_CTRL_SELECT: u32 = 0x0000_0001;
pub const SCE_CTRL_L3: u32 = 0x0000_0002;
pub const SCE_CTRL_R3: u32 = 0x0000_0004;
pub const SCE_CTRL_START: u32 = 0x0000_0008;
pub const SCE_CTRL_UP: u32 = 0x0000_0010;
pub const SCE_CTRL_RIGHT: u32 = 0x0000_0020;
pub const SCE_CTRL_DOWN: u32 = 0x0000_0040;
pub const SCE_CTRL_LEFT: u32 = 0x0000_0080;
/// Gâchette gauche de la Vita (sur la console, L et R remontent en LTRIGGER/RTRIGGER).
pub const SCE_CTRL_LTRIGGER: u32 = 0x0000_0100;
pub const SCE_CTRL_RTRIGGER: u32 = 0x0000_0200;
/// L1/R1 : uniquement manettes externes (PS TV).
pub const SCE_CTRL_L1: u32 = 0x0000_0400;
pub const SCE_CTRL_R1: u32 = 0x0000_0800;
pub const SCE_CTRL_TRIANGLE: u32 = 0x0000_1000;
pub const SCE_CTRL_CIRCLE: u32 = 0x0000_2000;
pub const SCE_CTRL_CROSS: u32 = 0x0000_4000;
pub const SCE_CTRL_SQUARE: u32 = 0x0000_8000;
/// Entrée interceptée par une autre application (menu PS, LiveArea…).
pub const SCE_CTRL_INTERCEPTED: u32 = 0x0001_0000;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct SceCtrlData {
    pub time_stamp: u64,
    pub buttons: u32,
    pub lx: u8,
    pub ly: u8,
    pub rx: u8,
    pub ry: u8,
    pub up: u8,
    pub right: u8,
    pub down: u8,
    pub left: u8,
    pub lt: u8,
    pub rt: u8,
    pub l1: u8,
    pub r1: u8,
    pub triangle: u8,
    pub circle: u8,
    pub cross: u8,
    pub square: u8,
    pub reserved: [u8; 4],
}

// SceTouch
pub const SCE_TOUCH_PORT_FRONT: u32 = 0;
pub const SCE_TOUCH_PORT_BACK: u32 = 1;
pub const SCE_TOUCH_SAMPLING_STATE_START: u32 = 1;
pub const SCE_TOUCH_MAX_REPORT: usize = 8;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct SceTouchReport {
    pub id: u8,
    pub force: u8,
    pub x: i16,
    pub y: i16,
    pub reserved: [u8; 8],
    pub info: u16,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct SceTouchData {
    pub time_stamp: u64,
    pub status: u32,
    pub report_num: u32,
    pub report: [SceTouchReport; SCE_TOUCH_MAX_REPORT],
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct SceTouchPanelInfo {
    pub min_aa_x: i16,
    pub min_aa_y: i16,
    pub max_aa_x: i16,
    pub max_aa_y: i16,
    pub min_disp_x: i16,
    pub min_disp_y: i16,
    pub max_disp_x: i16,
    pub max_disp_y: i16,
    pub min_force: u8,
    pub max_force: u8,
    pub reserved: [u8; 30],
}

// Tailles vérifiées contre les VITASDK_BUILD_ASSERT_EQ des en-têtes.
const _: () = assert!(core::mem::size_of::<SceTouchReport>() == 0x10);
const _: () = assert!(core::mem::size_of::<SceCtrlData>() == 0x20);
const _: () = assert!(core::mem::size_of::<SceTouchData>() == 0x90);
const _: () = assert!(core::mem::size_of::<SceTouchPanelInfo>() == 0x30);

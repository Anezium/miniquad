//! Origine : nouveau fichier, ajouté au miniquad 0.4.8 vendoré (crates.io, git cc6f8d1f).
//! Version : miniquad 0.4.8 + backend PS Vita (spike A, jalon 3, Alice Vita).
//! Modifications pour PS Vita : tout le fichier.
//!
//! Backend PS Vita (`target_os = "vita"`, cible `armv7-sony-vita-newlibeabihf`).
//!
//! Forme calquée sur `native/android.rs` (état + boucle) et `native/linux_x11.rs` (boucle mono-thread,
//! requêtes via le canal `native_requests`, gestion de quit_requested/quit_ordered).
//!
//! - Contexte : vitaGL lié statiquement, `vglInitExtended` (résolution, seuil RAM, MSAA),
//!   `vglSwapBuffers` en fin de frame. Pas d'EGL, pas de fenêtre, pas de surface.
//! - Fonctions GL : `vglGetProcAddress` + quelques shims (voir `gl_loader.rs`).
//! - Entrées : `sceCtrlPeekBufferPositive` + `sceTouchPeek` scrutés à chaque frame (`input.rs`).
//! - Écran fixe (960x544 par défaut), dpi_scale 1.0, high_dpi false.
//! - Fenêtrage, curseur, presse-papiers, clavier virtuel : no-op.
//!
//! Configuration : `miniquad::native::vita::set_config(VitaConfig { .. })` AVANT `miniquad::start`
//! (ou avant la boucle macroquad). Sans appel, `VitaConfig::default()`.

mod gl_loader;
pub mod input;
pub mod sys;

pub use input::{InputConfig, LeftStickMode};
pub use gl_loader::with_dxt5_uploads;

use crate::{event::EventHandler, native::NativeDisplayData};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Instant;

/// Paramètres spécifiques à la Vita (taille d'écran, mémoire vitaGL, entrées).
#[derive(Debug, Clone)]
pub struct VitaConfig {
    /// Résolution du framebuffer vitaGL. 960x544 = écran natif.
    pub width: i32,
    pub height: i32,
    /// Taille du pool « legacy » de vitaGL (immediate mode, glBegin/glEnd). miniquad ne s'en sert
    /// pas : 0 suffit (même valeur que spike-a/triangle `vglInit(0)`).
    pub legacy_pool_size: i32,
    /// RAM utilisateur laissée hors du tas vitaGL (octets), 3e argument de `vglInitExtended`.
    /// vitaGL prend tout le reste de la RAM libre pour ses pools. `vglInit` utilise 16 Mio.
    /// À augmenter si l'allocateur Rust (newlib) manque de mémoire.
    pub ram_threshold: i32,
    /// Taille des pools circulaires de vitaGL (`vglSetCircularPoolSize`), None = défaut vitaGL (32 Mio).
    pub circular_pool_size: Option<u32>,
    /// Journalise le temps de frame moyen sur stderr toutes les N frames (None = jamais).
    pub log_frame_time_every: Option<u32>,
    /// Log kernel, newlib and vitaGL free memory at startup, before the first swap,
    /// on panic, and every N wall-clock seconds on a separate thread (also during loading).
    /// None disables logging; Some(0) uses a one-second interval.
    pub log_memory_every: Option<u32>,
    /// Allocate initially empty stream buffers only when updated, sized to the
    /// uploaded payload. Opt in for renderers such as macroquad that replace
    /// the whole draw payload: bytes beyond the latest update are discarded.
    /// buffer_size still reports the logical maximum; update bounds stay checked.
    pub compact_stream_buffers: bool,
    /// Mapping des entrées.
    pub input: InputConfig,
}

impl Default for VitaConfig {
    fn default() -> Self {
        VitaConfig {
            width: 960,
            height: 544,
            legacy_pool_size: 0,
            ram_threshold: 0x0100_0000,
            circular_pool_size: None,
            log_frame_time_every: None,
            log_memory_every: Some(10),
            compact_stream_buffers: false,
            input: InputConfig::default(),
        }
    }
}

static CONFIG: Mutex<Option<VitaConfig>> = Mutex::new(None);
static COMPACT_STREAM_BUFFERS: AtomicBool = AtomicBool::new(false);

pub(crate) fn compact_stream_buffers() -> bool {
    COMPACT_STREAM_BUFFERS.load(Ordering::Relaxed)
}

/// Fixe la configuration Vita. À appeler avant `miniquad::start`.
pub fn set_config(config: VitaConfig) {
    COMPACT_STREAM_BUFFERS.store(config.compact_stream_buffers, Ordering::Relaxed);
    *CONFIG.lock().unwrap() = Some(config);
}

/// Presse-papiers : la Vita n'en a pas. No-op.
pub struct VitaClipboard;

impl crate::native::Clipboard for VitaClipboard {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _data: &str) {}
}

struct VitaDisplay {
    update_requested: bool,
}

impl VitaDisplay {
    fn process_request(&mut self, request: crate::native::Request) {
        use crate::native::Request::*;
        match request {
            ScheduleUpdate => self.update_requested = true,
            // Pas de fenêtre ni de curseur sur Vita.
            SetCursorGrab(_) | ShowMouse(_) | SetMouseCursor(_) => {}
            SetWindowSize { .. } | SetWindowPosition { .. } | SetFullscreen(_) => {}
            // TODO : clavier système via SceIme / SceCommonDialog (vglSwapBuffers(GL_TRUE)).
            ShowKeyboard(_) => {}
        }
    }
}

fn msaa_mode(sample_count: i32) -> core::ffi::c_int {
    match sample_count {
        i32::MIN..=1 => sys::SCE_GXM_MULTISAMPLE_NONE,
        2..=3 => sys::SCE_GXM_MULTISAMPLE_2X,
        _ => sys::SCE_GXM_MULTISAMPLE_4X,
    }
}

static VGL_READY: AtomicBool = AtomicBool::new(false);

fn log_free_memory(reason: &str) {
    let mut info = sys::SceKernelFreeMemorySizeInfo {
        size: core::mem::size_of::<sys::SceKernelFreeMemorySizeInfo>() as core::ffi::c_int,
        ..Default::default()
    };
    let result = unsafe { sys::sceKernelGetFreeMemorySize(&mut info) };
    let heap = unsafe { sys::mallinfo() };
    let now = unsafe { sys::sceKernelGetProcessTimeWide() };
    // Do not inspect vitaGL's heaps while vglInitExtended is creating them.
    if VGL_READY.load(Ordering::Acquire) {
        let (vram, ram, phycont, budget, external) = unsafe {
            (
                sys::vglMemFree(sys::VGL_MEM_VRAM),
                sys::vglMemFree(sys::VGL_MEM_RAM),
                sys::vglMemFree(sys::VGL_MEM_PHYCONT),
                sys::vglMemFree(sys::VGL_MEM_BUDGET),
                sys::vglMemFree(sys::VGL_MEM_EXTERNAL),
            )
        };
        eprintln!(
            "[miniquad/vita] memory reason={} time_us={} kernel_rc={} user={} cdram={} phycont={} newlib_arena={} newlib_uordblks={} newlib_fordblks={} vgl_VRAM={} vgl_RAM={} vgl_PHYCONT={} vgl_BUDGET={} vgl_EXTERNAL={}",
            reason, now, result, info.size_user, info.size_cdram, info.size_phycont,
            heap.arena, heap.uordblks, heap.fordblks, vram, ram, phycont, budget, external
        );
    } else {
        eprintln!(
            "[miniquad/vita] memory reason={} time_us={} kernel_rc={} user={} cdram={} phycont={} newlib_arena={} newlib_uordblks={} newlib_fordblks={} vgl=not_initialized",
            reason, now, result, info.size_user, info.size_cdram, info.size_phycont,
            heap.arena, heap.uordblks, heap.fordblks
        );
    }
}

pub fn run<F>(conf: &crate::conf::Conf, f: F)
where
    F: 'static + FnOnce() -> Box<dyn EventHandler>,
{
    let cfg = CONFIG.lock().unwrap().take().unwrap_or_default();

    let memory_running = Arc::new(AtomicBool::new(true));
    if let Some(seconds) = cfg.log_memory_every {
        eprintln!("[miniquad/vita] memory: VGL_MEM_SLOW absent in dca4b9d; using VGL_MEM_PHYCONT; vgl_EXTERNAL=0 means not measured by vitaGL");
        log_free_memory("startup");
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log_free_memory("panic");
            previous_hook(info);
        }));
        let running = Arc::clone(&memory_running);
        std::thread::Builder::new()
            .name("vita-memory".into())
            .stack_size(64 * 1024)
            .spawn(move || {
                let interval = std::time::Duration::from_secs(seconds.max(1) as u64);
                let mut deadline = Instant::now() + interval;
                while running.load(Ordering::Relaxed) {
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                    if !running.load(Ordering::Relaxed) {
                        break;
                    }
                    log_free_memory("tick");
                    deadline += interval;
                    if deadline <= Instant::now() {
                        deadline = Instant::now() + interval;
                    }
                }
            })
            .expect("Cannot start Vita memory logger");
    }

    unsafe {
        if let Some(size) = cfg.circular_pool_size {
            sys::vglSetCircularPoolSize(size);
        }
        // vitaGL (source/vgl.c, vglInitWithCustomSizes) returns `res_fallback`: GL_FALSE on a
        // normal init, GL_TRUE when the requested resolution exceeded the display maximum and was
        // clamped. It never reports failure through this value (the only GL_FALSE early return
        // is "already inited"). An earlier revision panicked on GL_FALSE, i.e. on every
        // successful init; that crashed the first on-console run (2026-10-11).
        let res_fallback = sys::vglInitExtended(
            cfg.legacy_pool_size,
            cfg.width,
            cfg.height,
            cfg.ram_threshold,
            msaa_mode(conf.sample_count),
        );
        if res_fallback == sys::GL_TRUE {
            eprintln!(
                "[miniquad/vita] vglInitExtended: {}x{} exceeds the display maximum, vitaGL clamped it",
                cfg.width, cfg.height
            );
        }
        // swap_interval : None ou != 0 → V-Sync.
        let vsync = conf.platform.swap_interval.map_or(true, |i| i != 0);
        sys::vglWaitVblankStart(if vsync { sys::GL_TRUE } else { sys::GL_FALSE });
    }

    VGL_READY.store(true, Ordering::Release);

    crate::native::gl::load_gl_funcs(gl_loader::get_proc_address);

    let (tx, rx) = std::sync::mpsc::channel();
    crate::set_display(NativeDisplayData {
        high_dpi: false,
        dpi_scale: 1.0,
        blocking_event_loop: conf.platform.blocking_event_loop,
        ..NativeDisplayData::new(cfg.width, cfg.height, tx, Box::new(VitaClipboard))
    });

    let mut input = input::InputState::new(cfg.input.clone(), cfg.width as f32, cfg.height as f32);
    let mut display = VitaDisplay {
        update_requested: true,
    };

    let mut event_handler = f();

    let mut frames: u32 = 0;
    let mut first_swap = true;
    let mut window_start = Instant::now();
    let mut phase_us = [0u64; 3];

    while !crate::native_display().try_lock().unwrap().quit_ordered {
        while let Ok(request) = rx.try_recv() {
            display.process_request(request);
        }

        input.poll(&mut *event_handler);

        {
            let d = crate::native_display().try_lock().unwrap();
            if d.quit_requested && !d.quit_ordered {
                drop(d);
                event_handler.quit_requested_event();
                let mut d = crate::native_display().try_lock().unwrap();
                if d.quit_requested {
                    d.quit_ordered = true;
                }
            }
        }

        if !conf.platform.blocking_event_loop || display.update_requested {
            display.update_requested = false;
            let profiling = cfg.log_frame_time_every.is_some();
            let update_start = profiling.then(Instant::now);
            event_handler.update();
            let draw_start = profiling.then(Instant::now);
            event_handler.draw();
            let swap_start = profiling.then(Instant::now);
            if first_swap && cfg.log_memory_every.is_some() {
                log_free_memory("first_swap");
                first_swap = false;
            }
            unsafe { sys::vglSwapBuffers(sys::GL_FALSE) };
            if let (Some(update), Some(draw), Some(swap)) = (update_start, draw_start, swap_start) {
                phase_us[0] += draw.duration_since(update).as_micros() as u64;
                phase_us[1] += swap.duration_since(draw).as_micros() as u64;
                phase_us[2] += swap.elapsed().as_micros() as u64;
            }
        } else {
            // Boucle « bloquante » émulée : pas d'attente d'événement sur Vita, on scrute à ~1 kHz.
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        if let Some(every) = cfg.log_frame_time_every {
            frames += 1;
            if frames >= every.max(1) {
                let elapsed = window_start.elapsed();
                eprintln!(
                    "[miniquad/vita] {} frames, {:.2} ms/frame fps={:.3} update_ms={:.3} draw_ms={:.3} swap_ms={:.3}",
                    frames,
                    elapsed.as_secs_f64() * 1000.0 / frames as f64,
                    frames as f64 / elapsed.as_secs_f64(),
                    phase_us[0] as f64 / frames as f64 / 1000.0,
                    phase_us[1] as f64 / frames as f64 / 1000.0,
                    phase_us[2] as f64 / frames as f64 / 1000.0
                );
                phase_us = [0; 3];
                frames = 0;
                window_start = Instant::now();
            }
        }
    }
    memory_running.store(false, Ordering::Relaxed);
    // Pas de destruction explicite du contexte : vitaGL n'expose pas de vglEnd dans vitaGL.h
    // (dca4b9d). Le retour de main termine le processus.
}

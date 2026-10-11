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

use crate::{event::EventHandler, native::NativeDisplayData};
use std::sync::Mutex;
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
            input: InputConfig::default(),
        }
    }
}

static CONFIG: Mutex<Option<VitaConfig>> = Mutex::new(None);

/// Fixe la configuration Vita. À appeler avant `miniquad::start`.
pub fn set_config(config: VitaConfig) {
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

pub fn run<F>(conf: &crate::conf::Conf, f: F)
where
    F: 'static + FnOnce() -> Box<dyn EventHandler>,
{
    let cfg = CONFIG.lock().unwrap().take().unwrap_or_default();

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
    let mut window_start = Instant::now();

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
            event_handler.update();
            event_handler.draw();
            unsafe { sys::vglSwapBuffers(sys::GL_FALSE) };
        } else {
            // Boucle « bloquante » émulée : pas d'attente d'événement sur Vita, on scrute à ~1 kHz.
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        if let Some(every) = cfg.log_frame_time_every {
            frames += 1;
            if frames >= every.max(1) {
                let elapsed = window_start.elapsed();
                eprintln!(
                    "[miniquad/vita] {} frames, {:.2} ms/frame",
                    frames,
                    elapsed.as_secs_f64() * 1000.0 / frames as f64
                );
                frames = 0;
                window_start = Instant::now();
            }
        }
    }
    // Pas de destruction explicite du contexte : vitaGL n'expose pas de vglEnd dans vitaGL.h
    // (dca4b9d). Le retour de main termine le processus.
}

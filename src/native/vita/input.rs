//! Origine : nouveau fichier, ajouté au miniquad 0.4.8 vendoré (crates.io, git cc6f8d1f).
//! Version : miniquad 0.4.8 + backend PS Vita (spike A, jalon 3, Alice Vita).
//! Modifications pour PS Vita : tout le fichier.
//!
//! Entrées Vita → événements miniquad. Lecture par scrutation à chaque frame
//! (`sceCtrlPeekBufferPositive`, `sceTouchPeek`), différence avec l'état précédent, puis appel
//! direct des callbacks de l'`EventHandler` (pas de thread ni de canal : tout est sur le thread
//! principal, comme la boucle X11).
//!
//! Mapping par défaut (voir `VitaConfig::default` et spike-a/JALON3-PREP.md) :
//!
//! | Vita                 | miniquad                      |
//! |----------------------|-------------------------------|
//! | Croix directionnelle | KeyCode::Up/Down/Left/Right   |
//! | Croix (×)            | KeyCode::Enter                |
//! | Rond (○)             | KeyCode::Backspace            |
//! | Carré (□)            | KeyCode::Space                |
//! | Triangle (△)         | KeyCode::E                    |
//! | START                | KeyCode::Escape               |
//! | SELECT               | KeyCode::Tab                  |
//! | L (gâchette)         | KeyCode::LeftShift (+ keymods.shift) |
//! | R (gâchette)         | KeyCode::LeftControl (+ keymods.ctrl) |
//! | L1 / R1 (PS TV)      | KeyCode::Q / KeyCode::R       |
//! | L3 / R3 (PS TV)      | KeyCode::F / KeyCode::V       |
//! | Stick gauche         | W/A/S/D (seuil) ou curseur souris virtuel |
//! | Stick droit          | raw_mouse_motion (regard caméra) |
//! | Écran tactile avant  | touch_event (Started/Moved/Ended), id = id du rapport Vita |
//!
//! macroquad émule déjà la souris à partir du tactile (`simulate_mouse_with_touch`) :
//! on n'émet donc pas d'événements souris pour le tactile.

use super::sys;
use crate::event::{EventHandler, KeyCode, KeyMods, MouseButton, TouchPhase};

/// Comportement du stick gauche.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LeftStickMode {
    /// Ignoré.
    Disabled,
    /// Quatre touches (haut, bas, gauche, droite) pressées au-delà de `stick_deadzone`.
    Keys {
        up: KeyCode,
        down: KeyCode,
        left: KeyCode,
        right: KeyCode,
    },
    /// Curseur souris virtuel : mouse_motion_event (position absolue) + raw_mouse_motion.
    /// `speed` en pixels par frame à déflexion maximale.
    Mouse { speed: f32 },
}

/// Configuration des entrées, modifiable avant `miniquad::start` via
/// `miniquad::native::vita::set_config`.
#[derive(Debug, Clone)]
pub struct InputConfig {
    /// (masque SCE_CTRL_*, touche miniquad). Plusieurs entrées peuvent viser la même touche.
    pub buttons: Vec<(u32, KeyCode)>,
    pub left_stick: LeftStickMode,
    /// Stick droit → raw_mouse_motion, en unités par frame à déflexion maximale (0 = désactivé).
    pub right_stick_raw_speed: f32,
    /// Zone morte des sticks, sur [0, 1].
    pub stick_deadzone: f32,
    /// En mode souris virtuelle, la croix (×) envoie aussi un clic gauche.
    pub cross_is_left_click: bool,
    /// Lire l'écran tactile avant.
    pub front_touch: bool,
}

impl Default for InputConfig {
    fn default() -> Self {
        InputConfig {
            buttons: vec![
                (sys::SCE_CTRL_UP, KeyCode::Up),
                (sys::SCE_CTRL_DOWN, KeyCode::Down),
                (sys::SCE_CTRL_LEFT, KeyCode::Left),
                (sys::SCE_CTRL_RIGHT, KeyCode::Right),
                (sys::SCE_CTRL_CROSS, KeyCode::Enter),
                (sys::SCE_CTRL_CIRCLE, KeyCode::Backspace),
                (sys::SCE_CTRL_SQUARE, KeyCode::Space),
                (sys::SCE_CTRL_TRIANGLE, KeyCode::E),
                (sys::SCE_CTRL_START, KeyCode::Escape),
                (sys::SCE_CTRL_SELECT, KeyCode::Tab),
                (sys::SCE_CTRL_LTRIGGER, KeyCode::LeftShift),
                (sys::SCE_CTRL_RTRIGGER, KeyCode::LeftControl),
                (sys::SCE_CTRL_L1, KeyCode::Q),
                (sys::SCE_CTRL_R1, KeyCode::R),
                (sys::SCE_CTRL_L3, KeyCode::F),
                (sys::SCE_CTRL_R3, KeyCode::V),
            ],
            left_stick: LeftStickMode::Keys {
                up: KeyCode::W,
                down: KeyCode::S,
                left: KeyCode::A,
                right: KeyCode::D,
            },
            right_stick_raw_speed: 8.0,
            stick_deadzone: 0.25,
            cross_is_left_click: false,
            front_touch: true,
        }
    }
}

#[derive(Clone, Copy)]
struct ActiveTouch {
    id: u8,
    x: f32,
    y: f32,
}

pub(crate) struct InputState {
    cfg: InputConfig,
    screen_w: f32,
    screen_h: f32,
    prev_buttons: u32,
    /// Directions du stick gauche en mode Keys : [haut, bas, gauche, droite].
    prev_stick_keys: [bool; 4],
    keymods: KeyMods,
    cursor: (f32, f32),
    touches: Vec<ActiveTouch>,
    /// Échelle panneau tactile → pixels écran.
    touch_scale: (f32, f32),
    touch_origin: (f32, f32),
    intercepted: bool,
}

impl InputState {
    pub(crate) fn new(cfg: InputConfig, screen_w: f32, screen_h: f32) -> InputState {
        unsafe {
            sys::sceCtrlSetSamplingMode(sys::SCE_CTRL_MODE_ANALOG_WIDE);
        }

        // Le panneau avant rapporte 0..1919 x 0..1087 (le double de 960x544). On le lit si
        // possible pour ne pas coder la valeur en dur.
        let mut touch_scale = (screen_w / 1920.0, screen_h / 1088.0);
        let mut touch_origin = (0.0, 0.0);
        if cfg.front_touch {
            unsafe {
                sys::sceTouchSetSamplingState(
                    sys::SCE_TOUCH_PORT_FRONT,
                    sys::SCE_TOUCH_SAMPLING_STATE_START,
                );
                let mut info = sys::SceTouchPanelInfo::default();
                if sys::sceTouchGetPanelInfo(sys::SCE_TOUCH_PORT_FRONT, &mut info) >= 0 {
                    let w = (info.max_disp_x as f32 - info.min_disp_x as f32) + 1.0;
                    let h = (info.max_disp_y as f32 - info.min_disp_y as f32) + 1.0;
                    if w > 1.0 && h > 1.0 {
                        touch_scale = (screen_w / w, screen_h / h);
                        touch_origin = (info.min_disp_x as f32, info.min_disp_y as f32);
                    }
                }
            }
        }

        InputState {
            cfg,
            screen_w,
            screen_h,
            prev_buttons: 0,
            prev_stick_keys: [false; 4],
            keymods: KeyMods::default(),
            cursor: (screen_w / 2.0, screen_h / 2.0),
            touches: Vec::with_capacity(sys::SCE_TOUCH_MAX_REPORT),
            touch_scale,
            touch_origin,
            intercepted: false,
        }
    }

    /// Lit les entrées et émet les événements. Renvoie true si au moins un événement a été émis.
    pub(crate) fn poll(&mut self, handler: &mut dyn EventHandler) -> bool {
        let mut any = false;

        let mut pad = sys::SceCtrlData::default();
        let ok = unsafe { sys::sceCtrlPeekBufferPositive(0, &mut pad, 1) } >= 0;
        if ok {
            any |= self.process_pad(&pad, handler);
        }

        if self.cfg.front_touch {
            let mut touch = sys::SceTouchData::default();
            let n = unsafe { sys::sceTouchPeek(sys::SCE_TOUCH_PORT_FRONT, &mut touch, 1) };
            if n >= 0 {
                any |= self.process_touch(&touch, handler);
            }
        }

        any
    }

    fn update_mods(&mut self, key: KeyCode, down: bool) {
        match key {
            KeyCode::LeftShift | KeyCode::RightShift => self.keymods.shift = down,
            KeyCode::LeftControl | KeyCode::RightControl => self.keymods.ctrl = down,
            KeyCode::LeftAlt | KeyCode::RightAlt => self.keymods.alt = down,
            KeyCode::LeftSuper | KeyCode::RightSuper => self.keymods.logo = down,
            _ => {}
        }
    }

    fn key(&mut self, handler: &mut dyn EventHandler, key: KeyCode, down: bool) {
        self.update_mods(key, down);
        if down {
            handler.key_down_event(key, self.keymods, false);
        } else {
            handler.key_up_event(key, self.keymods);
        }
    }

    fn process_pad(&mut self, pad: &sys::SceCtrlData, handler: &mut dyn EventHandler) -> bool {
        let mut any = false;

        // Menu PS / application en arrière-plan : la Vita intercepte les entrées.
        let intercepted = pad.buttons & sys::SCE_CTRL_INTERCEPTED != 0;
        if intercepted != self.intercepted {
            self.intercepted = intercepted;
            if intercepted {
                handler.window_minimized_event();
            } else {
                handler.window_restored_event();
            }
            any = true;
        }
        let buttons = if intercepted { 0 } else { pad.buttons };

        // Boutons → touches (fronts montants / descendants).
        let changed = buttons ^ self.prev_buttons;
        if changed != 0 {
            let mapping = self.cfg.buttons.clone();
            for (mask, key) in mapping {
                if changed & mask != 0 {
                    self.key(handler, key, buttons & mask != 0);
                    any = true;
                }
            }
            if self.cfg.cross_is_left_click && changed & sys::SCE_CTRL_CROSS != 0 {
                let (x, y) = self.cursor;
                if buttons & sys::SCE_CTRL_CROSS != 0 {
                    handler.mouse_button_down_event(MouseButton::Left, x, y);
                } else {
                    handler.mouse_button_up_event(MouseButton::Left, x, y);
                }
            }
        }
        self.prev_buttons = buttons;

        // Sticks : 0..255, centre 128. y vers le bas positif (comme l'écran).
        let dz = self.cfg.stick_deadzone;
        let axis = |v: u8| -> f32 {
            let f = (v as f32 - 128.0) / 127.0;
            let f = f.clamp(-1.0, 1.0);
            if f.abs() < dz {
                0.0
            } else {
                f
            }
        };
        let (lx, ly, rx, ry) = if intercepted {
            (0.0, 0.0, 0.0, 0.0)
        } else {
            (axis(pad.lx), axis(pad.ly), axis(pad.rx), axis(pad.ry))
        };

        match self.cfg.left_stick {
            LeftStickMode::Disabled => {}
            LeftStickMode::Keys {
                up,
                down,
                left,
                right,
            } => {
                let now = [ly < 0.0, ly > 0.0, lx < 0.0, lx > 0.0];
                let keys = [up, down, left, right];
                for i in 0..4 {
                    if now[i] != self.prev_stick_keys[i] {
                        self.key(handler, keys[i], now[i]);
                        any = true;
                    }
                }
                self.prev_stick_keys = now;
            }
            LeftStickMode::Mouse { speed } => {
                if lx != 0.0 || ly != 0.0 {
                    let (dx, dy) = (lx * speed, ly * speed);
                    self.cursor.0 = (self.cursor.0 + dx).clamp(0.0, self.screen_w - 1.0);
                    self.cursor.1 = (self.cursor.1 + dy).clamp(0.0, self.screen_h - 1.0);
                    handler.raw_mouse_motion(dx, dy);
                    handler.mouse_motion_event(self.cursor.0, self.cursor.1);
                    any = true;
                }
            }
        }

        let rs = self.cfg.right_stick_raw_speed;
        if rs != 0.0 && (rx != 0.0 || ry != 0.0) {
            handler.raw_mouse_motion(rx * rs, ry * rs);
            any = true;
        }

        any
    }

    fn process_touch(&mut self, data: &sys::SceTouchData, handler: &mut dyn EventHandler) -> bool {
        let mut any = false;
        let n = (data.report_num as usize).min(sys::SCE_TOUCH_MAX_REPORT);
        let mut current: Vec<ActiveTouch> = Vec::with_capacity(n);
        for r in &data.report[..n] {
            current.push(ActiveTouch {
                id: r.id,
                x: (r.x as f32 - self.touch_origin.0) * self.touch_scale.0,
                y: (r.y as f32 - self.touch_origin.1) * self.touch_scale.1,
            });
        }

        // Fins de contact.
        for old in &self.touches {
            if !current.iter().any(|t| t.id == old.id) {
                handler.touch_event(TouchPhase::Ended, old.id as u64, old.x, old.y);
                any = true;
            }
        }
        // Débuts et déplacements.
        for t in &current {
            match self.touches.iter().find(|o| o.id == t.id) {
                None => {
                    handler.touch_event(TouchPhase::Started, t.id as u64, t.x, t.y);
                    any = true;
                }
                Some(o) if o.x != t.x || o.y != t.y => {
                    handler.touch_event(TouchPhase::Moved, t.id as u64, t.x, t.y);
                    any = true;
                }
                _ => {}
            }
        }
        self.touches = current;
        any
    }
}

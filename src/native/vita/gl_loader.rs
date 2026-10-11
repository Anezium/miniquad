//! Origine : nouveau fichier, ajouté au miniquad 0.4.8 vendoré (crates.io, git cc6f8d1f).
//! Version : miniquad 0.4.8 + backend PS Vita (spike A, jalon 3, Alice Vita).
//! Modifications pour PS Vita : tout le fichier.
//!
//! Résolution des fonctions GL pour `crate::native::gl::load_gl_funcs`.
//!
//! Sur Android/Linux, miniquad passe par `eglGetProcAddress` (chargé par dlopen). Sur Vita il n'y a
//! pas de liaison dynamique : vitaGL est lié statiquement et expose `vglGetProcAddress`
//! (vitaGL.h, lookup.c), une table nom → adresse des ~360 fonctions GL qu'il implémente
//! (il retire aussi les suffixes EXT/ARB/OES). On s'en sert directement.
//!
//! miniquad 0.4.8 demande 125 fonctions ; 14 n'existent pas dans vitaGL dca4b9d :
//!   glBlendColor glClearBufferfi glClearBufferfv glClearBufferuiv glCompressedTexImage3D
//!   glDetachShader glDrawBuffers glFramebufferTextureLayer glGetQueryObjectui64v
//!   glRenderbufferStorageMultisample glTexImage3D glTexParameterfv glTexSubImage3D
//!   glVertexAttribIPointer
//! (liste obtenue en croisant `gl_loader!` de src/native/gl.rs avec la table de vitaGL/source/lookup.c).
//! Une fonction non résolue reste `None` dans miniquad et provoque un panic *à l'appel*.
//! Celles qui sont sur le chemin normal de miniquad reçoivent un remplaçant ci-dessous ;
//! les autres (textures 3D/tableaux, glClearBuffer*, requêtes 64 bits) restent non résolues :
//! vitaGL annonce "OpenGL ES 2.0", miniquad n'emprunte donc pas ces chemins (pas d'instancing,
//! pas de resolve MSAA, pas de texture array).
#![allow(non_snake_case)]

use super::sys;
use crate::native::gl::{GLboolean, GLclampf, GLenum, GLint, GLsizei, GLuint};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! { static DXT5_UPLOAD: Cell<bool> = const { Cell::new(false) }; }
static DXT5_COUNT: AtomicUsize = AtomicUsize::new(0);
static DXT5_SAVED: AtomicUsize = AtomicUsize::new(0);

/// Opt in only while creating immutable RGBA8 textures. Dynamic atlases, movies,
/// render targets and readback textures must stay outside this scope: vitaGL does
/// not support ordinary subimage updates on DXT storage. The scope is thread-local
/// and restores its previous state even if the closure unwinds.
pub fn with_dxt5_uploads<T>(upload: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            DXT5_UPLOAD.with(|flag| flag.set(self.0));
        }
    }
    let _restore = Restore(DXT5_UPLOAD.with(|flag| flag.replace(true)));
    upload()
}

unsafe extern "C" {
    #[link_name = "glTexImage2D"]
    fn vgl_tex_image_2d(
        target: GLenum,
        level: GLint,
        internal_format: GLint,
        width: GLsizei,
        height: GLsizei,
        border: GLint,
        format: GLenum,
        pixel_type: GLenum,
        data: *const core::ffi::c_void,
    );
}

unsafe extern "C" fn shim_glTexImage2D(
    target: GLenum,
    level: GLint,
    mut internal_format: GLint,
    width: GLsizei,
    height: GLsizei,
    border: GLint,
    format: GLenum,
    pixel_type: GLenum,
    data: *const core::ffi::c_void,
) {
    use crate::native::gl::{GL_RGBA, GL_RGBA8, GL_TEXTURE_2D, GL_UNSIGNED_BYTE};
    // Restrict to POT textures >= one block: vitaGL pads NPOT inputs and its
    // compressor reads full 4x4 blocks. Leave other GL formats untouched.
    if DXT5_UPLOAD.with(Cell::get)
        && target == GL_TEXTURE_2D
        && level == 0
        && border == 0
        && !data.is_null()
        && format == GL_RGBA
        && pixel_type == GL_UNSIGNED_BYTE
        && (internal_format == GL_RGBA as GLint || internal_format == GL_RGBA8 as GLint)
        && width >= 4
        && height >= 4
        && (width as u32).is_power_of_two()
        && (height as u32).is_power_of_two()
    {
        internal_format = 0x83F3; // GL_COMPRESSED_RGBA_S3TC_DXT5_EXT
        let saved = width as usize * height as usize * 3;
        let total = DXT5_SAVED.fetch_add(saved, Ordering::Relaxed) + saved;
        let count = DXT5_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if count == 1 || count % 64 == 0 {
            eprintln!("[miniquad/vita] DXT5 uploads={count} estimated_saved_bytes={total}");
        }
    }
    unsafe {
        vgl_tex_image_2d(
            target,
            level,
            internal_format,
            width,
            height,
            border,
            format,
            pixel_type,
            data,
        )
    };
}
use std::ffi::CString;

type Proc = unsafe extern "C" fn();

/// Résolveur passé à `load_gl_funcs`.
pub(crate) fn get_proc_address(name: &str) -> Option<Proc> {
    if name == "glTexImage2D" {
        return Some(unsafe {
            std::mem::transmute::<
                unsafe extern "C" fn(
                    GLenum,
                    GLint,
                    GLint,
                    GLsizei,
                    GLsizei,
                    GLint,
                    GLenum,
                    GLenum,
                    *const core::ffi::c_void,
                ),
                Proc,
            >(shim_glTexImage2D)
        });
    }
    let cname = CString::new(name).ok()?;
    let addr = unsafe { sys::vglGetProcAddress(cname.as_ptr()) };
    if !addr.is_null() {
        return Some(unsafe { std::mem::transmute::<*mut core::ffi::c_void, Proc>(addr) });
    }
    let fallback = fallback(name);
    if fallback.is_none() {
        // Non bloquant : n'échoue que si miniquad appelle réellement la fonction.
        eprintln!("[miniquad/vita] GL: {name} absent de vitaGL (non résolu)");
    } else {
        eprintln!("[miniquad/vita] GL: {name} absent de vitaGL, remplacé par un shim");
    }
    fallback
}

/// Remplaçants des fonctions absentes de vitaGL mais appelées par miniquad.
fn fallback(name: &str) -> Option<Proc> {
    unsafe {
        Some(match name {
            // Appelée à chaque création de shader (load_shader_internal). Sans effet notable :
            // miniquad supprime les shaders juste après, vitaGL les libère avec le programme.
            "glDetachShader" => {
                std::mem::transmute::<extern "C" fn(GLuint, GLuint), Proc>(shim_glDetachShader)
            }
            // Utilisée pour les passes de rendu (MRT / resolve). vitaGL n'a qu'un attachement
            // couleur : rien à sélectionner.
            "glDrawBuffers" => std::mem::transmute::<extern "C" fn(GLsizei, *const GLenum), Proc>(
                shim_glDrawBuffers,
            ),
            // Couleur de mélange constante : non supportée par vitaGL. Ignorée.
            "glBlendColor" => std::mem::transmute::<
                extern "C" fn(GLclampf, GLclampf, GLclampf, GLclampf),
                Proc,
            >(shim_glBlendColor),
            // Render textures avec sample_count > 1 : on retombe sur un renderbuffer sans MSAA.
            "glRenderbufferStorageMultisample" => {
                std::mem::transmute::<extern "C" fn(GLenum, GLsizei, GLenum, GLsizei, GLsizei), Proc>(
                    shim_glRenderbufferStorageMultisample,
                )
            }
            // Attributs entiers (VertexFormat::Int*) : GLES2 n'a que des attributs flottants.
            // On les passe en flottants non normalisés (valeurs converties, pas réinterprétées).
            "glVertexAttribIPointer" => std::mem::transmute::<
                extern "C" fn(GLuint, GLint, GLenum, GLsizei, *const core::ffi::c_void),
                Proc,
            >(shim_glVertexAttribIPointer),
            _ => return None,
        })
    }
}

extern "C" fn shim_glDetachShader(_program: GLuint, _shader: GLuint) {}

extern "C" fn shim_glDrawBuffers(_n: GLsizei, _bufs: *const GLenum) {}

extern "C" fn shim_glBlendColor(_r: GLclampf, _g: GLclampf, _b: GLclampf, _a: GLclampf) {}

extern "C" fn shim_glRenderbufferStorageMultisample(
    target: GLenum,
    _samples: GLsizei,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
) {
    unsafe { crate::native::gl::glRenderbufferStorage(target, internalformat, width, height) }
}

extern "C" fn shim_glVertexAttribIPointer(
    index: GLuint,
    size: GLint,
    type_: GLenum,
    stride: GLsizei,
    pointer: *const core::ffi::c_void,
) {
    unsafe {
        crate::native::gl::glVertexAttribPointer(
            index,
            size,
            type_,
            crate::native::gl::GL_FALSE as GLboolean,
            stride,
            pointer,
        )
    }
}

# miniquad pour PS Vita : origine et modifications

## Origine

| | |
|---|---|
| Crate | `miniquad` **0.4.8**, archive crates.io `miniquad-0.4.8.crate` |
| sha256 de l'archive | `2fb3e758e46dbc45716a8a49ca9edc54b15bcca826277e80b1f690708f67f9e3` (= `checksum` du `Cargo.lock` de Looking Glass) |
| Commit upstream | `cc6f8d1f5056eef6c4abb34b2bec376e0e262a9d` (not-fl3/miniquad, tag `v0.4.8`, d'après `.cargo_vcs_info.json` de l'archive) |
| Pourquoi cette version | `skulitom/LookingGlass` tag `v0.32.0-preview.2` (commit `0220970`) : `Cargo.toml` → `macroquad = "=0.4.14"`, `Cargo.lock` → `miniquad 0.4.8` (macroquad 0.4.14 exige `miniquad = "=0.4.8"`) |
| Licence | MIT OR Apache-2.0, fichiers `LICENSE-MIT` et `LICENSE-APACHE` conservés |

Branche `vita-0.4.8` du fork Anezium/miniquad, créée sur `cc6f8d1f`. Les sources de ce commit
sont celles de l'archive crates.io : seuls les fichiers que cargo ajoute à l'empaquetage diffèrent
(`Cargo.toml` normalisé, `Cargo.toml.orig` identique au `Cargo.toml` du dépôt, `.cargo_vcs_info.json`,
`Cargo.lock`). Trois commits par-dessus : le module `native/vita`, son câblage sous
`cfg(target_os = "vita")`, ce fichier. Un correctif complémentaire conserve les trois lignes
de provenance en tête de `src/lib.rs` et `src/native.rs` : les emplacements de panic incorporés
dans l’ELF restent ainsi identiques à ceux de la copie initiale lors de la migration.

Utilisation : sous-module `spike-a/miniquad` du dépôt Alice-American-McGees-PSVita-Port, et
`[patch.crates-io] miniquad = { path = "../miniquad" }` dans le crate consommateur
(voir `spike-a/macroquad-basic/Cargo.toml`).

## Modifications

Les autres backends (Linux X11/Wayland, Android, Windows, macOS, iOS, wasm) ne sont **pas** modifiés ;
tout l'ajout est derrière `#[cfg(target_os = "vita")]` (valeur vérifiée avec
`rustc --print cfg --target armv7-sony-vita-newlibeabihf` : `target_os="vita"`, `target_family="unix"`,
`target_env="newlib"`). `cargo check --lib` sur Linux x86_64 passe toujours (seul l'avertissement
`static_mut_refs` d'origine).

| Fichier | Nature | Détail |
|---|---|---|
| `src/native.rs` | modifié (+4 lignes) | `#[cfg(target_os = "vita")] pub mod vita;` |
| `src/lib.rs` | modifié (+6 lignes) | branche `#[cfg(target_os = "vita")] native::vita::run(&conf, f)` dans `start()` |
| `src/native/vita/mod.rs` | nouveau | `VitaConfig` + `set_config`, `run()` : `vglInitExtended`, chargement GL, `NativeDisplayData` 960x544 / dpi 1.0, boucle update/draw/`vglSwapBuffers`, requêtes de fenêtre no-op, presse-papiers no-op, temps de frame optionnel (`std::time::Instant`) |
| `src/native/vita/sys.rs` | nouveau | FFI à la main : vitaGL, SceCtrl, SceTouch, `sceKernelGetProcessTimeWide` ; `#[link(..., kind = "static", modifiers = "-bundle")]` avec la liste de `spike-a/triangle/src/vgl.rs` + SceCtrl_stub, SceTouch_stub, SceLibKernel_stub |
| `src/native/vita/gl_loader.rs` | nouveau | résolveur pour `gl::load_gl_funcs` : `vglGetProcAddress`, puis shims pour 5 fonctions absentes de vitaGL |
| `src/native/vita/input.rs` | nouveau | boutons → KeyDown/KeyUp, sticks → touches ou souris virtuelle / raw_mouse_motion, tactile avant → touch_event, interception (menu PS) → minimized/restored |
| `VITA-CHANGES.md` | nouveau | ce fichier |

Aucune dépendance ajoutée (pas de `libc` ni de `vitasdk-sys` sur la cible Vita) ; `build.rs` et
`Cargo.toml` inchangés.

### Free memory logging (`vita-input`)

`VitaConfig::log_memory_every` defaults to `Some(600)`. The backend logs free memory to
stderr once before vitaGL initialization, then after every 600 rendered/swapped frames.
The line reports `user`, `cdram` and `phycont` in bytes from `sceKernelGetFreeMemorySize`.
The startup sample already includes allocations made by the application before `miniquad::start`.
`None` disables both startup and periodic samples; `Some(0)` samples every rendered frame.
The counter is independent of frame-time logging and does not count idle iterations of the
blocking event loop. Failed SDK calls report their error code without printing invalid sizes.

The FFI uses the four C `int` fields from `psp2/kernel/sysmem.h`, initializes `size` to 0x10,
checks the structure size at compile time, and links `SceSysmem_stub` for the new import.

### Fonctions GL

miniquad 0.4.8 charge 125 fonctions par nom. 111 sont dans la table de `vglGetProcAddress`
(`vitaGL/source/lookup.c`, commit `dca4b9d`). Les 14 absentes :

| Fonction | Traitement Vita | Appelée par miniquad quand |
|---|---|---|
| `glDetachShader` | shim no-op | **à chaque shader** (`load_shader_internal`) : sans shim, panic au premier shader |
| `glDrawBuffers` | shim no-op | passes de rendu MRT / resolve |
| `glBlendColor` | shim no-op | jamais dans miniquad 0.4.8 (déclarée seulement) ; shim par précaution pour le code appelant `miniquad::gl` directement |
| `glRenderbufferStorageMultisample` | shim → `glRenderbufferStorage` sans MSAA | render pass avec `sample_count > 1` |
| `glVertexAttribIPointer` | shim → `glVertexAttribPointer(normalized = false)` | attributs entiers avec `gl_pass_as_float = false` (macroquad ne le fait pas) |
| `glClearBufferfi/fv/uiv`, `glTexImage3D`, `glTexSubImage3D`, `glCompressedTexImage3D`, `glFramebufferTextureLayer`, `glTexParameterfv`, `glGetQueryObjectui64v` | non résolues (panic si appelées) | textures 3D / tableaux, requêtes GPU : chemins GLES3, non pris puisque vitaGL annonce `OpenGL ES 2.0 VitaGL` |

Chaque fonction absente est signalée une fois sur stderr au démarrage
(`[miniquad/vita] GL: … absent de vitaGL`).

## macroquad et ses dépendances

**Aucun patch nécessaire.** `macroquad 0.4.14` sans feature (`default-features = false`, donc sans
`audio`/quad-snd, comme Looking Glass) compile tel quel pour `armv7-sony-vita-newlibeabihf` avec
fontdue 0.9.4, glam 0.27.0 (`scalar-math`), image 0.24.9 (png, tga), png 0.17.16, quad-rand 0.2.3,
ttf-parser 0.25.1. Si la feature `audio` est activée un jour, quad-snd n'a pas de backend Vita :
il faudra un backend `sceAudioOut` (jalon 6 du brief).

## 2026-10-11 — fix: vglInitExtended return value

`vglInitExtended` returns GL_FALSE on a normal init and GL_TRUE only when the resolution was clamped
(vitaGL `source/vgl.c`, `res_fallback`). The backend treated GL_FALSE as a failure and panicked on every
successful init; found on the first on-console run of the macroquad sample (ALICEMQ01, C2-12828-1).
Now logged on GL_TRUE, never fatal.


## Scoped DXT5 uploads (village experiment, 2026-10-11)

`native::vita::with_dxt5_uploads` opts in during creation of immutable RGBA8
textures only. The glTexImage2D resolver uses vitaGL's runtime DXT5 encoder
for non-null, level-zero, power-of-two images with both dimensions >=4.
Other calls pass through unchanged. The scope is thread-local, nestable and
restored on unwind. Dynamic textures, font atlases, render targets and textures
used for readback must remain outside it; ordinary glTexSubImage2D updates on
DXT storage are not supported by vitaGL. No CPU shadow copy is retained.
The upload counters report estimated uncompressed-minus-DXT bytes, not current
pool occupancy or confirmed successful allocation.

Host guard tests with a mocked vitaGL upload (no VitaSDK required):

```sh
rustc --edition 2021 --test tests/vita_dxt5_shim.rs -o /tmp/vita-dxt5-shim-tests
/tmp/vita-dxt5-shim-tests
```

Console validation is recorded in the parent project's DECISIONS.md.

## Compact empty stream buffers (village iteration 3)

`VitaConfig::compact_stream_buffers` defaults to false. Opting in defers storage
for `BufferUsage::Stream` + `BufferSource::Empty` until `buffer_update`, and
replaces it using `glBufferData` with the uploaded byte count. Logical maximum
size, bounds checks and index element type stay unchanged. The old GPU storage
is retired by vitaGL as usual; this does not enable BUFFERS_SPEEDHACK or overwrite
in-flight storage. Other buffer usages and initially populated streams keep their
existing path. Other platforms are unchanged.

This mode is for whole draw payload updates, as performed by macroquad: bytes
beyond the latest upload are discarded. The buffer must be updated before drawing
from it. It avoids reserving the game's global 60000 vertices and 180000 indices
(2400000 + 360000 bytes) for every small macroquad draw call. Larger draw payloads
still allocate their full size; no mesh is clamped by this change.

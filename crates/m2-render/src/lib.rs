//! Rendering abstraction.
//!
//! Translated rendering code talks to a backend trait instead of Direct3D 9
//! directly. The first backend forwards to the original D3D9 device; later
//! backends (Vulkan or Direct3D 12) implement the same trait.

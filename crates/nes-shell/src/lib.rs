//! The shell's parts, as a library so the gates reach them: the GPU
//! picture (`gpu`), the paced console loop (`run`). The binary in
//! `main.rs` is the window around them.

#![forbid(unsafe_code)]

pub mod gpu;
pub mod run;

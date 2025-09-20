// lib.rs
//! A library for creating bootable ISO 9660 images with UEFI support.

// Public modules for interacting with the library's core functionalities.
pub mod fat;
pub mod iso;

// The builder module contains high-level orchestration logic
// for creating a complete disk and ISO image.
pub mod builder;
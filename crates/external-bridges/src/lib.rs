//! # external-bridges
//!
//! Điều phối các công cụ mã nguồn mở bên ngoài qua CLI:
//! - **The Sleuth Kit (TSK)**: mmls, fsstat, fls, icat
//! - **PhotoRec / TestDisk**: File carving headless
//! - **GNU ddrescue**: Cứu dữ liệu từ ổ đĩa bị bad sector

pub mod ddrescue_wrapper;
pub mod photorec_wrapper;
pub mod tsk_wrapper;

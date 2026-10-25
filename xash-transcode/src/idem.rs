//! Xash3D IDEM demo container constants and layout.
//!
//! Mirrored from `xash3d-fwgs/engine/client/cl_demo.c`. Every constant here is
//! load-bearing: `CL_ParseDemoHeader` rejects the file outright if `id`,
//! `dem_protocol`, or `net_protocol` disagree.

/// `(('M'<<24)+('E'<<16)+('D'<<8)+'I')` — little-endian, so the first four
/// bytes on disk read `IDEM`.
pub const IDEMOHEADER: i32 = 0x4D45_4449;

/// Xash demo container revision. `CL_ParseDemoHeader` hard-rejects anything else.
pub const DEMO_PROTOCOL: i32 = 3;

pub const PROTOCOL_GOLDSRC_VERSION: i32 = 48;

/// `PROTOCOL_GOLDSRC_VERSION | BIT(7)` = 176.
///
/// This is the whole trick. With this value in `net_protocol`,
/// `CL_GetProtocolFromDemo` returns `PROTO_GOLDSRC` and the engine decodes the
/// contained messages with GoldSrc protocol-48 semantics — which is exactly
/// what a DoD demo carries.
pub const PROTOCOL_GOLDSRC_VERSION_DEMO: i32 = PROTOCOL_GOLDSRC_VERSION | (1 << 7);

/// `CL_ReadRawNetworkData` aborts playback above this. Observed DoD demos peak
/// around 27 KB, so there is headroom, but not a lot.
pub const MAX_INIT_MSG: u32 = 0x8000;

// demo commands (cl_demo.c)
pub const DEM_NOREWIND: u8 = 1;
pub const DEM_READ: u8 = 2;
pub const DEM_JUMPTIME: u8 = 3;
pub const DEM_USERDATA: u8 = 4;
pub const DEM_USERCMD: u8 = 5;
pub const DEM_STOP: u8 = 6;

// directory entry types
pub const DEMO_STARTUP: i32 = 0;
pub const DEMO_NORMAL: i32 = 1;

/// i32 id, i32 dem_protocol, i32 net_protocol, f64 host_fps,
/// char mapname\[64\], char comment\[64\], char gamedir\[64\], i32 directory_offset.
/// Declared `#pragma pack(1)` upstream, so there is no padding before the f64.
pub const HEADER_SIZE: usize = 216;

/// i32 entrytype, f32 playback_time, i32 playback_frames, i32 offset,
/// i32 length, i32 flags, char description\[64\].
pub const ENTRY_SIZE: usize = 88;

pub const FIELD_WIDTH: usize = 64;

// ---------------------------------------------------------------------------
// GoldSrc side (HLDEMO) — for reference; parsing is handled by the `dem` crate.
// ---------------------------------------------------------------------------

/// timestamp(4) + RefParams(232) + UserCmd(52) + MoveVars(132) + view(12) + viewmodel(4)
#[allow(dead_code)]
pub const GOLDSRC_DEMOINFO_SIZE: usize = 436;

/// Xash's `host_fps` is clamped to `MIN_FPS..MAX_FPS_HARD` by
/// `CL_GetDemoFramerate`; 0.0 would clamp to the floor and play back in slow
/// motion, so always write something sane.
pub const DEFAULT_HOST_FPS: f64 = 100.0;

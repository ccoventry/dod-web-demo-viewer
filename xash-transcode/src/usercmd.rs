//! Synthesizes Xash's `dem_usercmd` frames from GoldSrc's per-`NetworkMessage`
//! `UserCmd` data.
//!
//! GoldSrc bundles the recorded player input (crucially, view angles — there
//! is no live mouse during playback, so this is the *only* source of camera
//! direction) inside each `NetworkMessage`'s 436-byte `DemoInfo` header. Xash
//! has no equivalent inline field: it expects a separate `dem_usercmd` frame,
//! written during live recording by `CL_WriteDemoUserCmd()`
//! (`engine/client/cl_demo.c`) and read back by `CL_ReadDemoUserCmd()`, which
//! always deltas against an all-zero baseline (`from = -1` in
//! `CL_WriteUsercmd`). Without it, `dem_read`/`dem_norewind` frames replay
//! entities and HUD state correctly (that's all in the network payload,
//! untouched by this gap) but the camera never turns — confirmed against a
//! real playback test on 2026-08-14.
//!
//! The wire format is Xash's generic delta-field encoder
//! (`MSG_WriteDeltaUsercmd` -> `Delta_WriteField`, `engine/common/net_encode.c`)
//! over the `DT_USERCMD_T` field table, which is a fixed part of every real
//! HL/DoD install's `delta.lst` (see that file's `usercmd_t` block). Field
//! order, types, and bit widths below are transcribed from it and must stay
//! in lockstep if it ever changes upstream.
//!
//! Per field: one "changed" bit, then (if set) the value in `bits` bits;
//! unset means "copy from the baseline", which — since our baseline is
//! always zero — is exactly equivalent to writing an explicit zero. We only
//! have real recorded data for a subset of fields (view angles being the
//! one that matters); the rest are left "unchanged" (zero), matching what a
//! stationary, keyless input frame would encode to. Movement fields
//! (forwardmove/sidemove/upmove) are safe to zero because GoldSrc-protocol
//! demo playback positions entities from the server-authoritative network
//! stream, not from local usercmd prediction.
//!
//! Bit order mirrors `net_buffer.c`'s `MSG_WriteOneBit`/`MSG_WriteUBitLong`:
//! LSB-first within each byte, byte-padded only at the very end (there is no
//! per-frame padding — `CL_WriteDemoUserCmd` never calls
//! `MSG_StartBitWriting`/`MSG_EndBitWriting`, so bits from adjacent fields
//! pack contiguously).

use dem::types::UserCmd;

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            acc: 0,
            nbits: 0,
        }
    }

    /// Write the low `bits` bits of `value`, LSB first.
    fn write_bits(&mut self, value: u32, bits: u32) {
        debug_assert!(bits >= 1 && bits <= 32);
        let masked = if bits == 32 {
            value as u64
        } else {
            (value as u64) & ((1u64 << bits) - 1)
        };
        self.acc |= masked << self.nbits;
        self.nbits += bits;
        while self.nbits >= 8 {
            self.out.push((self.acc & 0xFF) as u8);
            self.acc >>= 8;
            self.nbits -= 8;
        }
    }

    fn write_bit(&mut self, bit: bool) {
        self.write_bits(bit as u32, 1);
    }

    /// "Unchanged" field: just the zero flag bit — decodes as a copy from
    /// the (always-zero) baseline, i.e. the value stays 0.
    fn skip_field(&mut self) {
        self.write_bit(false);
    }

    /// "Changed" field carrying an explicit unsigned/raw-bits value.
    fn write_field_raw(&mut self, value: u32, bits: u32) {
        self.write_bit(true);
        self.write_bits(value, bits);
    }

    /// "Changed" field carrying an angle, quantized like `MSG_WriteBitAngle`.
    fn write_field_angle(&mut self, angle_deg: f32, bits: u32) {
        self.write_bit(true);
        let shift = 1u32 << bits;
        let mask = shift - 1;
        let mut a = angle_deg % 360.0;
        if a < 0.0 {
            a += 360.0;
        }
        let d = ((a * shift as f32) / 360.0) as i32;
        self.write_bits((d as u32) & mask, bits);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.out.push((self.acc & 0xFF) as u8);
        }
        self.out
    }
}

fn angle(v: &[f32], idx: usize) -> f32 {
    v.get(idx).copied().unwrap_or(0.0)
}

/// Encode `cmd` as a delta-vs-zero `usercmd_t`, matching what
/// `MSG_ReadDeltaUsercmd` (called with an all-zero `from`) expects on the
/// read side. Field order is `DT_USERCMD_T` from `delta.lst`.
pub fn encode_usercmd(cmd: &UserCmd) -> Vec<u8> {
    let mut w = BitWriter::new();

    // lerp_msec: DT_SHORT, 9 bits (raw bit-reinterpret, unsigned on the wire)
    w.write_field_raw(cmd.lerp_msec as u16 as u32, 9);
    // msec: DT_BYTE, 8 bits
    w.write_field_raw(cmd.msec as u32, 8);
    // viewangles[1] (yaw): DT_ANGLE, 16 bits
    w.write_field_angle(angle(&cmd.view_angles, 1), 16);
    // viewangles[0] (pitch): DT_ANGLE, 16 bits
    w.write_field_angle(angle(&cmd.view_angles, 0), 16);
    // buttons: DT_SHORT, 16 bits
    w.write_field_raw(cmd.buttons as u32, 16);
    // forwardmove: DT_SIGNED|DT_FLOAT, 12 bits — no direct usercmd-driven
    // effect on GoldSrc-protocol demo playback (positions are server-driven).
    w.skip_field();
    // lightlevel: DT_BYTE, 8 bits (raw bit-reinterpret)
    w.write_field_raw(cmd.light_level as u8 as u32, 8);
    // sidemove: DT_SIGNED|DT_FLOAT, 12 bits
    w.skip_field();
    // upmove: DT_SIGNED|DT_FLOAT, 12 bits
    w.skip_field();
    // impulse: DT_BYTE, 8 bits (raw bit-reinterpret)
    w.write_field_raw(cmd.impulse as u8 as u32, 8);
    // viewangles[2] (roll): DT_ANGLE, 16 bits
    w.write_field_angle(angle(&cmd.view_angles, 2), 16);
    // impact_index: DT_INTEGER, 6 bits
    w.skip_field();
    // impact_position[0..3]: DT_SIGNED|DT_FLOAT, 16 bits each
    w.skip_field();
    w.skip_field();
    w.skip_field();

    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors `net_buffer.c`'s `MSG_ReadOneBit`/`MSG_ReadUBitLong`/
    /// `MSG_ReadBitAngle` — LSB-first bit reader, used only to round-trip
    /// [`encode_usercmd`]'s output in tests.
    struct BitReader<'a> {
        data: &'a [u8],
        bitpos: usize,
    }

    impl<'a> BitReader<'a> {
        fn new(data: &'a [u8]) -> Self {
            Self { data, bitpos: 0 }
        }

        fn read_bits(&mut self, bits: u32) -> u32 {
            let mut value: u32 = 0;
            for i in 0..bits {
                let byte = self.data[self.bitpos >> 3];
                let bit = (byte >> (self.bitpos & 7)) & 1;
                value |= (bit as u32) << i;
                self.bitpos += 1;
            }
            value
        }

        fn read_bit(&mut self) -> bool {
            self.read_bits(1) != 0
        }

        fn read_field_raw(&mut self, bits: u32) -> Option<u32> {
            if self.read_bit() {
                Some(self.read_bits(bits))
            } else {
                None
            }
        }

        fn read_field_angle(&mut self, bits: u32) -> Option<f32> {
            if self.read_bit() {
                let shift = (1u32 << bits) as f32;
                let i = self.read_bits(bits);
                let mut ret = i as f32 * (360.0 / shift);
                if ret < -180.0 {
                    ret += 360.0;
                } else if ret > 180.0 {
                    ret -= 360.0;
                }
                Some(ret)
            } else {
                None
            }
        }
    }

    fn make_cmd(pitch: f32, yaw: f32, roll: f32) -> UserCmd {
        UserCmd {
            lerp_msec: 0,
            msec: 0,
            unknown1: 0,
            view_angles: vec![pitch, yaw, roll],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            light_level: 0,
            unknonwn2: 0,
            buttons: 0,
            impulse: 0,
            weapon_select: 0,
            unknown3: 0,
            unknown4: 0,
            impact_index: 0,
            impact_position: vec![0.0, 0.0, 0.0],
        }
    }

    #[test]
    fn skipped_fields_encode_to_a_single_unchanged_bit_each() {
        // lerp_msec/msec/viewangles/buttons/lightlevel/impulse are always
        // written explicitly (we have real data for them); only the
        // movement/impact fields are left "unchanged". An all-zero cmd
        // should decode every skipped field back to its zeroed baseline.
        let bytes = encode_usercmd(&make_cmd(0.0, 0.0, 0.0));
        let mut r = BitReader::new(&bytes);

        assert_eq!(r.read_field_raw(9), Some(0)); // lerp_msec
        assert_eq!(r.read_field_raw(8), Some(0)); // msec
        assert_eq!(r.read_field_angle(16), Some(0.0)); // yaw
        assert_eq!(r.read_field_angle(16), Some(0.0)); // pitch
        assert_eq!(r.read_field_raw(16), Some(0)); // buttons
        assert_eq!(r.read_field_raw(12), None); // forwardmove
        assert_eq!(r.read_field_raw(8), Some(0)); // lightlevel
        assert_eq!(r.read_field_raw(12), None); // sidemove
        assert_eq!(r.read_field_raw(12), None); // upmove
        assert_eq!(r.read_field_raw(8), Some(0)); // impulse
        assert_eq!(r.read_field_angle(16), Some(0.0)); // roll
        assert_eq!(r.read_field_raw(6), None); // impact_index
        assert_eq!(r.read_field_raw(16), None); // impact_position[0]
        assert_eq!(r.read_field_raw(16), None); // impact_position[1]
        assert_eq!(r.read_field_raw(16), None); // impact_position[2]
    }

    #[test]
    fn view_angles_round_trip_through_the_real_read_order() {
        let mut cmd = make_cmd(-12.5, 273.0, 0.0);
        cmd.lerp_msec = 16;
        cmd.msec = 16;
        let bytes = encode_usercmd(&cmd);
        let mut r = BitReader::new(&bytes);

        assert_eq!(r.read_field_raw(9), Some(16)); // lerp_msec
        assert_eq!(r.read_field_raw(8), Some(16)); // msec
        let yaw = r.read_field_angle(16).expect("yaw should be present");
        let pitch = r.read_field_angle(16).expect("pitch should be present");
        assert_eq!(r.read_field_raw(16), Some(0)); // buttons
        assert_eq!(r.read_field_raw(12), None); // forwardmove
        assert_eq!(r.read_field_raw(8), Some(0)); // lightlevel
        assert_eq!(r.read_field_raw(12), None); // sidemove
        assert_eq!(r.read_field_raw(12), None); // upmove
        assert_eq!(r.read_field_raw(8), Some(0)); // impulse
        let roll = r.read_field_angle(16).expect("roll should be present");
        assert_eq!(r.read_field_raw(6), None); // impact_index
        assert_eq!(r.read_field_raw(16), None); // impact_position[0]
        assert_eq!(r.read_field_raw(16), None); // impact_position[1]
        assert_eq!(r.read_field_raw(16), None); // impact_position[2]

        // 16-bit angle quantization (~360/65536 degrees of slack), and
        // MSG_ReadBitAngle normalizes its result into (-180, 180], so
        // compare modulo 360 rather than raw values.
        let angle_diff = |a: f32, b: f32| {
            let mut d = (a - b) % 360.0;
            if d > 180.0 {
                d -= 360.0;
            } else if d < -180.0 {
                d += 360.0;
            }
            d.abs()
        };
        assert!(angle_diff(pitch, cmd.view_angles[0]) < 0.01);
        assert!(angle_diff(yaw, cmd.view_angles[1]) < 0.01);
        assert!(angle_diff(roll, cmd.view_angles[2]) < 0.01);
    }
}

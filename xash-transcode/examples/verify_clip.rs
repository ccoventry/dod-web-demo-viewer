//! TEMP diagnostic: walk the raw IDEM bytes of a transcoded clip and print
//! the first few dem_read/dem_norewind frames' incoming_sequence + payload
//! first byte, to verify the synthetic-baseline fix actually landed in the
//! written output (not just in the pre-write in-memory state).
use std::env;

fn rd_i32(data: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn main() {
    let path = env::args().nth(1).expect("usage: verify_clip <file.dem>");
    let data = std::fs::read(&path).unwrap();

    let dir_off = rd_i32(&data, 212) as usize;
    let n = rd_i32(&data, dir_off);
    println!("entries: {n}");

    for i in 0..n as usize {
        let p = dir_off + 4 + i * 88; // ENTRY_SIZE
        let off = rd_i32(&data, p + 12) as usize;
        let len = rd_i32(&data, p + 16) as usize;
        println!("entry {i}: off={off} len={len}");

        let (mut q, end) = (off, off + len);
        let mut shown = 0;
        while q < end && shown < 12 {
            let cmd = data[q];
            q += 1;
            q += 4; // dt
            match cmd {
                1 | 2 => {
                    // dem_norewind | dem_read
                    let seq = rd_i32(&data, q); // incoming_sequence, first i32
                    q += 28;
                    let mlen = rd_i32(&data, q) as usize;
                    q += 4;
                    let first_bytes = &data[q..q + mlen.min(4)];
                    println!(
                        "  [{}] cmd={cmd} incoming_sequence={seq} msglen={mlen} first_bytes={:?}",
                        shown, first_bytes
                    );
                    q += mlen;
                    shown += 1;
                }
                3 => {} // dem_jumptime — cmd+dt only, nothing else
                4 => {
                    // dem_userdata
                    let sz = rd_i32(&data, q);
                    q += 4 + sz.max(0) as usize;
                }
                5 => {
                    // dem_usercmd
                    q += 8;
                    let nb = u16::from_le_bytes([data[q], data[q + 1]]) as usize;
                    q += 2 + nb;
                }
                6 => break, // dem_stop
                _ => {}
            }
        }
    }
}

"""HLDEMO -> Xash3D IDEM transcoder + validator.
Target layout mirrored from xash3d-fwgs/engine/client/cl_demo.c."""
import struct
from collections import Counter
import hldemo as H

IDEMOHEADER   = 0x4D454449          # 'M'<<24|'E'<<16|'D'<<8|'I'  -> bytes "IDEM"
DEMO_PROTOCOL = 3
PROTOCOL_GOLDSRC_VERSION      = 48
PROTOCOL_GOLDSRC_VERSION_DEMO = PROTOCOL_GOLDSRC_VERSION | (1 << 7)   # 176
MAX_INIT_MSG  = 0x8000

DEM_NOREWIND, DEM_READ, DEM_JUMPTIME, DEM_USERDATA, DEM_USERCMD, DEM_STOP = 1,2,3,4,5,6
DEMO_STARTUP, DEMO_NORMAL = 0, 1

IDEM_HEADER_SIZE = 216   # i32,i32,i32,f64,64,64,64,i32  (packed)
IDEM_ENTRY_SIZE  = 88    # i32,f32,i32,i32,i32,i32,char[64]

def _pad(s, n):
    b = s.encode('latin-1')[:n-1]
    return b + b'\x00' * (n - len(b))

def transcode(buf, hdr, entries, host_fps=100.0, keep_userdata=False, rebase_time=True):
    out = bytearray()
    stats = Counter()

    out += struct.pack("<iii", IDEMOHEADER, DEMO_PROTOCOL, PROTOCOL_GOLDSRC_VERSION_DEMO)
    out += struct.pack("<d", host_fps)
    out += _pad(hdr['map_name'], 64)
    out += _pad("transcoded by dod-tools", 64)
    out += _pad(hdr['game_directory'], 64)
    dir_off_pos = len(out)
    out += struct.pack("<i", 0)
    assert len(out) == IDEM_HEADER_SIZE, len(out)

    out_entries = []
    for e in entries:
        start = len(out)
        nframes = 0
        base = e['frames'][0].time if (rebase_time and e['frames']) else 0.0

        for f in e['frames']:
            dt = f.time - base
            if f.ftype in (H.FT_NETMSG_START, H.FT_NETMSG_NORMAL) or f.ftype >= 10:
                cmd = DEM_NOREWIND if f.ftype == H.FT_NETMSG_START else DEM_READ
                mo, ml = f.msg
                if ml > MAX_INIT_MSG:
                    stats['SKIPPED_oversize_msg'] += 1
                    continue
                out += struct.pack("<Bf", cmd, dt)
                out += struct.pack("<7i", *f.seq)
                out += struct.pack("<i", ml)
                out += buf[mo:mo+ml]
                stats['netmsg->dem_read' if cmd == DEM_READ else 'netmsg->dem_norewind'] += 1
            elif f.ftype == H.FT_DEMOSTART:
                out += struct.pack("<Bf", DEM_JUMPTIME, dt)
                stats['DemoStart->dem_jumptime'] += 1
            elif f.ftype == H.FT_NEXTSECTION:
                out += struct.pack("<Bf", DEM_STOP, dt)
                stats['NextSection->dem_stop'] += 1
            elif f.ftype == H.FT_DEMOBUFFER and keep_userdata:
                po, plen = f.payload
                (blen,) = struct.unpack_from("<i", buf, po)
                out += struct.pack("<Bf", DEM_USERDATA, dt)
                out += struct.pack("<i", blen)
                out += buf[po+4:po+4+blen]
                stats['DemoBuffer->dem_userdata'] += 1
            else:
                stats[f'DROPPED_{H.FT_NAME.get(f.ftype, f.ftype)}'] += 1
                continue
            nframes += 1

        out_entries.append(dict(
            entrytype=DEMO_STARTUP if e['type'] == 0 else DEMO_NORMAL,
            playback_time=e['track_time'], playback_frames=nframes,
            offset=start, length=len(out) - start, flags=0,
            description=e['description']))

    dir_off = len(out)
    out += struct.pack("<i", len(out_entries))
    for oe in out_entries:
        out += struct.pack("<ifiiii", oe['entrytype'], oe['playback_time'],
                           oe['playback_frames'], oe['offset'], oe['length'], oe['flags'])
        out += _pad(oe['description'], 64)
    struct.pack_into("<i", out, dir_off_pos, dir_off)
    return bytes(out), out_entries, stats

# ---------------------------------------------------------------- validation
def validate(data):
    """Re-implements xash3d-fwgs CL_ParseDemoHeader + CL_PlayDemo_f + the read loop."""
    errs, notes = [], []
    if len(data) < IDEM_HEADER_SIZE:
        return ["file shorter than IDEM header"], notes
    ident, dem_proto, net_proto = struct.unpack_from("<iii", data, 0)
    (fps,) = struct.unpack_from("<d", data, 12)
    mapname = H.cstr(data[20:84]); comment = H.cstr(data[84:148]); gamedir = H.cstr(data[148:212])
    (dir_off,) = struct.unpack_from("<i", data, 212)

    if ident != IDEMOHEADER: errs.append(f"id != IDEMOHEADER (got 0x{ident:08x})")
    if dem_proto != DEMO_PROTOCOL: errs.append(f"dem_protocol {dem_proto} != 3")
    if net_proto not in (PROTOCOL_GOLDSRC_VERSION_DEMO,):
        errs.append(f"net_protocol {net_proto} not accepted")
    if not (0 < dir_off < len(data)): errs.append(f"directory_offset {dir_off} out of range")
    notes.append(f"id={data[0:4].decode()!r} dem_proto={dem_proto} net_proto={net_proto} "
                 f"host_fps={fps} map={mapname!r} gamedir={gamedir!r}")
    if errs: return errs, notes

    (n,) = struct.unpack_from("<i", data, dir_off)
    if not (1 <= n <= 1024): errs.append(f"bogus numentries {n}")
    if errs: return errs, notes

    p = dir_off + 4
    for i in range(n):
        et, ptime, pframes, off, length, flags = struct.unpack_from("<ifiiii", data, p)
        desc = H.cstr(data[p+24:p+88]); p += IDEM_ENTRY_SIZE
        notes.append(f"  entry{i}: type={et} desc={desc!r} frames={pframes:,} "
                     f"offset={off:,} length={length:,} time={ptime:.1f}s")
        if off + length > len(data): errs.append(f"entry {i} extends past EOF")

        # walk it the way CL_DemoReadMessage does
        q, seen, stop = off, Counter(), False
        while q < off + length:
            cmd = data[q]; q += 1
            if cmd > DEM_STOP: errs.append(f"entry {i}: cmd {cmd} > dem_stop at {q-1}"); break
            (dt,) = struct.unpack_from("<f", data, q); q += 4
            seen[cmd] += 1
            if cmd in (DEM_NOREWIND, DEM_READ):
                q += 28
                (mlen,) = struct.unpack_from("<i", data, q); q += 4
                if mlen < 0: errs.append(f"entry {i}: negative msglen"); break
                if mlen > MAX_INIT_MSG: errs.append(f"entry {i}: msglen {mlen} > MAX_INIT_MSG"); break
                q += mlen
            elif cmd == DEM_USERDATA:
                (sz,) = struct.unpack_from("<i", data, q); q += 4 + sz
            elif cmd == DEM_USERCMD:
                q += 8; (nb,) = struct.unpack_from("<H", data, q); q += 2 + nb
            elif cmd in (DEM_JUMPTIME, DEM_STOP):
                if cmd == DEM_STOP: stop = True
            else:
                errs.append(f"entry {i}: unhandled cmd {cmd}"); break
        if q != off + length:
            errs.append(f"entry {i}: frame walk ended at {q}, expected {off+length} (drift {q-(off+length):+})")
        if not stop: errs.append(f"entry {i}: no terminating dem_stop")
        notes.append(f"           cmds={ {k:v for k,v in sorted(seen.items())} }")
    return errs, notes


def cut(buf, hdr, entries, t_start, t_end, preroll=3.0, **kw):
    """Keep the signon section whole; keep only [t_start-preroll, t_end] of playback.

    CAVEAT: svc_deltapacketentities deltas against earlier frames, so a cut that
    lands mid-stream shows corrupt entities until the next full update. `preroll`
    is a blunt mitigation. The correct fix is to walk forward to the first frame
    carrying a non-delta svc_packetentities -- that needs netmsg parsing, which
    lives in dem-patch/netmsg_doer on the Rust side.
    """
    lo = max(0.0, t_start - preroll)
    clipped = []
    for e in entries:
        if e['type'] == 0:
            clipped.append(e); continue
        keep = [f for f in e['frames']
                if f.ftype in (H.FT_DEMOSTART, H.FT_NEXTSECTION) or lo <= f.time <= t_end]
        if not any(f.ftype == H.FT_NEXTSECTION for f in keep):
            keep.append(e['frames'][-1])
        ne = dict(e); ne['frames'] = keep
        ne['track_time'] = max(0.0, t_end - lo)
        clipped.append(ne)
    return transcode(buf, hdr, clipped, **kw)

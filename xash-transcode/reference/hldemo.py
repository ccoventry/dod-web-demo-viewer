"""HLDEMO (GoldSrc) reference reader — layouts mirrored from dod-tools/dem-patch/src/types.rs"""
import struct, sys
from collections import Counter

HEADER_SIZE      = 544
DIR_ENTRY_SIZE   = 92
DEMOINFO_SIZE    = 436   # timestamp(4) + RefParams(232) + UserCmd(52) + MoveVars(132) + view(12) + viewmodel(4)
SEQINFO_SIZE     = 28    # 7 x i32
FRAME_HDR_SIZE   = 9     # u8 type + f32 time + i32 frame

FT_NETMSG_START, FT_NETMSG_NORMAL = 0, 1
FT_DEMOSTART, FT_CONSOLECMD, FT_CLIENTDATA, FT_NEXTSECTION = 2, 3, 4, 5
FT_EVENT, FT_WEAPONANIM, FT_SOUND, FT_DEMOBUFFER = 6, 7, 8, 9

FT_NAME = {0:"NetMsgStart",1:"NetMsgNormal",2:"DemoStart",3:"ConsoleCommand",
           4:"ClientData",5:"NextSection",6:"Event",7:"WeaponAnim",8:"Sound",9:"DemoBuffer"}

def cstr(b):
    i = b.find(b'\x00')
    return b[:i if i >= 0 else len(b)].decode('latin-1')

class Frame:
    __slots__ = ('ftype','time','index','payload','seq','msg','offset','size')

def read_header(buf):
    magic = buf[0:8]
    if magic != b"HLDEMO\x00\x00":
        raise ValueError(f"magic is not HLDEMO: {magic!r}")
    demo_proto, net_proto = struct.unpack_from("<ii", buf, 8)
    map_name = cstr(buf[16:276])
    game_dir = cstr(buf[276:536])
    checksum, dir_offset = struct.unpack_from("<Ii", buf, 536)
    return dict(demo_protocol=demo_proto, network_protocol=net_proto, map_name=map_name,
                game_directory=game_dir, map_checksum=checksum, directory_offset=dir_offset)

def read_directory(buf, off):
    (count,) = struct.unpack_from("<i", buf, off)
    if not (1 <= count <= 1024):
        raise ValueError(f"bogus directory entry count: {count}")
    entries, p = [], off + 4
    for _ in range(count):
        t, = struct.unpack_from("<i", buf, p)
        desc = cstr(buf[p+4:p+68])
        flags, cd_track, track_time, frame_count, frame_offset, file_length = \
            struct.unpack_from("<iifiii", buf, p+68)
        entries.append(dict(type=t, description=desc, flags=flags, cd_track=cd_track,
                            track_time=track_time, frame_count=frame_count,
                            frame_offset=frame_offset, file_length=file_length))
        p += DIR_ENTRY_SIZE
    return entries

def parse_frames(buf, start, limit):
    """Walk frames from `start`. Stops after NextSection or when limit reached."""
    frames, p = [], start
    while p < limit:
        f = Frame(); f.offset = p
        f.ftype = buf[p]
        f.time, f.index = struct.unpack_from("<fi", buf, p+1)
        q = p + FRAME_HDR_SIZE
        f.seq = f.msg = f.payload = None

        if f.ftype in (FT_NETMSG_START, FT_NETMSG_NORMAL) or f.ftype >= 10:
            info_at = q
            q += DEMOINFO_SIZE
            f.seq = struct.unpack_from("<7i", buf, q); q += SEQINFO_SIZE
            (mlen,) = struct.unpack_from("<I", buf, q); q += 4
            if mlen > 8 * 1024 * 1024:
                raise ValueError(f"absurd message length {mlen} at frame offset {p}")
            f.msg = (q, mlen); q += mlen
            f.payload = (info_at, DEMOINFO_SIZE)
        elif f.ftype in (FT_DEMOSTART, FT_NEXTSECTION):
            pass
        elif f.ftype == FT_CONSOLECMD:
            f.payload = (q, 64); q += 64
        elif f.ftype == FT_CLIENTDATA:
            f.payload = (q, 32); q += 32
        elif f.ftype == FT_EVENT:
            f.payload = (q, 84); q += 84
        elif f.ftype == FT_WEAPONANIM:
            f.payload = (q, 8); q += 8
        elif f.ftype == FT_SOUND:
            _chan, slen = struct.unpack_from("<ii", buf, q)
            body = 8 + slen + 16
            f.payload = (q, body); q += body
        elif f.ftype == FT_DEMOBUFFER:
            (blen,) = struct.unpack_from("<i", buf, q)
            f.payload = (q, 4 + blen); q += 4 + blen
        else:
            raise ValueError(f"unknown frame type {f.ftype} at offset {p}")

        f.size = q - p
        frames.append(f)
        p = q
        if f.ftype == FT_NEXTSECTION:
            break
    return frames, p

def load(path):
    with open(path, 'rb') as fh:
        buf = fh.read()
    hdr = read_header(buf)
    entries = read_directory(buf, hdr['directory_offset'])
    for e in entries:
        limit = min(e['frame_offset'] + e['file_length'], hdr['directory_offset']) \
                if e['file_length'] > 0 else hdr['directory_offset']
        e['frames'], e['end'] = parse_frames(buf, e['frame_offset'], limit)
    return buf, hdr, entries

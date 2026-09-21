"""Mirror of xash-transcode/src/resources.rs parse_bsp/entity_value, run against
synthetic BSP v30 files to validate the byte offsets and string handling."""
import struct

def parse_bsp(bsp):
    version = struct.unpack_from("<i", bsp, 0)[0]
    ent_off, ent_len = struct.unpack_from("<ii", bsp, 4)
    if ent_off == 0 or ent_off + ent_len > len(bsp):
        raise ValueError(f"entity lump out of range ({ent_off}, {ent_len})")
    ents = bsp[ent_off:ent_off+ent_len].decode('latin-1')
    ws = first_entity(ents) or ents
    wads, seen = [], set()
    v = entity_value(ws, "wad")
    if v:
        for part in v.split(';'):
            base = part.replace('\\','/').split('/')[-1].strip()
            if not base or not base.lower().endswith('.wad'):
                continue
            if base.lower() not in seen:
                seen.add(base.lower()); wads.append(base)
    sky = entity_value(ws, "skyname")
    return dict(version=version, wads=wads, skyname=(sky.strip() if sky else None) or None)

def first_entity(ents):
    s = ents.find('{')
    if s < 0: return None
    e = ents.find('}', s)
    if e < 0: return None
    return ents[s:e]

def entity_value(block, key):
    needle = f'"{key}"'
    rest = block
    while True:
        k = rest.find(needle)
        if k < 0: return None
        after = rest[k+len(needle):]
        o = after.find('"')
        if o >= 0:
            ao = after[o+1:]
            c = ao.find('"')
            if c >= 0: return ao[:c]
        rest = rest[k+len(needle):]

def make_bsp(ent_text):
    ents = ent_text.encode('latin-1')
    hdr_size = 4 + 15*8
    body = b'\x00' * 64
    ent_off = hdr_size + len(body)
    buf = bytearray()
    buf += struct.pack("<i", 30)
    for i in range(15):
        buf += struct.pack("<ii", ent_off if i == 0 else 0, len(ents) if i == 0 else 0)
    buf += body + ents
    return bytes(buf)

# ---- cases ----
cases = [
 ("real-world dod map",
  '{\n"wad" "\\sierra\\half-life\\valve\\halflife.wad;\\sierra\\half-life\\dod\\dod.wad;"\n'
  '"skyname" "dusk"\n"classname" "worldspawn"\n}\n{\n"classname" "info_player_allies"\n}\n',
  ["halflife.wad","dod.wad"], "dusk"),
 ("forward slashes + duplicates",
  '{\n"wad" "/games/valve/halflife.wad;/games/dod/dod.wad;/other/HALFLIFE.WAD;"\n"classname" "worldspawn"\n}\n',
  ["halflife.wad","dod.wad"], None),
 ("no sky, trailing empty segment",
  '{\n"classname" "worldspawn"\n"wad" "c:\\hl\\dod\\dod.wad;"\n}\n',
  ["dod.wad"], None),
 ("no wad key at all",
  '{\n"classname" "worldspawn"\n"skyname" "morning"\n}\n', [], "morning"),
 ("junk segments filtered",
  '{\n"wad" ";;\\hl\\dod\\dod.wad;notawad.txt;\\hl\\valve\\liquids.wad;"\n"classname" "worldspawn"\n}\n',
  ["dod.wad","liquids.wad"], None),
 ("key appears in a later entity too",
  '{\n"classname" "worldspawn"\n"wad" "\\a\\first.wad;"\n}\n{\n"wad" "\\b\\second.wad;"\n}\n',
  ["first.wad"], None),
]

fails = 0
for name, ents, want_wads, want_sky in cases:
    got = parse_bsp(make_bsp(ents))
    ok = got['wads'] == want_wads and got['skyname'] == want_sky
    print(('  PASS  ' if ok else '  FAIL  ') + name)
    if not ok:
        fails += 1
        print(f"        wads got={got['wads']} want={want_wads}")
        print(f"        sky  got={got['skyname']!r} want={want_sky!r}")
    else:
        print(f"        v{got['version']} wads={got['wads']} sky={got['skyname']!r}")

# offsets sanity: entity lump must be findable at the declared place
b = make_bsp('{\n"classname" "worldspawn"\n}\n')
assert struct.unpack_from("<i", b, 0)[0] == 30
eo, el = struct.unpack_from("<ii", b, 4)
assert b[eo:eo+el].startswith(b'{'), "lump[0] offset wrong"
print("  PASS  header layout: version@0, lump[0]@4/8")

# sound path prefixing (mirror of resolve_path)
def resolve(kind, name):
    n = name.lstrip('/').replace('\\','/')
    return f"sound/{n}" if kind == "Sound" and not n.startswith("sound/") else n
for k, i, w in [("Sound","weapons/garand_fire.wav","sound/weapons/garand_fire.wav"),
                ("Sound","sound/ambience/wind.wav","sound/ambience/wind.wav"),
                ("Model","models\\player\\us_garand\\us_garand.mdl","models/player/us_garand/us_garand.mdl"),
                ("Model","/sprites/640hud1.spr","sprites/640hud1.spr")]:
    ok = resolve(k,i)==w
    print(('  PASS  ' if ok else '  FAIL  ')+f'resolve({k}, {i!r})')
    if not ok: fails+=1

print()
print("ALL CHECKS PASSED" if not fails else f"FAILURES: {fails}")

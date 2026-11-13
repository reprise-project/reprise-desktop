#!/usr/bin/env python3
"""Write src/tzcity/cities.txt, the towns the automatic time zone looks up, from GeoNames'
cities15000.txt (https://download.geonames.org/export/dump/cities15000.zip, CC BY 4.0).

Usage: scripts/tzcities.py CITIES15000_TXT [TZDATA_ZI] > src/tzcity/cities.txt

Each line is a zone and its towns as latitude,longitude in hundredths of a degree. Zone links
are replaced by their targets from tzdata.zi. A town is dropped when a bigger kept town of the
same zone lies within THIN_KM and no town of another zone lies within BORDER_KM, so only towns
away from a zone's edge are thinned.
"""
import math
import sys
from collections import defaultdict

THIN_KM = 80.0
BORDER_KM = 100.0
CELL = 2.0


def links(path):
    out = {}
    try:
        with open(path, encoding="utf-8") as f:
            for line in f:
                p = line.split()
                if len(p) == 3 and p[0] == "L":
                    out[p[2]] = p[1]
    except OSError:
        pass
    return out


def km(a, b):
    la1, lo1, la2, lo2 = map(math.radians, (a[0], a[1], b[0], b[1]))
    h = math.sin((la2 - la1) / 2) ** 2 + math.cos(la1) * math.cos(la2) * math.sin((lo2 - lo1) / 2) ** 2
    return 2 * 6371.0 * math.asin(min(1.0, math.sqrt(h)))


def cell(at):
    return (math.floor(at[0] / CELL), math.floor(at[1] / CELL))


def around(grid, at):
    ci, cj = cell(at)
    half = int(180 / CELL)
    for di in (-1, 0, 1):
        for dj in range(-3, 4):
            yield from grid.get((ci + di, (cj + dj + half) % (2 * half) - half), ())


def main():
    alias = links(sys.argv[2] if len(sys.argv) > 2 else "/usr/share/zoneinfo/tzdata.zi")
    towns = []
    with open(sys.argv[1], encoding="utf-8") as f:
        for line in f:
            c = line.rstrip("\n").split("\t")
            if len(c) < 18 or not c[17]:
                continue
            zone = alias.get(c[17], c[17])
            at = (round(float(c[4]), 2), round(float(c[5]), 2))
            towns.append((int(c[14] or 0), at, zone))
    towns.sort(key=lambda t: (-t[0], t[1], t[2]))

    every = defaultdict(list)
    for _, at, zone in towns:
        every[cell(at)].append((at, zone))

    kept = defaultdict(list)
    out = defaultdict(list)
    for _, at, zone in towns:
        near = lambda grid, d, same: any(
            (z == zone) == same and km(at, a) < d for a, z in around(grid, at)
        )
        if near(kept, THIN_KM, True) and not near(every, BORDER_KM, False):
            continue
        kept[cell(at)].append((at, zone))
        out[zone].append(at)

    for zone in sorted(out):
        pts = sorted(set(out[zone]))
        print(zone, " ".join(f"{round(a * 100)},{round(o * 100)}" for a, o in pts))


if __name__ == "__main__":
    main()

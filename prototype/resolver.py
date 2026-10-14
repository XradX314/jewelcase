"""Prototipo del resolvedor de imágenes reales (etapa 2).
Misma lógica que irá en src-tauri/src/artwork/: MusicBrainz -> Cover Art Archive.

Uso:  python resolver.py "Artista" "Álbum" [--upc 0123456789012] [--download carpeta]
      python resolver.py --batch albums.txt     (una línea por álbum: Artista | Álbum)
"""
import json, sys, time, urllib.parse, urllib.request, argparse, os

UA = "JewelCaseWidget/0.1 (uso personal)"
MB = "https://musicbrainz.org/ws/2"
CAA = "https://coverartarchive.org"
_last_mb = 0.0


def get(url, mb=False):
    global _last_mb
    if mb:  # MusicBrainz pide como máximo 1 consulta por segundo
        wait = 1.1 - (time.time() - _last_mb)
        if wait > 0:
            time.sleep(wait)
        _last_mb = time.time()
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(req, timeout=25) as r:
                return json.load(r)
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            if e.code in (429, 502, 503) and attempt < 4:   # límite de consultas: esperar y reintentar
                time.sleep(2.5 * (attempt + 1)); continue
            raise


def q(s):
    return urllib.parse.quote(s.replace('"', ''))


def find_release_group(artist, album, upc=None):
    """Devuelve (release_group_id, título, release_id_exacto|None)."""
    if upc:
        d = get(f"{MB}/release/?query=barcode:{upc}&fmt=json&limit=5", mb=True)
        rels = (d or {}).get("releases", [])
        if rels:
            r = rels[0]
            return r["release-group"]["id"], r["title"], r["id"]
    query = urllib.parse.quote(f'releasegroup:"{album}" AND artist:"{artist}"'.replace('""','"'))
    d = get(f'{MB}/release-group/?query={query}&fmt=json&limit=5', mb=True)
    groups = [g for g in (d or {}).get("release-groups", []) if g.get("score", 0) >= 85]
    albums = [g for g in groups if g.get("primary-type") == "Album"] or groups
    if not albums:
        return None, None, None
    g = albums[0]
    return g["id"], g["title"], None


def cd_releases(rg_id):
    """Ediciones en CD del release group que tienen imágenes en CAA, las más prometedoras primero."""
    out, offset = [], 0
    while True:
        d = get(f"{MB}/release?release-group={rg_id}&inc=media&fmt=json&limit=100&offset={offset}", mb=True)
        rels = d.get("releases", [])
        for r in rels:
            fmts = {m.get("format") for m in r.get("media", [])}
            caa = r.get("cover-art-archive", {})
            if caa.get("artwork") and any(f and "CD" in f for f in fmts):
                out.append((caa.get("count", 0), r["id"], r.get("country"), r.get("date", "")))
        offset += len(rels)
        if offset >= d.get("release-count", 0) or not rels:
            break
    out.sort(key=lambda x: -x[0])   # más imágenes = más chances de tener foto del disco
    return out


def images(release_id):
    d = get(f"{CAA}/release/{release_id}")
    res = {}
    for im in (d or {}).get("images", []):
        for t in im.get("types", []):
            res.setdefault(t, []).append({"url": im["image"], "thumb": im.get("thumbnails", {}).get("large"), "comment": im.get("comment", "")})
    return res


def resolve(artist, album, upc=None, max_releases=12):
    rg, title, exact = find_release_group(artist, album, upc)
    if not rg:
        return {"artist": artist, "album": album, "found": False}
    result = {"artist": artist, "album": title, "found": True, "release_group": rg, "medium": None, "front": None, "back": None, "spine": None, "checked": 0}
    candidates = cd_releases(rg)
    if exact:
        candidates = [c for c in candidates if c[1] == exact] + [c for c in candidates if c[1] != exact]
    for count, rid, country, date in candidates[:max_releases]:
        imgs = images(rid)
        result["checked"] += 1
        for key, typ in (("medium", "Medium"), ("front", "Front"), ("back", "Back"), ("spine", "Spine")):
            if not result[key] and imgs.get(typ):
                result[key] = {"release": rid, "country": country, "date": date, **imgs[typ][0]}
        if result["medium"] and result["back"]:
            break
    return result


def download(url, path):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=60) as r, open(path, "wb") as f:
        f.write(r.read())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("artist", nargs="?")
    ap.add_argument("album", nargs="?")
    ap.add_argument("--upc")
    ap.add_argument("--batch")
    ap.add_argument("--download")
    a = ap.parse_args()
    items = []
    if a.batch:
        for line in open(a.batch, encoding="utf-8"):
            if "|" in line:
                ar, al = [x.strip() for x in line.split("|", 1)]
                items.append((ar, al))
    else:
        items.append((a.artist, a.album))
    results = []
    for ar, al in items:
        try:
            r = resolve(ar, al, a.upc)
        except Exception as e:
            print(f"{ar} — {al}: error {e}", flush=True); continue
        results.append(r)
        flag = lambda k: "sí" if r.get(k) else "no"
        print(f"{ar} — {al}: grupo={'sí' if r['found'] else 'no'} | CD={flag('medium')} frente={flag('front')} dorso={flag('back')} lomo={flag('spine')} | ediciones revisadas={r.get('checked',0)}", flush=True)
        if a.download and r.get("found"):
            os.makedirs(a.download, exist_ok=True)
            for k in ("medium", "front", "back", "spine"):
                if r.get(k):
                    src = r[k]["thumb"] or r[k]["url"]
                    try: download(src, os.path.join(a.download, f"{ar} - {al} - {k}.jpg".replace("/", "_")))
                    except Exception as e: print(f"   no se pudo bajar {k}: {e}")
    hits = sum(1 for r in results if r.get("medium"))
    print(f"\nCon foto real del CD: {hits}/{len(results)}")
    json.dump(results, open("resultados.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()

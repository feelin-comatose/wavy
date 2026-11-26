#!/usr/bin/env python3
"""Build the compact kanji → reading dictionary for romaji lyrics (core/assets/romaji-dict.tsv.gz).

Source: JMdict (EDRDG, CC BY-SA 4.0, https://www.edrdg.org/edrdg/licence.html). Only common words are kept
(priority tags news1/ichi1/spec1/spec2/gai1). When a spelling has several readings, the most frequent one wins
(空: そら, not から). Verbs and adjectives also get their stem ("言う" → "言" = "い"), used when the kanji is
followed by kana (言って → itte); a plain word reading is used otherwise.
Lines: "<kanji>\\tw\\t<reading>" (word) or "<kanji>\\ts\\t<reading>" (stem).
Usage: python3 scripts/build-romaji-dict.py [JMdict_e.gz]   (downloads it if no file is given)
"""
import gzip, io, re, sys, urllib.request, xml.etree.ElementTree as ET

URL = "http://ftp.edrdg.org/pub/Nihongo/JMdict_e.gz"
COMMON = {"news1", "ichi1", "spec1", "spec2", "gai1"}
KANJI = re.compile(r"[一-鿿々]")  # CJK ideographs + 々
OUT = "core/assets/romaji-dict.tsv.gz"
# JMdict frequencies come from newspapers; song lyrics read these words differently (空 is "sky", not "empty")
OVERRIDES = {"空": "そら", "今日": "きょう", "昨日": "きのう", "明日": "あした", "二人": "ふたり", "一人": "ひとり", "大人": "おとな",
             "何": "なに", "今": "いま", "私": "わたし", "上": "うえ", "下": "した", "中": "なか", "前": "まえ", "後": "あと",
             "日": "ひ", "風": "かぜ", "月": "つき", "人": "ひと", "時": "とき", "色": "いろ", "声": "こえ", "愛": "あい", "夢": "ゆめ"}
# …and verbs: the stem of the verb lyrics use most (行こう is iku, not okonau; 愛してる is aishiteru)
STEM_OVERRIDES = {"行": "い", "想": "おも", "歩": "ある", "抱": "だ", "降": "ふ", "愛": "あい"}

def score(pri):
    """Higher = more common: each priority tag counts, plus the news frequency rank (nf01 best … nf48)."""
    s = sum(10 for p in pri if p in COMMON)
    nf = [int(p[2:]) for p in pri if p.startswith("nf")]
    return s + (50 - min(nf) if nf else 0)

raw = open(sys.argv[1], "rb").read() if len(sys.argv) > 1 else urllib.request.urlopen(URL).read()
xml = gzip.decompress(raw).decode("utf-8")
xml = re.sub(r"<!ENTITY [^>]*>", "", xml)          # JMdict declares entities in its DTD
xml = re.sub(r"&([a-zA-Z0-9-]+);", r"\1", xml)    # …and uses them as values; keep the names
words, stems = {}, {}
def put(d, k, reading, sc):
    if k not in d or sc > d[k][0]: d[k] = (sc, reading)
for _, e in ET.iterparse(io.StringIO(xml), events=("end",)):
    if e.tag != "entry": continue
    rels = [r for r in e.findall("r_ele") if not r.findall("re_nokanji")]
    pos = " ".join(p.text or "" for p in e.iter("pos"))
    for k in e.findall("k_ele"):
        keb = k.findtext("keb") or ""
        pri = {p.text for p in k.findall("ke_pri")}
        if not KANJI.search(keb) or not rels or not (pri & COMMON): continue
        # the reading that belongs to this spelling (re_restr), the most common one first
        rs = [r for r in rels if not r.findall("re_restr") or keb in {x.text for x in r.findall("re_restr")}] or rels
        reb = max(rs, key=lambda r: score({p.text for p in r.findall("re_pri")})).findtext("reb")
        sc = score(pri)
        put(words, keb, reb, sc)
        # verb/adjective stem: drop the matching trailing kana from both ("言う"/"いう" → "言"/"い")
        if re.search(r"\bv[15]|\badj-i\b", pos):
            tail = re.search(r"[぀-ゟ]+$", keb)
            if tail and reb.endswith(tail.group()) and len(keb) > len(tail.group()):
                stem, rstem = keb[: -len(tail.group())], reb[: -len(tail.group())]
                if KANJI.search(stem) and rstem: put(stems, stem, rstem, sc)
    e.clear()
for k, v in OVERRIDES.items(): words[k] = (999, v)
for k, v in STEM_OVERRIDES.items(): stems[k] = (999, v)
lines = sorted([f"{k}\tw\t{v[1]}" for k, v in words.items()] + [f"{k}\ts\t{v[1]}" for k, v in stems.items()])
with gzip.open(OUT, "wt", encoding="utf-8", compresslevel=9) as f: f.write("\n".join(lines) + "\n")
print(f"{len(words)} words + {len(stems)} stems → {OUT}")

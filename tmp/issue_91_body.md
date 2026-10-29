Potvrđeni bugovi iz testa (0.9.23 / commit 4feefc1 / 2af99b9):

1. ANONYMIZE (B/W cijeli klip): mask_escaped = Some() ali maskedmerge s 3 ulaza daje crno-bijeli video. Potrebno: generirati ispravnu gray mask (cache/anon/<id>.mkv) i koristiti blend/overlay ispravno.
2. BACKGROUND REMOVAL: 1-frame zoom mask zalijepljen preko videa — maska nije ispravne dimenzije/format.
3. STICKER OVERLAY + EXPORT: Filter not found (0xb3b6b908) kada je sticker na timeline v2 — overlay filter ne integriran ispravno s export_graph.
4. PREVIEW FRAME: media.duration_ms=0 (neproban) uzrokuje seek_for=0.0 — sada ispravljeno u seek_for, ali mask generiranje još nije.

Status: Osnovni export (bez AI) radi. AI efekti traže maskfile generiranje. Rješenje: mask_gen.rs + ispravna blend/overlay sintaksa u export_graph.rs za 0.9.24.

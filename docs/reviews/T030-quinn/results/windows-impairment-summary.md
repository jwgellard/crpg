# T030 impairment matrix summary (native Windows/MSVC, rustc 1.98.0)

Runs: 39; harness failures: 0. Generated from `windows-impairment-matrix.jsonl`.
Columns: datagram frames tx = client DATAGRAM frames transmitted by the sampling point; relay up = client->server;
inversions / max depth = relay-measured reordering of forwarded datagrams; socket loss = relay-forwarded minus server `udp_rx.datagrams`
(includes the pre-connection handshake datagram(s)); dedup discards = `quinn_proto` duplicate-discard events, both endpoints.

"patched" here is the vendored tree (`third_party/quinn-proto/crate`, quinn-proto 0.11.19 with patch 0001 plus
patch 0002, the widened dedup window), not T030's patch-only build; "unpatched" is the registry quinn-proto.

| mode | build | profile | rate/s | s | seed | app sent | datagram frames tx | relay up in | injected drops | inversions | max depth | server udp_rx | server datagram rx | app unique rx | dedup discards | client lost pkts | socket loss |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| quic | unpatched | clean | 30 | 20 | 1 | 600 | 600 | 614 | 0 | 4 | 1 | 613 | 600 | 600 | 0 | 1 | 1 |
| quic | patched | clean | 30 | 20 | 1 | 600 | 600 | 612 | 0 | 4 | 2 | 611 | 600 | 600 | 0 | 0 | 1 |
| udp | raw | clean | 30 | 20 | 1 | 600 | — | 600 | 0 | 0 | 0 | — | — | 600 | — | — | 0 |
| quic | unpatched | clean | 300 | 20 | 1 | 6000 | 5906 | 5916 | 0 | 3909 | 16 | 5915 | 5906 | 5906 | 0 | 497 | 1 |
| quic | patched | clean | 300 | 20 | 1 | 6000 | 6000 | 6011 | 0 | 4098 | 17 | 6010 | 6000 | 6000 | 0 | 471 | 1 |
| udp | raw | clean | 300 | 20 | 1 | 6000 | — | 6000 | 0 | 3426 | 5 | — | — | 6000 | — | — | 0 |
| quic | unpatched | clean | 3000 | 10 | 1 | 30000 | 3293 | 3302 | 0 | 2302 | 14 | 3293 | 3285 | 3285 | 0 | 321 | 1 |
| quic | patched | clean | 3000 | 10 | 1 | 30000 | 3465 | 3474 | 0 | 2408 | 18 | 3473 | 3465 | 3465 | 0 | 282 | 1 |
| udp | raw | clean | 3000 | 10 | 1 | 30000 | — | 30000 | 0 | 28362 | 86 | — | — | 30000 | — | — | 0 |
| quic | unpatched | reorder | 30 | 20 | 1 | 600 | 600 | 614 | 0 | 14 | 4 | 613 | 600 | 600 | 0 | 0 | 1 |
| quic | patched | reorder | 30 | 20 | 1 | 600 | 600 | 616 | 0 | 17 | 1 | 615 | 600 | 600 | 0 | 2 | 1 |
| udp | raw | reorder | 30 | 20 | 1 | 600 | — | 600 | 0 | 11 | 1 | — | — | 600 | — | — | 0 |
| quic | unpatched | reorder | 300 | 20 | 1 | 6000 | 702 | 710 | 0 | 298 | 7 | 708 | 701 | 701 | 0 | 75 | 1 |
| quic | patched | reorder | 300 | 20 | 1 | 6000 | 711 | 719 | 0 | 254 | 7 | 714 | 707 | 707 | 0 | 59 | 1 |
| udp | raw | reorder | 300 | 20 | 1 | 6000 | — | 6000 | 0 | 4177 | 14 | — | — | 6000 | — | — | 0 |
| quic | unpatched | reorder | 3000 | 10 | 1 | 30000 | 429 | 437 | 0 | 190 | 9 | 436 | 429 | 429 | 0 | 46 | 1 |
| quic | patched | reorder | 3000 | 10 | 1 | 30000 | 376 | 384 | 0 | 159 | 8 | 382 | 375 | 375 | 0 | 47 | 1 |
| udp | raw | reorder | 3000 | 10 | 1 | 30000 | — | 30000 | 0 | 28654 | 146 | — | — | 30000 | — | — | 0 |
| quic | unpatched | impaired | 30 | 20 | 1 | 600 | 600 | 614 | 16 | 193 | 7 | 597 | 584 | 584 | 0 | 67 | 1 |
| quic | patched | impaired | 30 | 20 | 1 | 600 | 595 | 609 | 16 | 197 | 5 | 587 | 574 | 574 | 0 | 66 | 1 |
| udp | raw | impaired | 30 | 20 | 1 | 600 | — | 600 | 16 | 10 | 1 | — | — | 584 | — | — | 0 |
| quic | unpatched | impaired | 300 | 20 | 1 | 6000 | 555 | 563 | 15 | 232 | 7 | 542 | 535 | 535 | 0 | 84 | 1 |
| quic | patched | impaired | 300 | 20 | 1 | 6000 | 571 | 579 | 16 | 223 | 7 | 559 | 552 | 552 | 0 | 72 | 1 |
| udp | raw | impaired | 300 | 20 | 1 | 6000 | — | 6000 | 203 | 3999 | 14 | — | — | 5797 | — | — | 0 |
| quic | unpatched | impaired | 3000 | 10 | 1 | 30000 | 325 | 333 | 10 | 127 | 7 | 319 | 312 | 312 | 0 | 47 | 1 |
| quic | patched | impaired | 3000 | 10 | 1 | 30000 | 332 | 340 | 10 | 131 | 8 | 328 | 321 | 321 | 0 | 46 | 1 |
| udp | raw | impaired | 3000 | 10 | 1 | 30000 | — | 30000 | 928 | 27552 | 157 | — | — | 29062 | — | — | 10 |
| quic | unpatched | impaired | 30 | 20 | 2 | 600 | 600 | 612 | 12 | 191 | 5 | 599 | 588 | 588 | 0 | 66 | 1 |
| quic | patched | impaired | 30 | 20 | 2 | 600 | 600 | 613 | 12 | 69 | 5 | 600 | 588 | 588 | 0 | 19 | 1 |
| udp | raw | impaired | 30 | 20 | 2 | 600 | — | 600 | 12 | 5 | 1 | — | — | 588 | — | — | 0 |
| quic | unpatched | impaired | 300 | 20 | 2 | 6000 | 602 | 610 | 12 | 223 | 7 | 597 | 590 | 590 | 0 | 64 | 1 |
| quic | patched | impaired | 300 | 20 | 2 | 6000 | 618 | 626 | 13 | 231 | 7 | 609 | 602 | 602 | 0 | 79 | 1 |
| udp | raw | impaired | 300 | 20 | 2 | 6000 | — | 6000 | 173 | 4022 | 14 | — | — | 5827 | — | — | 0 |
| quic | unpatched | impaired | 30 | 20 | 3 | 600 | 527 | 538 | 25 | 188 | 6 | 510 | 501 | 501 | 0 | 68 | 1 |
| quic | patched | impaired | 30 | 20 | 3 | 600 | 600 | 611 | 28 | 208 | 5 | 582 | 573 | 573 | 0 | 70 | 1 |
| udp | raw | impaired | 30 | 20 | 3 | 600 | — | 600 | 28 | 6 | 1 | — | — | 572 | — | — | 0 |
| quic | unpatched | impaired | 300 | 20 | 3 | 6000 | 563 | 571 | 27 | 225 | 8 | 539 | 532 | 532 | 0 | 75 | 1 |
| quic | patched | impaired | 300 | 20 | 3 | 6000 | 559 | 567 | 27 | 185 | 8 | 535 | 528 | 528 | 0 | 75 | 1 |
| udp | raw | impaired | 300 | 20 | 3 | 6000 | — | 6000 | 190 | 4078 | 14 | — | — | 5810 | — | — | 0 |

QUIC runs: 26; total dedup discards: 0; max relay reorder depth for QUIC traffic: 18

## Environment

- Windows build: Microsoft Windows 11 Home 10.0.26300
- CPU count: 16
- `rustc -V`: rustc 1.98.0 (88d9e12ae 2026-08-18)

## Agent log

- 2026-11-16 (UTC) · opencode-go/deepseek-v4-pro + T150a · Rendered the native Windows/MSVC impairment matrix (39 runs) from `windows-impairment-matrix.jsonl` with the Linux summary's columns, the vendored-tree (0001 + 0002) build note and the Windows environment block.

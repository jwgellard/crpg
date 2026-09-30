# T030 impairment matrix summary (Linux/GNU, rustc 1.98.0)

Runs: 39; harness failures: 0. Generated from `linux-impairment-matrix.jsonl`.
Columns: datagram frames tx = client DATAGRAM frames transmitted by the sampling point; relay up = client->server;
inversions / max depth = relay-measured reordering of forwarded datagrams; socket loss = relay-forwarded minus server `udp_rx.datagrams`
(includes the pre-connection handshake datagram(s)); dedup discards = `quinn_proto` duplicate-discard events, both endpoints.

| mode | build | profile | rate/s | s | seed | app sent | datagram frames tx | relay up in | injected drops | inversions | max depth | server udp_rx | server datagram rx | app unique rx | dedup discards | client lost pkts | socket loss |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| quic | unpatched | clean | 30 | 20 | 1 | 600 | 600 | 618 | 0 | 6 | 2 | 617 | 600 | 600 | 0 | 1 | 1 |
| quic | patched | clean | 30 | 20 | 1 | 600 | 600 | 621 | 0 | 2 | 2 | 620 | 600 | 600 | 0 | 1 | 1 |
| udp | raw | clean | 30 | 20 | 1 | 600 | — | 600 | 0 | 2 | 2 | — | — | 600 | — | — | 0 |
| quic | unpatched | clean | 300 | 20 | 1 | 6000 | 6000 | 6012 | 0 | 6 | 1 | 6011 | 6000 | 6000 | 0 | 0 | 1 |
| quic | patched | clean | 300 | 20 | 1 | 6000 | 6000 | 6012 | 0 | 12 | 2 | 6011 | 6000 | 6000 | 0 | 1 | 1 |
| udp | raw | clean | 300 | 20 | 1 | 6000 | — | 6000 | 0 | 6 | 2 | — | — | 6000 | — | — | 0 |
| quic | unpatched | clean | 3000 | 10 | 1 | 30000 | 30000 | 30013 | 0 | 17084 | 14 | 30012 | 30000 | 30000 | 0 | 84 | 1 |
| quic | patched | clean | 3000 | 10 | 1 | 30000 | 30000 | 30008 | 0 | 17273 | 29 | 30007 | 30000 | 30000 | 0 | 120 | 1 |
| udp | raw | clean | 3000 | 10 | 1 | 30000 | — | 30000 | 0 | 13981 | 18 | — | — | 30000 | — | — | 0 |
| quic | unpatched | reorder | 30 | 20 | 1 | 600 | 600 | 615 | 0 | 5 | 4 | 613 | 600 | 600 | 0 | 0 | 2 |
| quic | patched | reorder | 30 | 20 | 1 | 600 | 600 | 615 | 0 | 6 | 4 | 613 | 600 | 600 | 0 | 0 | 2 |
| udp | raw | reorder | 30 | 20 | 1 | 600 | — | 600 | 0 | 0 | 0 | — | — | 600 | — | — | 0 |
| quic | unpatched | reorder | 300 | 20 | 1 | 6000 | 769 | 778 | 0 | 270 | 6 | 774 | 767 | 767 | 0 | 77 | 2 |
| quic | patched | reorder | 300 | 20 | 1 | 6000 | 791 | 800 | 0 | 278 | 7 | 797 | 790 | 790 | 0 | 72 | 2 |
| udp | raw | reorder | 300 | 20 | 1 | 6000 | — | 6000 | 0 | 3478 | 9 | — | — | 6000 | — | — | 0 |
| quic | unpatched | reorder | 3000 | 10 | 1 | 30000 | 448 | 457 | 0 | 151 | 7 | 451 | 444 | 444 | 0 | 37 | 2 |
| quic | patched | reorder | 3000 | 10 | 1 | 30000 | 441 | 450 | 0 | 165 | 7 | 441 | 434 | 434 | 0 | 37 | 2 |
| udp | raw | reorder | 3000 | 10 | 1 | 30000 | — | 30000 | 0 | 26437 | 94 | — | — | 30000 | — | — | 0 |
| quic | unpatched | impaired | 30 | 20 | 1 | 600 | 600 | 616 | 16 | 50 | 4 | 598 | 584 | 584 | 0 | 24 | 2 |
| quic | patched | impaired | 30 | 20 | 1 | 600 | 600 | 616 | 16 | 61 | 3 | 598 | 584 | 584 | 0 | 28 | 2 |
| udp | raw | impaired | 30 | 20 | 1 | 600 | — | 600 | 16 | 0 | 0 | — | — | 584 | — | — | 0 |
| quic | unpatched | impaired | 300 | 20 | 1 | 6000 | 716 | 725 | 20 | 237 | 6 | 700 | 693 | 693 | 0 | 74 | 2 |
| quic | patched | impaired | 300 | 20 | 1 | 6000 | 718 | 727 | 20 | 236 | 6 | 701 | 694 | 694 | 0 | 76 | 2 |
| udp | raw | impaired | 300 | 20 | 1 | 6000 | — | 6000 | 203 | 3345 | 8 | — | — | 5797 | — | — | 0 |
| quic | unpatched | impaired | 3000 | 10 | 1 | 30000 | 408 | 417 | 12 | 132 | 7 | 399 | 392 | 392 | 0 | 45 | 2 |
| quic | patched | impaired | 3000 | 10 | 1 | 30000 | 395 | 404 | 11 | 138 | 7 | 387 | 380 | 380 | 0 | 45 | 3 |
| udp | raw | impaired | 3000 | 10 | 1 | 30000 | — | 30000 | 928 | 25562 | 94 | — | — | 29072 | — | — | 0 |
| quic | unpatched | impaired | 30 | 20 | 2 | 600 | 600 | 612 | 12 | 83 | 5 | 599 | 588 | 588 | 0 | 28 | 1 |
| quic | patched | impaired | 30 | 20 | 2 | 600 | 600 | 614 | 12 | 80 | 5 | 601 | 589 | 589 | 0 | 29 | 1 |
| udp | raw | impaired | 30 | 20 | 2 | 600 | — | 600 | 12 | 0 | 0 | — | — | 588 | — | — | 0 |
| quic | unpatched | impaired | 300 | 20 | 2 | 6000 | 670 | 677 | 14 | 222 | 7 | 658 | 652 | 652 | 0 | 79 | 2 |
| quic | patched | impaired | 300 | 20 | 2 | 6000 | 756 | 763 | 16 | 291 | 13 | 743 | 737 | 737 | 0 | 90 | 1 |
| udp | raw | impaired | 300 | 20 | 2 | 6000 | — | 6000 | 173 | 3346 | 10 | — | — | 5827 | — | — | 0 |
| quic | unpatched | impaired | 30 | 20 | 3 | 600 | 600 | 617 | 28 | 139 | 6 | 588 | 574 | 574 | 0 | 54 | 1 |
| quic | patched | impaired | 30 | 20 | 3 | 600 | 600 | 616 | 28 | 109 | 5 | 587 | 574 | 574 | 0 | 54 | 1 |
| udp | raw | impaired | 30 | 20 | 3 | 600 | — | 600 | 28 | 0 | 0 | — | — | 572 | — | — | 0 |
| quic | unpatched | impaired | 300 | 20 | 3 | 6000 | 748 | 758 | 33 | 231 | 6 | 716 | 708 | 708 | 0 | 79 | 2 |
| quic | patched | impaired | 300 | 20 | 3 | 6000 | 636 | 646 | 29 | 209 | 5 | 613 | 605 | 605 | 0 | 81 | 2 |
| udp | raw | impaired | 300 | 20 | 3 | 6000 | — | 6000 | 190 | 3290 | 9 | — | — | 5810 | — | — | 0 |

QUIC runs: 26; total dedup discards: 0; max relay reorder depth for QUIC traffic: 29

## Agent log

- 2026-09-30 (UTC) · claude-code + T030 evidence artifact · Generated from the pinned-toolchain (1.98.0) matrix run so the dossier's findings can be re-derived rather than trusted.

# R17 — Hosting friction for co-op players (deep)

## Recommendations (ranked)

1. **Add a relay fallback behind STUN hole punching instead of port-forwarding alone.**  
   Why: Our transport is QUIC over UDP, so hole punching applies, but measured success is ~70%; CGNAT and UDP-blocking networks still need a relay to avoid player friction.  
   Evidence: https://www.factorio.com/blog/post/fff-143 — "Unfortunately it will not work under all circumstances (we expect about 70%)"  
   Strength: practitioner report. Effort: large.

2. **Implement STUN address discovery + UDP hole punching as the first connection attempt.**  
   Why: Ford measured ~82% UDP support and DCUtR's 4.4M attempts show ~70% hole-punch success, with no platform SDK dependency.  
   Evidence: https://arxiv.org/abs/cs/0603074v1 — "about 82% of the NATs tested support hole punching for UDP"  
   Strength: measured study. Effort: medium.

3. **Use QUIC connection migration to recover punched sessions without re-punching.**  
   Why: Re-punching costs RTTs; migration restores disrupted sessions more cheaply on mobile/weak networks.  
   Evidence: https://arxiv.org/abs/2408.01791v1 — "QUIC connection migration for connection restoration saves 2 RTTs compared to QUIC re-punching"  
   Strength: measured study. Effort: medium.

4. **Do not make a platform SDK (Steam SDR, EOS, PlayFab Party) the primary transport.**  
   Why: SDR relay access requires a Steam release and game coordinator; open-source GNS cannot reach the relay; PlayFab Party is a paid Azure relay.  
   Evidence: https://github.com/ValveSoftware/GameNetworkingSockets — "Note that the opensource code does not support accessing the relay network."  
   Strength: vendor doc. Effort: small.

5. **If we host a relay, prefer self-hosted TURN (coturn) or a minimal HTTPS relay over managed relays.**  
   Why: coturn is BSD-3-Clause and runs on both Windows and Linux; managed relays add cost and platform lock-in.  
   Evidence: https://github.com/coturn/coturn — "Target platforms: ... Windows (native with, e.g., MSVC toolchain)"  
   Strength: vendor doc. Effort: medium.

6. **Support IPv6 direct endpoints as an alternate path where both peers have it.**  
   Why: IPv6 removes most NAT traversal complexity; CGNAT is IPv4-only, though client IPv6 support still has gaps.  
   Evidence: https://tailscale.com/blog/how-nat-traversal-works — "in an IPv6-only world, all of this becomes much simpler"  
   Strength: practitioner report. Effort: medium.

7. **Replace manual IP entry with short join codes (plus optional platform invites) for lobby flow.**  
   Why: Valheim crossplay uses numeric join codes via its relay; Minecraft Realms uses invite links; both reduce host friction.  
   Evidence: https://valheim.fandom.com/wiki/Dedicated_servers — "players supply Valheim with a numeric Join Code"  
   Strength: practitioner report. Effort: small.

## Comparison table

| Option | NAT traversal | Relay fallback | Cost to us | Cost to players | Platform lock-in | Licence | Win/Linux server support | QUIC compat. | URL |
|---|---|---|---|---|---|---|---|---|---|
| Direct UDP + port forward (status quo) | None (host opens port) | None | None | Port-forward setup | None | N/A | Yes/Yes | Yes | https://wiki.factorio.com/Multiplayer |
| STUN hole punch + direct | UDP hole punch | None (then fail) | STUN host | None | None | RFC 8445 (std) | Yes/Yes | Yes | https://www.rfc-editor.org/rfc/rfc8445.html |
| Self-hosted TURN (coturn) | STUN + TURN | TURN relay | Server bandwidth | None | None | BSD-3-Clause | Yes/Yes | Yes (UDP relay) | https://github.com/coturn/coturn |
| Steam Sockets / SDR | ICE/WebRTC-style | Valve backbone | None (if on Steam) | Steam account | Steam (SDR needs Steam release) | BSD-3 (GNS) / proprietary SDR | Yes/Yes | Partial (UDP) | https://partner.steamgames.com/doc/features/multiplayer/steamdatagramrelay |
| Epic Online Services P2P | EOS relay/punch | EOS relay | Free tier | EOS account | Epic | Proprietary SDK | Yes/Yes | No (EOS transport) | https://dev.epicgames.com/docs | [check: unavailable to V11]
| PlayFab Party | Cloud relay + direct P2P | Azure relay | Metered Azure | None (Xbox subs on console) | Microsoft/Xbox | Proprietary SDK | Yes (Linux stated) | No (Party UDP) | https://learn.microsoft.com/en-us/xbox/playfab/multiplayer/networking/ |
| Overlay (Tailscale/Headscale) | STUN/ICE + DERP | DERP HTTPS relay | Infra + control server | Tailscale client install | Tailscale identity | BSD-3 (headscale) | Linux control server; clients all OS | No (WireGuard) | https://github.com/juanfont/headscale |
| Hosted realms (Minecraft-style) | Managed host | Managed host | Hosting cost | Subscription | Vendor host | Proprietary | Vendor-managed | No | https://minecraft.wiki/w/Realms |

## What the research literature says

- Ford, Srisuresh, Kegel, "Peer-to-Peer Communication Across Network Address Translators", USENIX '05 (arXiv cs/0603074): ~82% of NATs tested support UDP hole punching, ~64% TCP. https://arxiv.org/abs/cs/0603074v1
- Trautwein et al., "Challenging Tribal Knowledge..." (arXiv 2510.27500, 2025): 4.4M attempts across 85k+ networks; 70% ± 7.1% hole-punch success; TCP and QUIC success statistically indistinguishable (~70%). https://arxiv.org/abs/2510.27500v1
- Liang et al., "Implementing NAT Hole Punching with QUIC" (arXiv 2408.01791, VTC2024-Fall): QUIC-based hole punching reduces hole-punch time vs TCP; QUIC connection migration saves 2 RTTs vs QUIC re-punching and 3 RTTs vs TCP re-punching. https://arxiv.org/abs/2408.01791v1
- Kanaris, Pouwelse, "Mass Adoption of NATs..." (arXiv 2311.04658, 2023): CGNAT prevalence and experiments across 4 Dutch carriers; 3 of 4 allowed cross-connectivity. https://arxiv.org/abs/2311.04658v2
- Richter et al., "A Multi-perspective Analysis of Carrier-Grade NAT Deployment" (IMC 2016, arXiv 1605.05606): CGN deployment scope and behavior. https://arxiv.org/abs/1605.05606
- Edeline et al., "Using UDP for Internet Transport Evolution" (arXiv 1612.07816, 2016): UDP works on most networks; impairments concentrated in access networks. https://arxiv.org/abs/1612.07816v1

## Contradictions

- UDP superiority for NAT traversal: Ford 2005 reports UDP 82% vs TCP 64% (https://arxiv.org/abs/cs/0603074v1), while DCUtR 2025 finds TCP and QUIC success statistically indistinguishable at ~70% (https://arxiv.org/abs/2510.27500v1).
- UDP availability: Wikipedia/QUIC notes Google found only a small fraction of connections blocked (https://en.wikipedia.org/wiki/QUIC), while Tailscale documents whole networks blocking UDP (e.g., UC Berkeley guest Wi-Fi) (https://tailscale.com/blog/how-nat-traversal-works).
- IPv6 as NAT escape hatch: Tailscale argues IPv6 makes traversal much simpler (https://tailscale.com/blog/how-nat-traversal-works), but Valheim still reports client-side IPv6 connectivity issues (https://valheim.fandom.com/wiki/Dedicated_servers).

## Not applicable

- Tailscale/Headscale is WireGuard-based and would replace our QUIC transport rather than carry it, so it is not a drop-in relay for us.
- Epic Online Services P2P and PlayFab Party use their own transports/relays, so they do not preserve our pinned-trust QUIC semantics.
- IPv6-only operation is not viable today: CGNAT and many clients remain IPv4-only.
- Platform-locked relays (Steam SDR, EOS, PlayFab) conflict with our open-source, non-store-locked engine and Linux dedicated-server tooling.
- Minecraft Realms-style subscriptions are a player-cost model we are unlikely to adopt for a co-op CRPG.

## Agent log

- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + T106 fold-in · Keeps the R17 deep pass on co-op hosting friction as the T106 record; one marker flags the citation the V11 check could not fetch.

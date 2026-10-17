# R18 deep — Sharing and combining modules and mods

## Recommendations (ranked by expected impact)

1. Ship modules as zip packages with a manifest (name, version, description, dependencies, website) plus icon/README; use SemVer. Why: matches our data+Lua modules and deterministic CLI validation. Evidence: https://wiki.thunderstore.io/mods/creating-a-package — "A valid Thunderstore package is a zip file which **must** contain at least the following files at the root of the zip:" Strength: vendor doc. Effort: small.
2. Declare dependencies explicitly and install them recursively; declare incompatibilities. Why: prevents broken modules/saves from hidden deps. Evidence: https://wiki.thunderstore.io/mods/creating-a-package — "Dependencies declared in the manifest are automatically installed by the mod manager when a package is installed." and https://wiki.factorio.com/Modding — "The dependencies can also be used to declare a mod to be incompatible with other mods, which prevents them from being loaded together." Strength: vendor doc. Effort: small.
3. Gate publishing with automated scanning: MIME/zip validation, file-schema allow-lists, malware scan (ClamAV), rules engine, manual curation. Why: authors first but unsafe uploads must not reach players. Evidence: https://docs.mod.io/moderation/automated-scanning — "Virus and malware scanning" and "Automatically scan UGC contents, to ensure the files contained are ones your game supports". Strength: vendor doc. Effort: medium.
4. Provide an in-game/API browser plus collections/playlists and compatibility voting. Why: discovery and sharing combinations without manual installs. Evidence: https://docs.mod.io/collections — "we also added some other practical features, such as a compatibility voting system, allowing users to confirm the compatibility between the mods that are part of a collection." Strength: vendor doc. Effort: large.
5. Define combination rules as explicit load order with precedence and "rule of one" overrides; prefer patch operations over full overwrites. Why: deterministic replay and avoids silent overrides. Evidence: https://loot.github.io/docs/help/introduction-to-load-orders/ — "The game can only apply one version of a record, so if multiple plugins contain the same record, the last-loaded plugin's version overrides all others. This is known as the *rule of one*" and https://rimworldwiki.com/wiki/Modding_Tutorials/PatchOperations — "Prior to RimWorld alpha 17, modders could only modify Defs by creating new ones that overwrote the original; this often resulted in compatibility issues". Strength: vendor doc/practitioner report. Effort: large.
6. Support a bottom-of-stack compatibility framework and patch/compat mods. Why: resolves same-content conflicts without editing originals. Evidence: https://bg3.wiki/wiki/Modding:Compatibility_Framework — "CF however needs to be at the bottom of the load order." Strength: practitioner report. Effort: medium.
7. Version modules against engine/schema versions and support branch pinning. Why: migrations and avoids broken saves after updates. Evidence: https://partner.steamgames.com/doc/features/workshop/itemversioning — "By default, users who subscribe to a Steam Workshop item will download the latest version of that item." Strength: vendor doc. Effort: medium.
8. Keep author rights and avoid monetization lock-in: do not make monetization disable dependencies. Why: authors first; mod.io shows this trade-off. Evidence: https://docs.mod.io/dependency-management — "UGC that either allows dependencies, or depends on other content, cannot be monetized". Strength: vendor doc. Effort: small.

## Comparison table

### Channels (half 1)

| Channel | Observed traits | URL |
|---|---|---|
| Nexus Mods (Vortex) | zip mods, collections, mod-manager load order | https://www.nexusmods.com/site/mods/1?tab=files |
| mod.io | REST API/SDKs, deps, collections, moderation, in-game UI, 20GB uploads | https://docs.mod.io/ |
| Steam Workshop | ready-to-use or curated; ISteamUGC; item versioning by game branch | https://partner.steamgames.com/doc/features/workshop |
| Thunderstore | zip with icon.png/README.md/manifest.json; SemVer; deps; 5GB limit | https://wiki.thunderstore.io/mods/creating-a-package |
| CurseForge | REST API (x-api-key), fingerprints, file deps, categories | https://docs.curseforge.com/rest-api/ |
| Neverwinter Vault | community hub for NWN modules/haks | https://neverwintervault.org/ |
| GOG | store; "GOG One-click Mods" link; no dedicated mod API seen | https://www.gog.com/ |
| Epic Games | not covered in this run (no page loaded) | n/a |
| Bethesda.net Creations | not covered in this run (page not loaded) | n/a |

### Combination systems (half 2)

| System | Observed traits | URL |
|---|---|---|
| NWN haks/override | .hak ERF files; override precedence; ordered haks | https://nwn.fandom.com/wiki/Hak_pak |
| Bethesda load order/LOOT | 255 plugin limit; rule of one; hex positions; .esl light masters | https://loot.github.io/docs/help/introduction-to-load-orders/ |
| Paradox descriptors | not covered in this run (wiki blocked) | n/a |
| Minecraft Forge/Fabric | mixins (SpongePowered) for bytecode injection; Forge mods as jars | https://docs.minecraftforge.net/en/latest/gettingstarted/ |
| Factorio | info.json deps (required/optional/incompatible); data stages; migrations | https://lua-api.factorio.com/latest/auxiliary/mod-structure.html |
| RimWorld XML patch | PatchOperations (Add/Insert/Remove/Replace) instead of overwrites; LoadFolders.xml | https://rimworldwiki.com/wiki/Modding_Tutorials/PatchOperations |
| BG3 mod managers | .pak mods; modsettings.lsx load order; Script Extender; CF at bottom | https://bg3.wiki/wiki/Modding:Installing_mods |
| Mod ecosystems research | Modulith engine for modding; modding scenes; modding definition | https://api.openalex.org/works?search=modding+video+games&per-page=3 |

## What the research literature says

- Laukkanen (2005), "Modding Scenes — Introduction to user-created content in computer gaming" (book): "Modding is an increasingly popular participatory practice where game fans modify and extend officially released game titles with their own creations." (https://openalex.org/W18444137)
- Götz & von Mammen (2023), "Modulith: A Game Engine Made for Modding" (FDG 2023): "A modular game engine facilitates development of game technologies and contents. The latter includes content creation by the player in the form of modding." (https://openalex.org/W4363651367)
- Gaudelli & Taylor (2011), "Modding the global classroom? Serious video games and teacher reflection": "Modding, in title, refers to mass-produced software, either built-in alteration settings can be changed through external, add-on program." (OpenAlex record) [check: not found by V11]
- Kynigos & Yiannoutsou (2018), "Children challenging the design of half-baked games: Expressing values through the process of game modding" (Int. J. Child-Computer Interaction).
- Sotamaa (2026), "Modding Video Games" (Palgrave Encyclopedia of Cyberpsychology) (OpenAlex record).
- Petit (2026), "Pornoscopy and Video Games: From Modding Practices to Pornoscopic Experiences" (AN-ICON).

## Contradictions

- Curated vs open upload: Steam Workshop offers both "Ready-to-use" (no curation) and "Curated" (developer approval) models — https://partner.steamgames.com/doc/features/workshop.
- Monetization vs dependencies: mod.io allows dependencies only when content is not monetized — https://docs.mod.io/dependency-management.
- PatchOperations vs overwrites: RimWorld moved from overwriting Defs to PatchOperations to avoid compatibility issues — https://rimworldwiki.com/wiki/Modding_Tutorials/PatchOperations.

## Not applicable

- Epic Games and Bethesda.net mod platforms: no pages loaded in this run; excluded from evidence.
- Paradox mod descriptors: wiki blocked in this run; no descriptor details captured.
- Anna's Archive/LibGen/Sci-Hub/Z-Library: not used (shadow libraries prohibited).
- Numeric scores/benchmarks: this brief is qualitative; no benchmark numbers cited.

## Agent log

- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + T107 fold-in · Keeps the R18 deep pass on module and mod distribution as the T107 record; one marker flags the citation the V11 check could not find at its source.

# Legal and policy constraints: a standalone Halo 2 built on MCC files

Research notes, 2026-10-09. Not legal advice. These notes summarize public
documents and news reports. Each claim gives its source. Where something is
unknown or unconfirmed, the notes say so.

## 1. The MCC EULA names matchmaking directly

The Halo: The Master Chief Collection EULA on Steam
(https://store.steampowered.com/eula/976730_eula_0) has a section called
"COMPETING SERVICE". It opens: "you are not permitted to use our Game to stand
up a competing service, for instance you should not:" The list includes:

- Code Use: "use the code contained within the Game(s) or any part of it in
  another program or application outside of the MCC environment;"
- Matchmaking: "Host, provide or develop matchmaking services for the Game(s),
  or intercept, emulate or redirect the communication protocols used by our
  Game in any way, for any purpose, including without limitation unauthorized
  play over the internet, network play, ..."
- Unauthorized Connections: "any connection to any unauthorized server that
  emulates, or attempts to emulate, the Platform; and (ii) any connection using
  third-party programs or tools not expressly authorized by 343 Industries."

Other parts of the same EULA:

- "ONE MAJOR RULE": "you must not distribute the Game or anything we've made
  unless we specifically agree to it." The EULA counts as distribution:
  "Make commercial use of anything we've made", "Try to make money from
  anything we've made", and "Let other people get access to anything we've
  made in a way that is unfair or unreasonable."
- Mods are allowed: "If you bought the Game, you may play around with and
  modify it by adding modifications, tools, or plugins" ("Mods"). "Basically,
  Mods are okay to distribute; hacked versions or Modded Versions of the Game
  client or server software are not okay to distribute."
- A Mod is defined as "something original that you or someone else created
  that doesn't contain a substantial part of our copyrightable code or content."
- "You may only use or play our Game with your valid Xbox Live account."
- Microsoft may terminate the EULA if you breach it. You can end it by
  uninstalling the game.
- The main EULA does not mention EasyAntiCheat. Reverse engineering is
  forbidden in the bundled Mod Tools license, section 3(b).
- The fetched text carries no revision date.

The bundled "Microsoft Software License Terms: Mod Tools for Halo: The Master
Chief Collection Games" (the same text appears at
https://store.steampowered.com//eula/1695794_eula_0) say the tools may be used
"solely with" the listed MCC titles. They forbid: "reverse engineer, decompile
or disassemble the software", "work around any technical limitations", and
"use the software for commercial, non-profit, or revenue-generating
activities". They also forbid providing "the software as a stand-alone offering
for others to use".

Halo Waypoint's "MCC's EULA: The FAQ"
(https://www.halowaypoint.com/news/mccs-eula-the-faq) says:

- "Anything contained within MCC is allowed to be used for modding efforts."
- "Only the assets contained within the retail version of MCC are allowed for
  use in modding efforts."
- Selling mods or being paid for them is not allowed.
- "If you think it breaks the EULA, chances are it does."
- Creators who break the terms without knowing it are usually asked to take
  the content down rather than being banned.

EAC-off mode (https://support.halowaypoint.com/hc/en-us/articles/360037475251):
it is the official way to play with mods. With EAC off, "the public
matchmaking features will be unavailable", and online play is limited to
Custom Games and campaign. Microsoft's own mod path therefore keeps mods out
of matchmaking.

Official mod releases go through the Steam Workshop. Examples are the Excession
uploader (https://learn.microsoft.com/en-us/halo-master-chief-collection/excession/excessionoverview)
and the Halo 2 E3 2003 demo, published as a free Workshop mod on 2024-11-09
(https://www.gematsu.com/2024/11/halo-2-e3-2003-demo-coming-to-halo-the-master-chief-collection-for-pc-as-steam-workshop-mod-on-november-9).
That demo had no announced release for the Microsoft Store PC version. The
Halo 2 and Halo 3 mod tools need a Steam license for the base game
(https://www.techradar.com/news/halo-2-and-halo-3-finally-get-official-mod-tools-on-pc).

## 2. Microsoft's Game Content Usage Rules (all Microsoft games)

Source: https://www.xbox.com/en-US/developers/rules (last updated January 2015).

- The license is "personal, non-exclusive, non-sublicenseable,
  non-transferable, revocable, limited" and covers personal, non-commercial use.
- "You can't reverse engineer our games to access the assets".
- You can't sell your Item or earn money from it. Ads in the Item are not
  allowed, and you "can't use Game Content in an app that you sell in an app
  store". A free app must stay free and must not earn ad revenue.
- You must include Microsoft's notice and a link to the Rules.
- You may not use Microsoft's trademarks or logos except as Microsoft's
  trademark pages allow, or suggest that Microsoft made the Item.
- Microsoft "can revoke this limited-use license at any time and for any
  reason". It may tell you to stop distributing your Item right away.
- The Rules do not mention mods or standalone games by name.

## 3. Steam and trademark terms

- Steam Subscriber Agreement section 2.G
  (https://store.steampowered.com/subscriber_agreement/): users may not
  "reverse engineer, derive source code from, modify, disassemble, decompile,
  create derivative works based on" Content and Services. Content and Services
  includes third-party games. Two exceptions apply: anything Subscription Terms
  allow, and anything "under applicable law notwithstanding these
  restrictions". Valve may close accounts (sections 4.D and 9.C).
- Microsoft Trademark and Brand Guidelines
  (https://www.microsoft.com/en-us/legal/intellectualproperty/trademarks):
  "Don't use Microsoft's Brand Assets in the name of your business, product,
  service, app, domain name". The same applies to a "fan group". A true
  "compatible with" statement is allowed, and so is a sentence like "The
  Contoso Game can be played on Xbox gaming consoles". Do not imply
  endorsement.
  - This bears on the repository name "halo2-rs". Project Reclaimer and
    Project Cartographer keep "Halo" out of their names.

## 4. US and EU law (background)

- Sega v. Accolade (9th Cir. 1992): taking apart a program's code to reach its
  functional elements for compatibility was fair use when there was no other
  way to get them
  (https://www.copyright.gov/fair-use/summaries/segaenters-accolade-9thcir1992.pdf).
- 17 U.S.C. 1201(f) allows getting around a technical protection only to make
  an "independently created computer program" work with other programs, and
  only by someone who "has lawfully obtained the right to use a copy"
  (https://www.govinfo.gov/content/pkg/USCODE-2024-title17/html/USCODE-2024-title17-chap12-sec1201.htm).
- Davidson & Associates v. Jung (bnetd), 422 F.3d 630 (8th Cir. 2005)
  (https://en.wikipedia.org/wiki/Bnetd):
  - bnetd was an open-source copy of Blizzard's Battle.net matchmaking
    service, built by reverse engineering, that skipped CD-key checks.
  - The courts held that the EULA's ban on reverse engineering was enforceable
    and that bnetd broke the DMCA. The interoperability exception did not
    save it.
  - This is the closest US precedent to a home-made matchmaking server for
    someone else's game client.
- MDY v. Blizzard, 629 F.3d 928 (9th Cir. 2010)
  (https://en.wikipedia.org/wiki/MDY_Industries,_LLC_v._Blizzard_Entertainment,_Inc.):
  - Breaking terms of use is copyright infringement only when there is "a
    nexus between the condition and the licensor's exclusive rights of
    copyright".
  - Trafficking in a tool that gets around Warden still broke the DMCA's
    anti-circumvention rule.
  - So breaking a EULA is mainly a contract and account risk. Copying code or
    getting around a protection is a copyright or DMCA risk.
- 37 CFR 201.40(b)(19), last amended 2024-10-28
  (https://www.ecfr.gov/current/title-37/chapter-II/subchapter-A/part-201/section-201.40):
  - The DMCA exemption for games whose servers have shut down covers only
    "personal, local gameplay", defined as "not through an online service or
    facility".
  - It also needs the owner to have stopped server support.
  - It does not cover MCC, which is still live, or an online matchmaking
    service.
- EU Directive 2009/24/EC
  (https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32009L0024):
  - Article 5(3) lets a lawful user observe, study and test a program.
  - Article 6 allows decompiling for interoperability, within limits.
  - Under Article 8, contract terms that conflict with Article 6 are "null and
    void". EULA bans on reverse engineering are weaker in the EU than in the
    US after bnetd. John's country is not stated; he is assumed to be in the US.

## 5. Enforcement history and what has been tolerated

| Project | What it did | Outcome | Source |
|---|---|---|---|
| Halo Online / ElDewrito | PC mod built on assets from a leaked, cancelled game | 2015: DMCA notices against the Halo Online leak. 2018-04: Microsoft made the team stop. Its statement said the project was "built upon Microsoft-owned assets that were never lawfully released or authorized for this purpose" and that protecting its IP "isn't optional" | https://wccftech.com/microsoft-halt-eldewrito-halo-online-mod/amp/ ; https://www.theregister.com/2018/04/25/halo_online_dmca_notice/ ; https://siliconangle.com/2015/04/06/halo-online-modders-moving-forward-despite-microsofts-dmca-notices/ |
| Installation 01 | Fan game; programming written from scratch, art based on Halo designs | 2017-06: 343 said the team was "not under imminent legal threat". Conditions: stay non-commercial, take no donations, sell nothing | https://www.gamespot.com/articles/fan-made-halo-game-is-legal-and-development-can-co/1100-6451273 ; https://en.wikipedia.org/wiki/Installation_01 |
| Spartan Survivors | Free fan game using Halo characters | 2025-06: Halo Studios approached the team and gave permission. Only condition: a legal disclaimer. It is free on itch.io and planned for free release on Steam and Xbox | https://www.gamedeveloper.com/business/halo-studios-grants-blessing-for-fan-halo-survivors-game |
| Project Misriah | Halo 3 assets ported into a Counter-Strike 2 Workshop mod | 2025-12: Microsoft DMCA on the Steam Workshop listing | https://www.windowscentral.com/gaming/halo/microsoft-dmca-takedown-project-misriah-halo-cs2-mod |
| SPV3 | Halo CE mod | Requires a legal copy of Halo PC. No Microsoft stance found | https://pcgamesn.com/halo-combat-evolved/halo-combat-evolved-spv3-mod |
| Project Cartographer | Halo 2 Vista online after GFWL ended | No public action found. Forum users say the installer includes the full Halo 2 Vista game and that the project cites the 2015 "Class 23" exemption. Both claims are unverified | https://steamcommunity.com/app/976730/discussions/0/1752394498611439100 |
| Insignia | Fan replacement for original Xbox Live servers; Halo 2 matchmaking public beta from 2024-03-15 on original Xbox | Says it is "neither endorsed by nor affiliated with Microsoft". No public action found | https://insignia.live/ ; https://shacknews.com/article/139112/halo-2-online-matchmaking-public-beta |
| Halo CE browser port | Built from a decompilation; the user supplies their own disc image; ships no game data | 2026-10: no Microsoft statement, and no DMCA in GitHub's record as of the article | https://www.digitalcitizen.life/halo-combat-evolved-browser-port-no-game-data/ |
| halo2-decompiled | Matching decompilation of Xbox Halo 2; contributors' work under CC0; no game files, leaked code or symbols | Status with Microsoft unknown | https://github.com/kirklandsig/halo2-decompiled |
| **Project Reclaimer** (Halo 3) | Standalone launcher that loads Halo 3 from the player's own Steam MCC install, using the engine file halo3/halo3.dll. Ships "no game files". Each release is tied to one game build. Free and non-commercial; not open source yet. **Matchmaking is "not part of the project"**: custom games on community servers only | Covered in the press 2026-10-03 to 05. No statement from Microsoft, Halo Studios or Activision found. It has been public for only weeks, so "tolerated" is not yet established | https://projectreclaimer.dev/ ; https://projectreclaimer.dev/legal.html ; https://projectreclaimer.dev/host.html ; https://windowsforum.com/news/project-reclaimer-adds-64-player-halo-3-custom-games-on-windows.447125/ ; https://www.dexerto.com/halo/halo-3-mod-quadruples-multiplayer-matches-with-64-player-battles-3415552/ |

Activision is now in charge of Halo. On 2026-09-22 Matt Booty wrote that
"Activision will also be developing the next Halo title". "A small team at
Halo Studios" will keep supporting games already on sale
(https://godisageek.com/2026/09/xbox-halo-activision-268-layoffs/ ;
https://www.pcgamer.com/games/halo/halo-needed-to-change-but-activision-faces-a-huge-challenge-to-fix-it/).
A leak, not confirmed, says Halo Studios' Halo 2 and Halo 3 remakes were
shelved (https://www.destructoid.com/halo-projects-canceled/).

Activision's own record with fan multiplayer clients for its games:

- 2023-05: cease-and-desist letters to SM2 and X Labs; BOIII shut down
  (https://gameranx.com/updates/id/466701/article/activision-takes-down-call-of-duty-fan-clients-x-labs-boiii-and-more/).
- Plutonium survived after adding ownership checks
  (https://www.ggrecon.com/articles/call-of-duty-modded-clients-are-blocking-pirates-to-avoid-activisions-dmca-hammer).
- 2024-08-15: cease-and-desist to H2M-Mod, a fan Modern Warfare 2
  multiplayer mod running on owned Modern Warfare Remastered, the day before
  its release
  (https://www.videogameschronicle.com/news/activision-issues-cease-and-desist-to-modern-warfare-remastered-mod-that-shot-it-back-up-the-steam-charts/).

Whether Activision will police Halo fan projects the same way is unknown.

Take-Two v. re3/reVC: in 2021 Take-Two sued the makers of reverse-engineered
source for GTA III and Vice City, which needed the player's own game data. It
claimed $150,000 per work. The case settled in 2023 on undisclosed terms
(https://www.gamingonlinux.com/2021/09/take-two-filed-a-lawsuit-against-the-reverse-engineered-gta-iii-and-vice-city-developers/ ;
https://torrentfreak.com/take-two-dismisses-claims-against-lead-defendants-in-gta-mods-lawsuit-230405/).
This shows that "you must own the game" alone has not stopped a publisher
from suing.

## 6. Ranking system: the patent question is minor

- Halo 2's 1-50 ranks were not TrueSkill. They were an Elo-style hidden XP
  table designed by Max Hoberman (https://halopedia.org/Rank_(Halo_2)).
  h2live's `levels.rs` implements that same XP-table idea.
- Microsoft's TrueSkill patents are listed as expired by Google Patents, an
  estimate rather than a legal finding:
  - US7050868: anticipated expiry 2025-01-24 (https://patents.google.com/patent/US7050868B1/en)
  - US7376474: "Expired - Fee Related", adjusted 2025-05-06 (https://patents.google.com/patent/US7376474B2/en)
  - US7840288: "Expired - Fee Related" (https://patents.google.com/patent/US7840288B2/en)
- This only matters if TrueSkill were used.

## 7. Risk by approach (a reading of the sources above, not a legal ruling)

### A. A launcher that loads MCC's DLLs (the Project Reclaimer model)

- It runs Microsoft's own engine code "outside of the MCC environment". That
  is exactly what the EULA's Code Use clause names.
- Adding matchmaking would hit the Matchmaking clause head on: "Host, provide
  or develop matchmaking services", and "intercept, emulate or redirect the
  communication protocols". That holds in particular if the original
  online/session code is reused or redirected.
- If the launcher skips Steam or Xbox Live ownership or sign-in checks inside
  the DLL, a DMCA §1201 question arises (bnetd). Whether such checks exist in
  halo2.dll is unknown.
- The players' own EULA licenses are at risk, along with their accounts in
  theory. No reports of bans for Reclaimer were found.
- Reclaimer itself avoids matchmaking. Its documentation does not say why.
- Practical risk: each MCC update can break the launcher, because releases
  are tied to one game build.
- Legal risk: medium for custom games, by analogy to Reclaimer, which has no
  enforcement record yet. Higher for a launcher that adds matchmaking.

### B. A from-scratch engine that reads MCC map files at runtime (halo2-rs today, with a different file source)

- No Microsoft code is shipped or run. This is closest to independent
  creation (Sega v. Accolade, §1201(f)) and to Installation 01.
- It does not "use the code contained within the Game(s)". The matchmaking
  clause says "for the Game(s)". Whether a matchmaking service for an
  independent engine that loads MCC content counts as "for the Game(s)" is
  untested. Microsoft could read it broadly.
- The Game Content Usage Rules' line "You can't reverse engineer our games to
  access the assets" is in tension with a reader for the map format. The MCC
  FAQ allows "legally obtained tools" for modding MCC, but that is about mods
  inside MCC.
- The decompilation used as a reference should stay behaviour-only, with no
  code copied (re3 shows that reverse-engineered code is a lawsuit target).
- Legal risk: lowest of the three. A cease-and-desist is still possible at
  Microsoft's or Activision's discretion.

### C. Redistributing modified MCC DLLs

- This is barred by "ONE MAJOR RULE" and by "Modded Versions ... are not okay
  to distribute". It is plain copying of copyrighted code, so a DMCA takedown
  would be easy.
- ElDewrito was shut down for distributing Microsoft material that was never
  lawfully released.
- Distributing only a binary patch that the user applies to their own DLL is
  a grey area not addressed in the sources found.
- Legal risk: highest.

## 8. Precautions common to the projects that were tolerated

- Ship no game files: the repository's AGENTS.md already requires this.
  Require each player's own legally owned Steam MCC install, as Reclaimer and
  SPV3 do.
- Stay free and non-commercial: no donations, Patreon, ads or paid tiers.
  This was Installation 01's condition and is in the Game Content Usage Rules.
- Keep "Halo" out of the project's name and logo. Use only a plain-text
  "works with" statement, add a disclaimer of non-affiliation and a trademark
  notice, and give a contact address for takedown requests (Reclaimer's
  legal page is a template).
- Never connect to Microsoft, Xbox Live, Steam or MCC servers. Never redirect
  or emulate MCC's network protocol. Use the project's own protocol (h2net)
  and server (h2live).
- Moving from Halo 2 Vista files obtained through the Cartographer installer
  to a purchased Steam MCC license makes the source of the game files clear.
  It is unverified whether the Cartographer installer bundles the game.
- Optional: ask Microsoft or Activision for permission, as Installation 01
  and Spartan Survivors did. The chance of success under Activision is
  unknown.

## 9. Unknowns

- No public statement from Microsoft, Halo Studios or Activision on Project
  Reclaimer, or on any MCC-based standalone launcher.
- How Activision will enforce Halo IP after 2026-09-22.
- Whether halo2.dll in MCC contains Steam or Xbox Live ownership checks that a
  launcher would have to get around.
- Whether the Microsoft Store/Xbox app copy of MCC stores files in a form a
  third-party program can read without getting around a protection.
- Exactly what the Project Cartographer installer distributes.
- Insignia's current Halo 2 ranked status and its legal footing.
- John's country, which decides whether US or EU interoperability law applies.

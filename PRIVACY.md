# Privacy

Night Client has **no analytics, telemetry or advertising**, and no Night Client server receives your
data. This page lists every connection it makes and everything it stores.

## Connections

| Service | When | What is sent |
|---|---|---|
| Microsoft (`login.microsoftonline.com`) | Signing in, renewing a session | OAuth requests from your browser and from Night Client. Your password is typed only on Microsoft's page. |
| Xbox Live (`user.auth.xboxlive.com`, `xsts.auth.xboxlive.com`) | Signing in, renewing a session | Microsoft tokens, to obtain Xbox tokens |
| Minecraft services (`api.minecraftservices.com`) | Signing in, checking game ownership | Xbox tokens; returns your Minecraft profile |
| Mojang (`piston-meta.mojang.com`, `launchermeta.mojang.com`, `libraries.minecraft.net`, `resources.download.minecraft.net` and the download hosts their manifests name) | Installing or repairing versions and Java | File requests only |
| Fabric (`meta.fabricmc.net`, `maven.fabricmc.net`), Quilt (`meta.quiltmc.org`, `maven.quiltmc.org`), NeoForge (`maven.neoforged.net`) and Forge (`maven.minecraftforge.net`) | Creating or installing a modded instance | File requests only |
| Modrinth (`api.modrinth.com`, `cdn.modrinth.com`) | When you browse, install or update mods, and when an instance's Mods tab finds jar files it does not know yet | Searches, project and version ids, and SHA-512 hashes of mod files (to identify them and find updates) |
| Night Client's release feed (GitHub) | Shortly after start (can be turned off), or when you click Check now | Your Night Client version and Windows architecture |

Every request also reveals your IP address to that service. Night Client's requests carry a
`User-Agent` naming Night Client and its version; update checks carry the updater library's own
(`tauri-plugin-updater/<version>`). Game servers you join are contacted by Minecraft itself, not by Night Client.

## What is stored on your PC

- **`%APPDATA%\NightClient`:** settings, instances, download history and account names/UUIDs (in
  `night-client.db`), and game data, unless you moved it.
- **`%LOCALAPPDATA%\NightClient\vault`:** Microsoft refresh tokens, encrypted with Windows DPAPI for
  your Windows account. They are never written to logs or the database.
- **`%LOCALAPPDATA%\NightClient\logs`:** launcher logs and crash reports, with tokens removed. They stay
  on your PC unless you export them (Diagnostics → Export…) and share the file yourself.

Uninstalling removes the program but keeps your game data and accounts. To remove everything, delete
the two `NightClient` folders above after uninstalling.

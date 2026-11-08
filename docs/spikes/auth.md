# Spike: Microsoft authentication (Stage 0)

## Status

The chain is implemented in `crates/nc-launcher/src/auth.rs`. Every step and error mapping is
tested against a mock server (`tests/auth.rs`). **It has not been run end to end against Microsoft and
Mojang**, because the project has no approved Azure application ID yet.

## Project owner actions (blocking for end-to-end sign-in)

1. **Register an app** in Microsoft Entra ID (Azure portal → App registrations → New registration):
   - Supported account types: **personal Microsoft accounts only** (the launcher uses the `consumers` tenant).
   - Platform: **Mobile and desktop applications**, redirect URI `http://localhost`.
   - Authentication → *Allow public client flows*: **Yes** (no client secret exists).
   - API permissions: none to add. The launcher requests `XboxLive.signin offline_access` at runtime.
2. **Request Minecraft API access** through the *Minecraft Java App ID review* form linked from
   `aka.ms/mce-reviewappid`. Until Mojang approves the ID, `login_with_xbox` returns **403**, which
   Night Client reports as `auth_app_not_approved`.
3. Give the client ID to builds through the `NIGHT_CLIENT_MSA_CLIENT_ID` build variable or env var.
   It is a public identifier, not a secret, but keeping it out of the source means forks register their own.

## Flow

1. `Loopback::bind()` listens on `127.0.0.1:<ephemeral>`. The redirect URI is `http://localhost:<port>`.
2. PKCE S256 (32 random bytes from the OS CSPRNG) plus a random `state`. The system browser opens the
   authorize URL (`prompt=select_account`, which makes account switching explicit).
3. The callback is validated: `state` must match (`auth_state_mismatch` otherwise), `error=` maps to
   `auth_denied`, and favicon or other paths are ignored. The listener times out after 5 minutes and
   can be cancelled.
4. Code → token exchange (`/consumers/oauth2/v2.0/token`, with `code_verifier`).
5. Xbox Live `user/authenticate` (RPS) → XSTS (`rp://api.minecraftservices.com/`) → `login_with_xbox`.
6. `entitlements/mcstore` must contain `game_minecraft` or `product_minecraft`. Then `minecraft/profile`.

### Mapped errors

| Condition | Code |
|---|---|
| XSTS `XErr 2148916233`: no Xbox profile | `auth_no_xbox_account` |
| `2148916235`: region | `auth_region` |
| `2148916236/7`: age verification | `auth_age_verification` |
| `2148916238`: child account | `auth_child_account` |
| `login_with_xbox` 403 | `auth_app_not_approved` |
| No entitlement | `auth_not_owned` |
| Profile 404 | `auth_no_profile` |
| Refresh `invalid_grant` | `auth_reauth_required` |
| 429 | `auth_rate_limited` |

## Open questions for the end-to-end run

- Entra accepts `http://localhost` with **any** port for public clients (RFC 8252). This is expected but
  not yet verified against a real registration.
- Browsers that resolve `localhost` to `::1` first: we bind IPv4 only and rely on the browser falling back
  to IPv4. If that fails in practice, also bind `[::1]` on the same port.
- Token storage (Stage 3): the refresh token is encrypted with DPAPI and stored in `%LOCALAPPDATA%\NightClient\vault`; see `nc_launcher::vault` and `nc_launcher::accounts`. Windows Credential Manager was rejected because of its ~2.5 KB blob limit.
